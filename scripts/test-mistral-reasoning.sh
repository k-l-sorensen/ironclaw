#!/usr/bin/env bash
#
# Live smoke test for Mistral reasoning_effort — LOCAL FORK helper.
# (See CLAUDE-local.md → "Mistral reasoning fix".)
#
# Reads MISTRAL_API_KEY from the environment and sends the SAME reasoning prompt to
# the Mistral API twice — once WITH reasoning_effort (the field this fork
# injects) and once WITHOUT — then compares them. Reasoning shows up as a large
# jump in completion tokens and, usually, a <think> trace in the content.
#
# This validates the API contract our injection relies on. The IronClaw-side
# wiring (gating + flattening the field into the request) is covered by unit
# tests: `cargo test -p ironclaw_llm mistral`.
#
# Usage (MISTRAL_API_KEY must be set in the environment):
#   MISTRAL_API_KEY=... ./scripts/test-mistral-reasoning.sh
#   MISTRAL_API_KEY=... MISTRAL_MODEL=mistral-small-latest MISTRAL_REASONING_EFFORT=low ./scripts/test-mistral-reasoning.sh
#
set -euo pipefail

MODEL="${MISTRAL_MODEL:-mistral-medium-latest}"
EFFORT="${MISTRAL_REASONING_EFFORT:-high}"
ENDPOINT="https://api.mistral.ai/v1/chat/completions"
PROMPT='John is one of 4 children. The first sister is 4 years old. Next year, the second sister will be twice as old as the first sister. The third sister is two years older than the second sister. The third sister is half the age of her older brother. How old is John?'

for bin in jq curl; do
  command -v "$bin" >/dev/null || { echo "error: '$bin' not found in PATH" >&2; exit 1; }
done

API_KEY="${MISTRAL_API_KEY:-}"
[ -n "$API_KEY" ] || { echo "error: MISTRAL_API_KEY is not set in the environment" >&2; exit 1; }

# Send one chat-completion request. $1 = full JSON body. Echoes the raw response.
call() {
  curl -sS "$ENDPOINT" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $API_KEY" \
    -d "$1"
}

# Reliable signal: Mistral returns `message.content` as a plain STRING normally,
# but as a structured ARRAY with a "thinking" part when reasoning_effort engages.
# $1 = label, $2 = response json. Human-readable lines → stderr; the machine value
# (1 if a thinking part is present, else 0) → stdout for the caller to capture.
report() {
  local label="$1" resp="$2" ctype has_think text tokens
  if ! ctype="$(echo "$resp" | jq -e -r '.choices[0].message.content | type' 2>/dev/null)"; then
    echo "  [$label] unexpected response:" >&2
    echo "$resp" | jq . >&2 2>/dev/null || echo "$resp" >&2
    exit 1
  fi
  tokens="$(echo "$resp" | jq -r '.usage.completion_tokens // "?"')"
  if [ "$ctype" = "array" ]; then
    has_think="$(echo "$resp" | jq -r 'if (.choices[0].message.content | map(.type) | index("thinking")) != null then 1 else 0 end')"
    text="$(echo "$resp" | jq -r '[.choices[0].message.content[] | select(.type=="text") | .text] | join(" ")')"
  else
    has_think=0
    text="$(echo "$resp" | jq -r '.choices[0].message.content')"
  fi
  {
    echo "  content shape     : $ctype"
    echo "  thinking part     : $([ "$has_think" = 1 ] && echo yes || echo no)"
    echo "  completion_tokens : $tokens"
    echo "  answer (tail)     : $(echo "$text" | tail -c 160 | tr '\n' ' ')"
  } >&2
  echo "$has_think"   # stdout: 1 if a thinking part is present, else 0
}

base_body="$(jq -n --arg m "$MODEL" --arg p "$PROMPT" \
  '{model:$m, messages:[{role:"user", content:$p}]}')"
reasoning_body="$(echo "$base_body" | jq --arg e "$EFFORT" '. + {reasoning_effort:$e}')"

echo
echo "== WITHOUT reasoning_effort ($MODEL) =="
think_off="$(report off "$(call "$base_body")" | tail -n1)"

echo
echo "== WITH reasoning_effort=$EFFORT ($MODEL) =="
think_on="$(report on "$(call "$reasoning_body")" | tail -n1)"

echo
echo "== Verdict =="
if [ "$think_on" = 1 ] && [ "$think_off" = 0 ]; then
  echo "  PASS — reasoning_effort=$EFFORT engaged a 'thinking' content part that is"
  echo "         absent without the field. Mistral honors the field this fork injects."
elif [ "$think_on" = 1 ]; then
  echo "  PASS (weak) — reasoning request produced a 'thinking' part, but the baseline"
  echo "         also did (think_off=$think_off). Field is honored; contrast is muddy."
else
  echo "  FAIL — no 'thinking' part on the reasoning request (think_on=$think_on)."
  echo "  The model may not support reasoning_effort (use mistral-small*/mistral-medium*)."
  exit 1
fi
