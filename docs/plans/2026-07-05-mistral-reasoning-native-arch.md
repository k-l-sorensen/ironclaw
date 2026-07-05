# Mistral Reasoning — Native `reasoning_details` Architecture (C4 Level 3)

> **Status:** Approved architecture (C4 L3, component level). Date 2026-07-05.
> Scope: enable `reasoning_effort=high` for **Mistral Medium 3.5** and **Mistral Small 4**
> on **IronClaw v1** (the path in use today) and Reborn. Tracks fork issue
> [k-l-sorensen/ironclaw#8](https://github.com/k-l-sorensen/ironclaw/issues/8).
>
> **Supersedes** `docs/plans/2026-06-24-mistral-reasoning-provider-architecture.md` (the
> retired custom-provider design). That doc is retained for its preserved analysis (decisions
> D1–D10) and acceptance criteria; the parts it built on top of — `ReasoningBlock`, the CTR-1
> cross-turn plumb, the SIG-1 signature threading (~35 files) — are **withdrawn**: upstream now
> owns them. Companion research: `docs/providers/mistral-reasoning.md` (provider-agnostic API
> findings, still valid).
>
> This document is **purely C4 Level 3** — components and the seams between them. It is not a
> code-implementation plan; the code-level work-unit breakdown is a separate follow-up.

---

## 1. Context — why re-architect

The goal (locked by the user): use Mistral **to the fullest → `reasoning_effort=high`**, since
Mistral is the largest EU provider and should be a first-class, properly-supported path.

**What changed since the retired design.** Upstream shipped a general, provider-agnostic
reasoning pipeline that the fork's custom carry now duplicates. Present in-tree today:

- `ReasoningDetail` enum — `crates/ironclaw_llm/src/provider.rs:78` —
  `{ Text { text, signature }, Encrypted(String), Redacted { data }, Summary(String) }`,
  serde-tagged (`#[serde(tag = "type", content = "content", rename_all = "snake_case")]`).
- `ReasoningDetails { id, content: Vec<ReasoningDetail> }` — `provider.rs:95`; `from_text`,
  `is_empty` (treats a signature-only `Text` as non-empty — preserves Gemini `thought_signature`),
  `display_text`.
- `ChatMessage.reasoning: Option<String>` (`:182`) + `ChatMessage.reasoning_details:
  Option<ReasoningDetails>` (`:186`); builders `with_reasoning` (`:278`), `with_reasoning_details`
  (`:292`, keeps the legacy string in sync while preserving opaque/signature blocks).
- `ToolCompletionResponse.reasoning_details` — `provider.rs:606`.
- Full bidirectional round-trip in `crates/ironclaw_llm/src/rig_adapter.rs`:
  `iron_reasoning_to_rig` (`:615`), `rig_reasoning_to_iron` (`:640`), collected in
  `extract_response` (`:689`), emitted in `convert_messages` (`:414`, `:456`).
- Persistence, cross-turn replay, and per-block signatures ride this typed channel — so the
  retired `ReasoningBlock`/CTR-1/SIG-1 carry is **no longer needed** (a per-block signature is
  already `ReasoningDetail::Text.signature`).

**The gap that remains (why any Mistral-specific component is still needed).** rig-core is now
pinned at **0.33** (`crates/ironclaw_llm/Cargo.toml`), but neither rig path carries Mistral
reasoning:

1. **Generic `open_ai_completions`** (Mistral's current wiring) models `message.content` as a
   `String`; Mistral's `reasoning_effort=high` response is an **array of typed chunks**, so it
   fails to deserialize — `JsonError: did not match any variant of untagged enum ApiResponse`
   (every turn fails, retried, then errors).
2. **rig-core 0.33's dedicated Mistral client** (`providers/mistral/completion.rs`) also models
   assistant `content` as a plain `String`, **silently drops** `AssistantContent::Reasoning` on
   send, and **never parses** reasoning on receive. Switching Mistral to a rig-mistral protocol
   does not rescue it.

**Conclusion.** IronClaw must own the Mistral request/response at the wire boundary — but only to
*translate* Mistral's chunk array onto upstream's existing `reasoning_details` seam. One thin new
component; everything downstream is inherited.

---

## 2. Mistral model catalog (which models support `reasoning_effort=high`)

Verified 2026-07-05 against `docs.mistral.ai/studio-api/conversations/reasoning`, promptfoo, and
OpenRouter.

| Model | API id | Aliases | `reasoning_effort=high`? |
|---|---|---|---|
| **Mistral Medium 3.5** | `mistral-medium-2604` | `mistral-medium-latest`, `mistral-medium-3.5` | **Yes** — recommended for agentic/code |
| **Mistral Small 4** | `mistral-small-2603` | `mistral-small-latest`, `magistral-small-latest` | **Yes** — hybrid model |
| Mistral Large | `mistral-large-latest` | — | No |
| tiny / nemo / 7b / embeddings | — | — | No |

Key facts to honour in the design:

- **`reasoning_effort` is boolean-ish: `"high"` | `"none"`** — *not* the OpenAI low/medium/high
  scale. Model it as a two-variant enum, never a 3-level enum.
- At `"high"`, `message.content` is a **chunk array**: a `ThinkChunk` (`type:"thinking"`, whose
  `thinking` is itself a list of `TextChunk`) followed by a `TextChunk` (`type:"text"`, the final
  answer). At `"none"`/omitted, `content` is a plain string.
- The **thinking is a separate chunk from the message** — parsing must split them, and multi-turn
  replay must send the whole `content` array (thinking chunk *and* message chunk) back.
- Standalone **Magistral** reasoning models are **deprecated**; reasoning now rides the general
  small/medium models. `magistral-small-latest` now aliases Mistral Small 4.
- Recommended sampling at `high`: `temperature=0.7`, `top_p=0.95`.

---

## 3. C4 L3 — View A: v1 / shared `ironclaw_llm` crate (the path in use today)

`ironclaw_llm` is shared by both the v1 monolith and Reborn, so this view delivers the provider
for both. It adds **exactly one new component** and threads a typed reasoning-effort value from
the binary's env layer to it; everything else is reused.

### Component diagram

```
  ┌──────────────────────────────────────────────────────────────────────────┐
  │ BINARY (env boundary)                                                      │
  │   src/config/llm.rs  ──reads MISTRAL_REASONING (high|none, default high)── │
  └───────────────┬──────────────────────────────────────────────────────────┘
                  │ typed MistralReasoningEffort (High|None) via RegistryProviderConfig
                  ▼
  ┌──────────────────────────────────────────────────────────────────────────┐
  │ crates/ironclaw_llm                                                        │
  │                                                                            │
  │   lib.rs factory dispatch                                                  │
  │     match ProviderProtocol::Mistral → create_mistral_from_registry(...)    │
  │                          │                                                 │
  │                          ▼                                                 │
  │   ┌─────────────────────────────────────────────┐   gate:                 │
  │   │ NEW  MistralProvider  (src/mistral.rs)       │◄── reasoning_models.rs  │
  │   │  impl LlmProvider (own reqwest + JSON model) │    supports_mistral_    │
  │   │  • REQUEST: set reasoning_effort (gated)     │    reasoning(model)     │
  │   │  • RESPONSE: parse array content →           │                         │
  │   │      ThinkChunk → ReasoningDetail::Text{      │                         │
  │   │                     text, signature}         │                         │
  │   │      TextChunk  → content                    │                         │
  │   │    (also handles plain-string content)       │                         │
  │   │  • complete() AND complete_with_tools()      │                         │
  │   │  • Mistral error body → LlmError (w/ cause)  │                         │
  │   └───────────────────────┬─────────────────────┘                         │
  │                           │ CompletionResponse / ToolCompletionResponse    │
  │                           │   .with_reasoning_details(...)   ── REUSED ──   │
  │                           ▼                                                 │
  │   build_provider_chain()  (Retry → SmartRouting → Failover →               │
  │                            CircuitBreaker → Cached → Swappable →           │
  │                            Recording)                      ── UNCHANGED ──  │
  └───────────────────────────┬──────────────────────────────────────────────┘
                              │ ChatMessage.reasoning_details (round-tripped)
                              ▼
   Agent loop / Reasoning engine  +  SafetyLayer leak-scan before user  ── REUSED/UNCHANGED ──
```

Multi-turn replay uses the **existing** `rig_adapter.rs` round-trip: on the next turn the prior
assistant `reasoning_details` is re-emitted as `AssistantContent::Reasoning`. The `MistralProvider`
translates that back into Mistral's `content` array shape on the wire.

### Components touched

| # | Component | Location | Change |
|---|---|---|---|
| A1 | **`MistralProvider`** (new) | `crates/ironclaw_llm/src/mistral.rs` (new) | `impl LlmProvider`; own reqwest client + JSON model; request sets `reasoning_effort` gated to supported models and replays prior thinking from `reasoning_details`; response parses array `content` → splits `ThinkChunk`→`ReasoningDetail::Text{text,signature}` + `TextChunk`→`content`, also handles string content; `complete()` + `complete_with_tools()`; maps error bodies → `LlmError`. |
| A2 | **Protocol enum** | `registry.rs` (`ProviderProtocol`, `:62`) | Add `Mistral` variant (`#[serde(rename_all="snake_case")]` → wire `mistral`); include in `has_dedicated_config()` (`:122`). |
| A3 | **Factory dispatch** | `crates/ironclaw_llm/src/lib.rs` | Add `create_mistral_from_registry(...)`, mirroring `create_deepseek_from_registry` (`:628`) / `create_openrouter_from_registry` (`:682`) / `create_gemini_from_registry` (`:775`); wrap so it still flows through `build_provider_chain` (`:1187`). |
| A4 | **Reasoning-model gate** | `crates/ironclaw_llm/src/reasoning_models.rs` | Add `supports_mistral_reasoning(model)` (pattern-match `mistral-small`/`mistral-medium`; exclude large/tiny/nemo), alongside `supports_openai_reasoning` (`:78`) etc. |
| A5 | **Reasoning-effort type** | `crates/ironclaw_llm/src/mistral.rs` (or a small sibling module) | `enum MistralReasoningEffort { High, None }`, wire-stable (`#[serde(rename_all="snake_case")]`). Owned by `ironclaw_llm`; not `ironclaw_common`. |
| A6 | **Provider schema** | `registry.rs` (`ProviderDefinition`, `:327`) | Add optional reasoning-effort field carrying `MistralReasoningEffort` (`#[serde(deny_unknown_fields)]` forbids stray keys, so the field must be declared). |
| A7 | **Provider registry** | `providers.json` (Mistral entry, `:418-438`) | `protocol` `open_ai_completions` → `mistral`; `default_model` `mistral-large-latest` → `mistral-medium-latest` (Medium 3.5); default reasoning `high`. |
| A8 | **Env boundary** | `src/config/llm.rs` | Read `MISTRAL_REASONING` (`high`\|`none`, default `high`) → typed `MistralReasoningEffort`. Env parsing stays in the binary; the crate stays env-agnostic. |
| A9 | **Registry guard test** | `registry.rs` (`reasoning_aware_providers_use_dedicated_protocol_not_openai_compat`, `:818`) | Extend to assert Mistral uses `ProviderProtocol::Mistral`, not `open_ai_completions`. |
| A10 | **Reasoning leak-scan** | `SafetyLayer` / `ironclaw_safety::LeakDetector` response seam | Confirm Mistral thinking text flows through the existing response leak scan before user delivery (documented at `src/agent/CLAUDE.md:126`, `src/NETWORK_SECURITY.md`, `crates/ironclaw_llm/src/recording.rs`). Signature is opaque → exempt. **Implementation must confirm the exact reasoning-surfacing call site** (the retired doc's `src/bridge/router.rs` path no longer exists). |
| A11 | **Provider / capability docs** | `docs/capabilities/llm-providers.md` | Add the Mistral reasoning row. |

### Reused — do **NOT** rebuild

- `ReasoningDetail` / `ReasoningDetails` + `with_reasoning_details` — `provider.rs:78/95/292/606`.
- The `rig_adapter.rs` round-trip — `iron_reasoning_to_rig` `:615`, `rig_reasoning_to_iron` `:640`,
  `extract_response` `:689`, `convert_messages` `:414/:456`.
- The decorator chain — `build_provider_chain` (`lib.rs:1187`).
- Model-capability registries — `reasoning_models.rs`, `vision_models.rs`.
- The `LlmProvider` trait — the single seam; **no trait changes**.
- Upstream persistence + cross-turn replay + per-block signature handling.

**Explicitly withdrawn from the retired design:** `ReasoningBlock`, the CTR-1 cross-turn `reasoning`
plumb, the SIG-1 `reasoning_signature` threading, and any parallel `reasoning` field. The signature
rides in `ReasoningDetail::Text.signature`; cross-turn replay rides `reasoning_details`.

---

## 4. C4 L3 — View B: Reborn follow-up

Because `ironclaw_llm` is shared, View A already delivers the provider to Reborn. This view is a
**verification-first** pass — it is expected to add **no new component**, only to confirm (or
close a gap in) the flow of `reasoning_details` through Reborn's persistence and context rebuild.

### Component check

```
  MistralProvider ──reasoning_details──► ChatMessage ──► Reborn turn persistence ──► store
                                                    │                                   │
                                                    └──────── context rebuild ◄─────────┘
                                                            (must carry reasoning_details)
```

| Component | Location | Verify |
|---|---|---|
| Reborn conversation record | `crates/ironclaw_reborn_traces/src/conversation_message.rs` (`ConversationMessage`) | Carries `reasoning_details` through persist + hydrate; nothing strips it. |
| Turn persistence / context rebuild | Reborn thread/turn store + hydration path | Reasoning survives the write→read round-trip and re-enters context on the next turn. |

### Persistence rule constraint (`database.md`)

- The retired design added dedicated `reasoning` columns + dual-backend migrations (PG `V32` /
  libSQL `v26`). **This re-architecture does not.** Upstream already persists `reasoning_details`.
- `database.md` directs new persistence onto the unified `RootFilesystem`/`ScopedFilesystem`
  plane — **not** new `src/db/` sub-traits or per-domain columns.
- LLM reasoning is LLM output → **never stripped or deleted** ("Never Delete LLM Output Data").
- **Finding to record:** if `reasoning_details` already survives the Reborn round-trip, state that
  Reborn inherits reasoning for free and this view adds nothing. If a component drops it, close the
  gap by routing through the existing `reasoning_details` path — not by adding schema.

---

## 5. Decisions

- **D-BUILD (build, don't upgrade).** Own the Mistral wire via `MistralProvider`. A rig-core bump
  does not rescue this — 0.33's Mistral client drops/no-ops reasoning and models content as a
  `String`; the retired doc already checked the latest rig-core and confirmed no native
  array-`content` parsing. **Pre-work gate:** before implementation, re-confirm the current-latest
  rig-core still lacks Mistral array-`content` + reasoning round-trip; if that ever lands upstream,
  reduce this to a dependency bump + protocol switch.
- **D-DEFAULT (reasoning default-on, default model Medium 3.5).** Per the user: `providers.json`
  default reasoning `high`, `default_model` → `mistral-medium-latest`. Large stays available
  (non-reasoning); `supports_mistral_reasoning` gates whether `reasoning_effort` is sent.
- **D-ENUM (two-variant effort).** `MistralReasoningEffort { High, None }` — boolean-ish, never a
  3-level enum. See `types.md`.
- **D-REUSE (map onto upstream).** Translate `ThinkChunk` → `ReasoningDetail::Text{text,signature}`;
  do not mirror or re-declare the upstream reasoning types (`type-placement.md`).

## 6. Acceptance criteria (preserved from the retired live test)

Implementation-independent; the acceptance spec for #8 (from the retired
`tests/e2e_live_mistral_reasoning.rs`, preserved at tag `backup/main-pre-catchup`):

1. **Clean round-trip.** A real Mistral `reasoning_effort=high` response round-trips through the
   agent loop and yields a non-empty reply with **no** `JsonError: did not match any variant of
   untagged enum ApiResponse` (the original blocker).
2. **Multi-turn replay.** A second user turn does **not** HTTP 400 when the prior turn's parsed
   thinking chunk is replayed back to the provider.
3. **Non-goal.** Do not assert on `StatusUpdate::Thinking` events — on the v1 path they are reused
   for generic status. Prove *parsing* deterministically offline; prove *replay* live.
4. **Live invocation (reference):** `IRONCLAW_LIVE_TEST=1 LLM_BACKEND=mistral MISTRAL_REASONING=high
   MISTRAL_API_KEY=… cargo test …`. The offline parser matrix (cases C1–C12) enumerates the
   response shapes that must parse; recover it from the backup tag.

## 7. Rule-compliance constraints (`.claude/rules/`)

| Rule | Constraint baked into the design |
|---|---|
| `types.md` | `reasoning_effort` is an enum, not bool/string; Mistral wire chunks (`ThinkChunk`/`TextChunk`) get named types, not ad-hoc `Value` matching. |
| `type-placement.md` | One definition each, owned by `ironclaw_llm`; **reuse** upstream `ReasoningDetail`/`ReasoningDetails` (a field-for-field mirror + `From` is a violation); no `pub use` shims; nothing added to `ironclaw_common`. |
| `error-handling.md` | Map Mistral error bodies → `LlmError` **carrying the cause** (no `map_err(\|_\| …)` that drops it); detect **413 → `ContextOverflow`**, **5xx → `BadGateway`**; no `.unwrap()`/`.expect()` and no silent-failure on the array-parse path — a malformed reasoning response fails loud, never collapses to empty content. |
| `safety-and-sandbox.md` | Mistral thinking text (LLM output toward the user) passes the response leak detector before delivery (A10); the opaque `signature` is exempt. |
| `architecture.md` | One component through the existing `build_provider_chain` (no second dispatch pipeline); aggregate into a config struct rather than `#[allow(too_many_arguments)]` if the signature nears 7 args; no `Option<Arc<…>>`+`with_*` optional-dep smell; `mistral.rs` aims < 800 lines (split tests into a submodule if needed). |
| `testing.md` | Tiers: offline parser matrix at **Unit** (`cargo test`); live multi-turn replay at **Live** (`cargo test --features integration -- --ignored`, `IRONCLAW_LIVE_TEST=1`). **Test through the caller:** `supports_mistral_reasoning()` gates whether `reasoning_effort` is sent, so assert at the request-build call site that a Small 4 / Medium 3.5 request **carries** `reasoning_effort=high` and a `mistral-large` request **omits** it — a predicate-only unit test is insufficient. |
| `doc-hygiene.md` | No developer-local absolute paths in this or any committed doc. |

## 8. Out of scope

- rig-core bump (revisit only if upstream rig lands native Mistral reasoning — see D-BUILD gate).
- `prompt_mode` (risks layering Mistral's own system prompt over IronClaw's).
- New DB columns / migrations for reasoning (superseded by upstream `reasoning_details`).
- TUI streaming of the thinking trace.
- The code-level work-unit breakdown (a separate follow-up plan).

## References

- `docs/plans/2026-06-24-mistral-reasoning-provider-architecture.md` — superseded; preserved
  analysis (D1–D10) + acceptance criteria.
- `docs/plans/2026-06-24-mistral-reasoning-impl.md` — retired work-unit breakdown; useful for the
  edge cases it enumerates.
- `docs/providers/mistral-reasoning.md` — provider-agnostic API research (still valid).
- Mistral reasoning docs: `docs.mistral.ai/studio-api/conversations/reasoning`.
- `scripts/test-mistral-reasoning.sh` — raw-API probe confirming `reasoning_effort=high` behaviour.
