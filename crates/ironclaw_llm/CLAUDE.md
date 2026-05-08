# ironclaw_llm guardrails

Owns reusable LLM provider contracts for IronClaw.

- Keep provider-facing types (`LlmProvider`, messages, completion requests/responses, errors, cost helpers) outside the root application crate so Reborn composition crates do not depend on `ironclaw` just to call a model.
- This crate is allowed to define backend/provider abstractions. Reborn loop crates should depend on this crate through narrow adapters such as `HostManagedModelGateway`, not import root `src/llm` or provider clients directly.
- Do not depend on Reborn workflow/runtime crates (`ironclaw_turns`, `ironclaw_loop_support`, `ironclaw_reborn`, dispatcher, capabilities, host runtime). Model calls are an external service boundary, not a turn/run owner.
- Keep raw provider errors and auth details inside provider implementations. Callers that cross user/runtime boundaries must map to safe summaries.
- Add concrete backend implementations here over time; keep root `src/llm` compatibility as migration-only.

Current scope in this branch:

- provider/error contract types extracted from root `src/llm`;
- no concrete provider factory yet.

Tests:

- Unit tests cover provider request normalization, tool-message sanitization, tool-call IDs, and auth error guidance.
- Architecture tests ensure Reborn model gateway wiring uses this crate instead of root `ironclaw::llm`.
