//! Live smoke test for the Mistral `reasoning_effort=high` wire contract.
//!
//! The **primary** regression net for Mistral reasoning is the offline,
//! deterministic matrix in `crates/ironclaw_llm/src/mistral/tests.rs`: it
//! already proves — with loopback mock servers, no API key — that
//! `reasoning_effort` is sent for small/medium and omitted for large/off, that
//! the array (`[{thinking},{text}]`) and string responses both parse, the
//! error mapping, and that a prior turn's thinking (+ signature) is replayed
//! as a chunk on the next turn.
//!
//! This live test is a thin smoke layer over the one thing the offline matrix
//! cannot reach: that a *real* Mistral response round-trips through
//! `MistralProvider` without the original
//! `JsonError: did not match any variant of untagged enum ApiResponse` parse
//! failure, and that a second turn replaying the parsed thinking chunk does
//! not 400.
//!
//! ## Why this shape, not a full agent-loop harness
//!
//! The fork's original version of this test drove the real v1 agent loop
//! end-to-end (`ironclaw::channels::OutgoingResponse` + a bespoke
//! `support::live_harness`). That whole harness tier was deleted with the v1
//! monolith (see `docs/internal/live-canary.md`'s `deterministic-replay` row:
//! "its `tests/e2e_live*.rs` fixtures ... were deleted with the v1 monolith").
//! Resurrecting it here would fight that retirement. Instead this test talks
//! to `MistralProvider` directly — the same shape as
//! `reborn_live_github_pat_contract.rs` (a standalone reqwest-shaped live
//! contract check, no shared harness) — which still proves the real thing the
//! offline matrix cannot: an actual Mistral HTTP round-trip through the exact
//! provider code the agent loop calls.
//!
//! ## Running
//!
//! ```bash
//! MISTRAL_API_KEY=... cargo test --test reborn_live_mistral_reasoning_contract -- --ignored --nocapture
//!
//! # Point at the small model instead of the default mistral-medium-latest:
//! MISTRAL_API_KEY=... MISTRAL_MODEL=mistral-small-latest \
//!   cargo test --test reborn_live_mistral_reasoning_contract -- --ignored --nocapture
//! ```

use ironclaw_llm::{
    ChatMessage, CompletionRequest, LlmProvider, MistralReasoningEffort, mistral::MistralProvider,
};
use secrecy::SecretString;

/// The all-labels-wrong box puzzle: a genuine deduction task (not
/// arithmetic/counting), reachable only by reasoning.
const REASONING_PROMPT: &str = "Three boxes are labeled APPLES, ORANGES, and MIXED. You are told \
     that every single label is wrong. You may take exactly one fruit out of exactly one box and \
     look at it. From which one box should you take a fruit so that you can then correctly \
     relabel all three boxes? Think before you answer, then end with \"ANSWER: <box label>\".";

fn provider() -> MistralProvider {
    let api_key = std::env::var("MISTRAL_API_KEY")
        .expect("set MISTRAL_API_KEY to run this live Mistral contract test");
    let model =
        std::env::var("MISTRAL_MODEL").unwrap_or_else(|_| "mistral-medium-latest".to_string());
    MistralProvider::new(
        model,
        SecretString::from(api_key),
        Some(MistralReasoningEffort::High),
        90,
    )
    .expect("MistralProvider::new should build a client")
}

/// Single reasoning turn: proves the real `[{thinking},{text}]` response
/// deserializes and the provider produces a coherent answer (the exact path
/// the original `ApiResponse` parse bug broke).
#[tokio::test]
#[ignore = "requires MISTRAL_API_KEY (live Mistral API call)"]
async fn mistral_reasoning_round_trips() {
    let provider = provider();
    let response = provider
        .complete(CompletionRequest::new(vec![ChatMessage::user(
            REASONING_PROMPT,
        )]))
        .await
        .expect("live Mistral completion should succeed");

    assert!(
        !response.content.trim().is_empty(),
        "expected a non-empty reply from Mistral"
    );
    assert!(
        response.content.to_ascii_uppercase().contains("ANSWER"),
        "expected the model to follow the ANSWER: <box label> format, got: {}",
        response.content
    );
    eprintln!(
        "[MistralLiveContract] reasoning present: {}",
        response.reasoning.is_some()
    );
}

/// Multi-turn: the prior assistant turn (including its parsed thinking +
/// signature) is replayed back to Mistral as a `[{thinking},{text}]` chunk on
/// turn 2. Confirms the live ThinkChunk replay does not 400. Offline C8/turn_two
/// tests prove the builder replays the chunk; this proves the real API accepts
/// it.
#[tokio::test]
#[ignore = "requires MISTRAL_API_KEY (live Mistral API call)"]
async fn mistral_reasoning_multi_turn_replay_does_not_400() {
    let provider = provider();

    let turn1 = provider
        .complete(CompletionRequest::new(vec![ChatMessage::user(
            REASONING_PROMPT,
        )]))
        .await
        .expect("turn 1: live Mistral completion should succeed");
    assert!(!turn1.content.trim().is_empty(), "turn 1: expected a reply");

    // Rebuild the assistant message the agent loop would have stored,
    // carrying the reasoning trace the provider parsed off turn 1 — this is
    // exactly the input `chat_message_to_wire` reconstructs into a `thinking`
    // chunk for turn 2's request.
    let assistant_turn1 =
        ChatMessage::assistant(turn1.content.clone()).with_reasoning(turn1.reasoning.clone());

    let turn2 = provider
        .complete(CompletionRequest::new(vec![
            ChatMessage::user(REASONING_PROMPT),
            assistant_turn1,
            ChatMessage::user("Now briefly restate why that box was the right one to open."),
        ]))
        .await
        .expect(
            "turn 2: live Mistral completion should succeed — an HTTP 400 here means the \
             ThinkChunk replay was rejected",
        );
    assert!(
        !turn2.content.trim().is_empty(),
        "turn 2: expected a reply after replaying the ThinkChunk"
    );

    eprintln!("[MistralLiveContract] multi-turn reasoning replay succeeded across 2 turns");
}
