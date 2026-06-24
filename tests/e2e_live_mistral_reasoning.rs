//! Live E2E smoke tests for Mistral reasoning through the full agent loop.
//!
//! These are the Rust, Live-tier replacement for the former
//! `scripts/test-mistral-reasoning-ironclaw.sh` bash harness. The
//! **primary** regression net for this feature is the offline, deterministic
//! matrix in `crates/ironclaw_llm/src/mistral_tests.rs` (C1–C12): it already
//! proves — with loopback mock servers, no API key — that `reasoning_effort`
//! is sent for small/medium and omitted for large/off, that the array
//! (`[{thinking},{text}]`) and string responses both parse, the error mapping,
//! and that a prior turn's thinking is replayed as a chunk. These live tests
//! are a thin smoke layer over the one thing the offline matrix cannot reach:
//! that a *real* Mistral reasoning response round-trips through the agent loop
//! without the original `JsonError: did not match any variant of untagged enum
//! ApiResponse` parse failure, and that a second turn does not 400 when the
//! parsed thinking chunk is replayed.
//!
//! ## What we assert (and what we deliberately don't)
//!
//! The verdict is **a non-empty reply with no failure signature** — exactly the
//! bash harness's evidence-based core check. We do NOT assert on
//! `StatusUpdate::Thinking`: on the v1 agent path those events are reused for
//! generic status ("Step complete — N in / M out tokens", message previews),
//! so their presence is not a reasoning signal. Reasoning *parsing* is proven
//! deterministically by offline C6; the multi-turn test exercises the replay
//! round-trip end-to-end.
//!
//! ## Running
//!
//! ```bash
//! # Live — real Mistral API call. Reasoning is on by default for medium/small.
//! IRONCLAW_LIVE_TEST=1 LLM_BACKEND=mistral MISTRAL_REASONING=high MISTRAL_API_KEY=... \
//!   cargo test --features libsql --test e2e_live_mistral_reasoning -- --ignored --nocapture
//!
//! # Point at the small model instead of the default mistral-medium-latest:
//! IRONCLAW_LIVE_TEST=1 LLM_BACKEND=mistral MISTRAL_MODEL=mistral-small-latest \
//!   MISTRAL_API_KEY=... cargo test --features libsql --test e2e_live_mistral_reasoning \
//!   -- --ignored --nocapture
//! ```
//!
//! Outside `IRONCLAW_LIVE_TEST=1` (i.e. the default `cargo test` matrix and CI)
//! the harness is built with `with_no_trace_recording()`, so it resolves to
//! `TestMode::Skipped` and the test does no network work and needs no key.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod live_tests {
    use std::time::Duration;

    use ironclaw::channels::OutgoingResponse;

    use crate::support::live_harness::{LiveTestHarnessBuilder, TestMode};

    /// The all-labels-wrong box puzzle: a genuine deduction task (not
    /// arithmetic/counting), reachable only by reasoning. Carried over from the
    /// bash harness so the live behaviour matches what was hand-verified.
    const REASONING_PROMPT: &str = "Three boxes are labeled APPLES, ORANGES, and MIXED. \
         You are told that every single label is wrong. You may take exactly one fruit out of \
         exactly one box and look at it. From which one box should you take a fruit so that you \
         can then correctly relabel all three boxes? Think before you answer, then end with \
         \"ANSWER: <box label>\".";

    /// Substrings that mean the Mistral round-trip broke. Mirrors the bash
    /// harness `FAIL_RE`, plus the sanitized channel-boundary phrasings the
    /// agent surfaces for a provider failure (per `.claude/rules/error-handling.md`).
    const FAILURE_MARKERS: &[&str] = &[
        "did not match any variant",
        "jsonerror",
        "invalid response from",
        "empty response from",
        "temporarily unavailable",
        "authentication failed",
        "request failed",
    ];

    const RUN_HINT: &str = "Run with: IRONCLAW_LIVE_TEST=1 LLM_BACKEND=mistral \
         MISTRAL_REASONING=high MISTRAL_API_KEY=... cargo test --features libsql \
         --test e2e_live_mistral_reasoning -- --ignored --nocapture";

    /// Live-tier gate. The harness has no backend override (it resolves
    /// `Config::from_env()`), so the runner must explicitly select Mistral —
    /// the same runner-sets-the-env pattern `e2e_live_reasoning.rs` uses for
    /// `LLM_MODEL`. This guarantees we never silently run against whatever
    /// backend the developer happens to have configured.
    fn mistral_backend_selected() -> bool {
        std::env::var("LLM_BACKEND")
            .unwrap_or_default()
            .eq_ignore_ascii_case("mistral")
    }

    /// Point `Config::from_env()` at libSQL so config resolution does not demand
    /// a Postgres `DATABASE_URL`. The rig always runs against its own fresh,
    /// empty temp libSQL DB (see `tests/support/LIVE_TESTING.md`), so this only
    /// satisfies config validation — it doesn't change where data lands. We
    /// write through the runtime overlay that `Config::from_env()` reads
    /// (`optional_env` → `env_or_override`), not unsafe `std::env::set_var`, and
    /// only when the runner hasn't already chosen a backend. Mirrors the
    /// `DATABASE_BACKEND=libsql` the former bash harness set internally.
    fn ensure_libsql_backend() {
        if std::env::var("DATABASE_BACKEND")
            .ok()
            .filter(|v| !v.is_empty())
            .is_none()
        {
            ironclaw::config::set_runtime_env("DATABASE_BACKEND", "libsql");
        }
    }

    /// Assert the reply is a real answer, not empty and not a failure/boundary
    /// error. This is the live-only fact: the array-shaped reasoning response
    /// deserialized and round-tripped through the agent loop.
    fn assert_clean_reply(resp: &OutgoingResponse, turn: &str) {
        let content = resp.content.trim();
        assert!(
            !content.is_empty(),
            "{turn}: expected a non-empty reply from Mistral, got an empty one"
        );
        let lower = content.to_ascii_lowercase();
        for marker in FAILURE_MARKERS {
            assert!(
                !lower.contains(marker),
                "{turn}: reply contains failure signature {marker:?} — the Mistral reasoning \
                 round-trip broke.\nReply: {content}"
            );
        }
    }

    /// Single reasoning turn: proves the real `[{thinking},{text}]` response
    /// deserializes and the agent produces a coherent answer (the exact path
    /// the original `ApiResponse` parse bug broke).
    #[tokio::test]
    #[ignore] // Live tier: requires IRONCLAW_LIVE_TEST=1 + LLM_BACKEND=mistral + MISTRAL_API_KEY
    async fn mistral_reasoning_round_trips() {
        if !mistral_backend_selected() {
            eprintln!("[MistralReasoningE2E] LLM_BACKEND != mistral — skipping. {RUN_HINT}");
            return;
        }
        ensure_libsql_backend();

        let harness = LiveTestHarnessBuilder::new("mistral_reasoning_round_trips")
            .with_no_trace_recording()
            .build()
            .await;

        if harness.mode() != TestMode::Live {
            eprintln!(
                "[MistralReasoningE2E] Live-only — skipping outside IRONCLAW_LIVE_TEST=1. {RUN_HINT}"
            );
            return;
        }

        let rig = harness.rig();
        rig.send_message(REASONING_PROMPT).await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(90)).await;

        assert!(
            !responses.is_empty(),
            "expected at least one response from Mistral within the timeout"
        );
        assert_clean_reply(&responses[0], "turn 1");

        eprintln!(
            "[MistralReasoningE2E] ✓ reasoning round-trip produced a clean reply ({} chars)",
            responses[0].content.len()
        );
    }

    /// Multi-turn: the prior assistant turn (including its parsed thinking) is
    /// replayed back to Mistral as a `[{thinking},{text}]` chunk on turn 2.
    /// Confirms the live ThinkChunk replay does not 400 — `WU7`'s open item.
    /// Offline C8 proves the builder replays the chunk; this proves the real
    /// API accepts it.
    #[tokio::test]
    #[ignore] // Live tier: requires IRONCLAW_LIVE_TEST=1 + LLM_BACKEND=mistral + MISTRAL_API_KEY
    async fn mistral_reasoning_multi_turn_replays() {
        if !mistral_backend_selected() {
            eprintln!("[MistralReasoningE2E] LLM_BACKEND != mistral — skipping. {RUN_HINT}");
            return;
        }
        ensure_libsql_backend();

        let harness = LiveTestHarnessBuilder::new("mistral_reasoning_multi_turn_replays")
            .with_no_trace_recording()
            .build()
            .await;

        if harness.mode() != TestMode::Live {
            eprintln!(
                "[MistralReasoningE2E] Live-only — skipping outside IRONCLAW_LIVE_TEST=1. {RUN_HINT}"
            );
            return;
        }

        let rig = harness.rig();

        // Turn 1.
        rig.send_message(REASONING_PROMPT).await;
        let first = rig.wait_for_responses(1, Duration::from_secs(90)).await;
        assert!(
            !first.is_empty(),
            "turn 1: expected a response before sending turn 2"
        );
        assert_clean_reply(&first[0], "turn 1");

        // Turn 2 in the same session: the engine replays turn 1's assistant
        // message (with its thinking chunk) into the next Mistral request.
        rig.send_message("Now briefly explain why that is the right box to open first.")
            .await;
        let all = rig.wait_for_responses(2, Duration::from_secs(90)).await;
        assert!(
            all.len() >= 2,
            "turn 2: expected a second response — the multi-turn thinking replay must not fail \
             (HTTP 400 on turn 2 was the failure mode WU7 guards against), got {} response(s)",
            all.len()
        );
        assert_clean_reply(&all[1], "turn 2");

        eprintln!("[MistralReasoningE2E] ✓ multi-turn reasoning replay succeeded across 2 turns");
    }
}
