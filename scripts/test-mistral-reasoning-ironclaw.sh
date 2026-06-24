#!/usr/bin/env bash
#
# Live end-to-end acceptance test for Mistral reasoning THROUGH IronClaw.
# (See CLAUDE-local.md → "Mistral reasoning — BUILT".)
#
# WHY THIS EXISTS
# ---------------
# With reasoning on, Mistral returns `message.content` as an ARRAY of typed
# chunks (`[{thinking},{text}]`), not a string. The original bug: rig-core could
# not deserialize that shape and every turn failed with
# `JsonError: did not match any variant of untagged enum ApiResponse`. The custom
# MistralProvider now owns that parsing. This script proves the *receive* path
# works against the real API — and, crucially, it WRITES A LOG OF THE ACTUAL
# INTERACTION so the result can be inspected, not just trusted.
#
# The verdict is evidence-based, never a bare grep for a "right answer"
# (arithmetic correctness tells you nothing about whether reasoning round-tripped,
# and LLMs are unreliable at arithmetic anyway). For each run it checks:
#   • reasoning_effort actually sent on the request (or correctly omitted)
#   • a thinking trace parsed back out of the response (or correctly absent)
#   • a non-empty reply with none of the known failure signatures
# The full reasoning trace and answer are logged for human/agent inspection.
#
# COVERAGE
# --------
#   • both reasoning models   — mistral-small-latest, mistral-medium-latest
#   • a non-reasoning model    — mistral-large-latest (param auto-omitted)
#   • the off toggle           — MISTRAL_REASONING=off on a reasoning model
#   • a genuine reasoning task  — a logic-deduction puzzle (not calc/count)
#   • the original arithmetic prompt, reworded
#
# Usage:
#   ./scripts/test-mistral-reasoning-ironclaw.sh
#   MISTRAL_API_KEY=... ./scripts/test-mistral-reasoning-ironclaw.sh   # skip 1Password
#
# Do NOT use `set -e` — we want every test to run and a summary at the end.
set -uo pipefail

OP_REF="***REDACTED***"

SMALL="mistral-small-latest"
MEDIUM="mistral-medium-latest"
LARGE="mistral-large-latest"

# Prompts. The arithmetic one is kept (reworded to "Think before you answer") for
# continuity; the logic puzzle is the genuine reasoning test — a unique answer
# reachable only by deduction (the all-labels-wrong box problem; answer: MIXED).
PROMPT_MULT='What is 17 multiplied by 23? Think before you answer, then end with "ANSWER: <number>".'
PROMPT_LOGIC='Three boxes are labeled APPLES, ORANGES, and MIXED. You are told that every single label is wrong. You may take exactly one fruit out of exactly one box and look at it. From which one box should you take a fruit so that you can then correctly relabel all three boxes? Think before you answer, then end with "ANSWER: <box label>".'

# (model | MISTRAL_REASONING | prompt-key | expectation)
#   expectation = "reasoning"  → reasoning_effort sent AND a thinking trace returns
#   expectation = "plain"      → reasoning_effort omitted AND no thinking trace
TESTS=(
  "$MEDIUM|high|LOGIC|reasoning"
  "$SMALL|high|LOGIC|reasoning"
  "$MEDIUM|high|MULT|reasoning"
  "$LARGE|high|LOGIC|plain"
  "$MEDIUM|off|MULT|plain"
)

# Failure signatures: any of these in stdout/stderr fails the run hard.
FAIL_RE='did not match any variant|JsonError|Invalid response from mistral|Empty response from mistral|temporarily unavailable \(HTTP|Authentication failed for provider .mistral.|panicked|Provider mistral request failed'

# ── prerequisites ───────────────────────────────────────────────────────────
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

if [ -z "${MISTRAL_API_KEY:-}" ]; then
  command -v op >/dev/null || { echo "error: 'op' not found and MISTRAL_API_KEY unset" >&2; exit 1; }
  echo "Reading MISTRAL_API_KEY from 1Password ($OP_REF)..."
  MISTRAL_API_KEY="$(op read "$OP_REF")"
fi
[ -n "${MISTRAL_API_KEY:-}" ] || { echo "error: empty API key" >&2; exit 1; }
export MISTRAL_API_KEY

STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_DIR="${IRONCLAW_MISTRAL_LOG_DIR:-/tmp/ironclaw-mistral-tests/$STAMP}"
mkdir -p "$LOG_DIR"
SUMMARY="$LOG_DIR/summary.log"
: > "$SUMMARY"

echo "Building ironclaw (once)..."
cargo build --quiet --bin ironclaw || { echo "error: build failed" >&2; exit 1; }
BIN="$REPO_ROOT/target/debug/ironclaw"
[ -x "$BIN" ] || { echo "error: built binary not found at $BIN" >&2; exit 1; }

echo "Logs: $LOG_DIR"
echo

PASS=0
FAIL=0

run_test() {
  local idx="$1" model="$2" reasoning="$3" prompt_key="$4" expect="$5"
  local prompt; eval "prompt=\"\${PROMPT_$prompt_key}\""
  local tag; tag="$(printf '%02d-%s-%s' "$idx" "$model" "$reasoning")"
  local base="$LOG_DIR/test-$tag"
  local out="$base.stdout.log" err="$base.stderr.log" log="$base.log"

  echo "── TEST $idx: $model | reasoning=$reasoning | $prompt_key | expect=$expect"

  # Throwaway libSQL backend so config resolution doesn't demand a Postgres URL.
  # mistral=debug → the request body (with reasoning_effort early). reasoning=trace
  # → the parsed thinking trace (the only TRACE-level source, so TRACE lines ARE
  # the reasoning). Note: the stderr writer caps each event at 500 bytes.
  env \
    LLM_BACKEND="mistral" \
    MISTRAL_MODEL="$model" \
    MISTRAL_REASONING="$reasoning" \
    DATABASE_BACKEND="libsql" \
    LIBSQL_PATH="$(mktemp -u /tmp/ironclaw-mistral-$tag.XXXXXX.db)" \
    RUST_LOG="warn,ironclaw_llm::mistral=debug,ironclaw_llm::reasoning=trace" \
    "$BIN" --cli-only --no-db --no-onboard --auto-approve -m "$prompt" \
    >"$out" 2>"$err"
  local code=$?

  # ── evidence extraction ──
  local req_bodies effort_sent thinking_parsed trace_lines trace_count replay answer fails
  req_bodies="$(grep -a "Mistral request body" "$err" || true)"
  if echo "$req_bodies" | grep -aq '"reasoning_effort":"high"'; then effort_sent=yes; else effort_sent=no; fi
  # Receive-side signal: the provider emits each parsed thinking trace on the
  # dedicated `ironclaw_llm::reasoning` target at TRACE level. That is the ONLY
  # TRACE-level source we enable via RUST_LOG, so any TRACE line is a parsed
  # reasoning trace — its presence proves the array-shaped response was
  # deserialized and the thinking chunk extracted (exactly what the original bug
  # could NOT do). Each event is capped at 500B by the stderr writer, which is
  # plenty to read. (IronClaw's formatter wraps the level token in ANSI color
  # even when redirected to a file, so strip ANSI before matching " TRACE ".)
  trace_lines="$(sed $'s/\x1b\\[[0-9;]*m//g' "$err" | grep -aE '[[:space:]]TRACE[[:space:]]' || true)"
  trace_count="$(printf '%s\n' "$trace_lines" | grep -ac . || true)"
  [ -n "$trace_lines" ] || trace_count=0
  if [ "$trace_count" -ge 1 ]; then thinking_parsed=yes; else thinking_parsed=no; fi
  # Bonus: ThinkChunk replayed on a later turn (proves the full round-trip).
  if echo "$req_bodies" | grep -aq '"type":"thinking"'; then replay=yes; else replay=no; fi
  answer="$(cat "$out")"
  fails="$(grep -aE "$FAIL_RE" "$out" "$err" || true)"

  # ── verdict ──
  local verdict="PASS" reason=""
  if [ -n "$fails" ]; then
    verdict="FAIL"; reason="failure signature present"
  elif [ -z "${answer//[[:space:]]/}" ]; then
    verdict="FAIL"; reason="empty reply"
  elif [ "$expect" = "reasoning" ]; then
    if [ "$effort_sent" != yes ]; then verdict="FAIL"; reason="reasoning_effort was NOT sent (gating/env bug)";
    elif [ "$thinking_parsed" != yes ]; then verdict="FAIL"; reason="no thinking chunk parsed from the array response (receive path failed — the original bug)";
    else reason="reasoning_effort sent + thinking chunk parsed back + coherent reply"; fi
  else # expect plain
    if [ "$effort_sent" = yes ]; then verdict="FAIL"; reason="reasoning_effort WAS sent but should have been omitted (model-gate/toggle bug)";
    elif [ "$thinking_parsed" = yes ]; then verdict="FAIL"; reason="unexpected thinking chunk for a non-reasoning run";
    else reason="reasoning_effort correctly omitted + no thinking chunk + coherent reply"; fi
  fi

  # ── structured per-test log ──
  {
    echo "============================================================"
    echo "TEST $idx — $model | reasoning=$reasoning | expect=$expect"
    echo "Prompt: $prompt"
    echo "============================================================"
    echo
    echo "--- request reasoning_effort (from debug body log; capped 500B/event) ---"
    echo "${req_bodies:-(no request-body log lines captured)}"
    echo
    echo "--- reasoning trace parsed from the response (TRACE events; 500B/event cap) ---"
    echo "${trace_lines:-(no reasoning trace — none parsed)}"
    echo
    echo "--- final answer (full stdout) ---"
    echo "${answer:-(empty)}"
    echo
    echo "--- failure scan ---"
    echo "${fails:-none}"
    echo
    echo "--- diagnostics ---"
    echo "exit code                         : $code"
    echo "reasoning_effort=high requested   : $effort_sent"
    echo "thinking chunk parsed (receive)   : $thinking_parsed"
    echo "reasoning trace events logged     : $trace_count"
    echo "multi-turn ThinkChunk replay seen : $replay"
    echo "raw stdout / stderr               : $out / $err"
    echo
    echo "VERDICT: $verdict — $reason"
  } > "$log"

  echo "    $verdict — $reason"
  echo "    log: $log"
  {
    echo "TEST $idx | $model | reasoning=$reasoning | $prompt_key | expect=$expect"
    echo "  -> $verdict — $reason"
    echo "     effort_sent=$effort_sent thinking_parsed=$thinking_parsed trace_events=$trace_count replay=$replay exit=$code"
    echo "     $log"
  } >> "$SUMMARY"

  if [ "$verdict" = PASS ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); fi
}

i=0
for row in "${TESTS[@]}"; do
  i=$((i+1))
  IFS='|' read -r m r pk ex <<<"$row"
  run_test "$i" "$m" "$r" "$pk" "$ex"
  echo
done

echo "============================================================"
echo "SUMMARY — $PASS passed, $FAIL failed (of ${#TESTS[@]})"
echo "Per-test + summary logs under: $LOG_DIR"
echo "Read them with:  for f in $LOG_DIR/test-*.log; do echo \"== \$f ==\"; cat \"\$f\"; done"
echo "============================================================"
cat "$SUMMARY"

[ "$FAIL" -eq 0 ]
