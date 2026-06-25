# Mistral Reasoning — Knowledge & Constraints (fork research)

> Reference notes gathered while investigating how to make IronClaw use Mistral
> reasoning "to the fullest" (`reasoning_effort=high`). Captured so the
> architecture/implementation plan can be made without re-deriving any of it.
> This is **research/documentation only** — the exploratory code was reverted;
> see `CLAUDE-local.md`. Two live test scripts are retained under `scripts/`.
>
> Sources: https://docs.mistral.ai/studio-api/conversations/reasoning ,
> Mistral `/v1/chat/completions` API reference, and live testing against
> `mistral-medium-latest` on 2026-06-23.

## 1. How Mistral reasoning works

### Request parameter

- **`reasoning_effort`** is the control. Documented values are **`"high"`** and
  **`"none"`** — it is effectively a **boolean**, *not* the OpenAI-style
  `low`/`medium`/`high` scale. (An earlier assumption that it was low/medium/high
  was wrong — do not model it as a 3-level enum.)
  - `"high"` — full thinking trace is produced and surfaced.
  - `"none"` — no thinking trace.
  - omitted — model default (for these models, behaves like plain output).
- There is also a `prompt_mode` knob in some SDK surfaces (controls whether
  Mistral injects its own default reasoning system prompt), but the reasoning
  page treats `reasoning_effort` as the primary control. **Not needed** for our
  purpose and risks layering Mistral's system prompt over IronClaw's.
- Available on the chat-completions endpoint, and on the agents/conversations
  endpoints via a `completion_args` field.

### Supported models

- `mistral-small-latest`
- `mistral-medium-latest` / `mistral-medium-3-5` (`high` recommended for
  agentic/code use cases)
- **Not** `mistral-large`, `mistral-7b`, `mistral-tiny`, `mistral-nemo`, or
  embedding models. The dedicated **Magistral** reasoning models are deprecated;
  reasoning now rides on the general small/medium models.

### Response format — THE crux

The shape of `message.content` is dictated by `reasoning_effort`:

| `reasoning_effort` | `message.content` |
|---|---|
| `"high"` | **array of chunks** (see below) |
| `"none"` / omitted | **plain string** |

When `"high"`, `message.content` is a list of typed chunks. **The thinking is a
separate chunk from the message** — they are distinct entries in the `content`
array, not a single string with the reasoning inlined:

```jsonc
"content": [
  { "type": "thinking",
    "thinking": [
      { "type": "text", "text": "... first reasoning step ..." },
      { "type": "text", "text": "... later reasoning step ..." }
    ] },
  { "type": "text", "text": "... final answer ..." }
]
```

- `ThinkChunk` — `type: "thinking"`; its `thinking` field is itself a **list of
  `TextChunk`** (one or more text fragments making up that turn's reasoning
  trace). Treat it as a list, not a single string — a turn's thinking can span
  several `TextChunk` entries.
- `TextChunk` — `type: "text"`; when it appears at the top level of `content` it
  is the final answer (the message). The same `TextChunk` type is reused *inside*
  a `ThinkChunk`'s `thinking` list for the reasoning fragments.
- (Other chunk types exist in the same union: `reference`, `file`, `audio`.)

There is **no option to get the reasoning as inline `<think>…</think>` tags
inside a string** (the DeepSeek style). With `reasoning_effort=high` the array
is mandatory. So you cannot sidestep array parsing by asking for string output —
that would mean `reasoning_effort=none`, i.e. no reasoning.

### Multi-turn requirement

The docs are explicit: to avoid degraded performance, **replay the full
assistant message back into history on subsequent turns — both the `ThinkChunk`
*and* the final `TextChunk` (the message), in the same array shape they were
received.** Do not strip the `ThinkChunk` before replaying; the model relies on
the reasoning trace to stay coherent across turns. It is not enough to replay
only the final message text, nor only the thinking — the whole `content` array
(thinking chunk + message chunk) must be sent back.

So a correct integration must both (a) parse the array out (separating the
thinking from the message) and (b) reconstruct and send that full array —
thinking chunk *and* message chunk — back in on the next turn.

## 2. Why it does not work in IronClaw today (`rig-core 0.30`)

Mistral is wired in `providers.json` as `id: "mistral"`,
`protocol: "open_ai_completions"`, so it flows through
`crates/ironclaw_llm/src/lib.rs::create_openai_compat_from_registry` →
`RigAdapter` over rig-core's OpenAI Chat Completions client.

**Neither rig-core 0.30 path can consume the array-shaped reasoning response:**

1. **Generic OpenAI-compat client** (current path): its response model expects
   `message.content` to be a **string**. The array response fails to
   deserialize:
   `JsonError: data did not match any variant of untagged enum ApiResponse`.
   (Observed live: every agent turn fails, retried 3× then errors.)

2. **rig-core's dedicated `mistral` client**
   (`rig-core-0.30.0/src/providers/mistral/`): also models assistant `content`
   as `String`, and it **`panic!`s** on reasoning content when building requests
   from history:
   `panic!("Reasoning content is not currently supported on Mistral via Rig")`
   (`completion.rs` ~lines 159 and 628). So switching Mistral to a dedicated
   protocol (the fix that solved DeepSeek `reasoning_content` and Gemini
   `thought_signature`) does **not** rescue Mistral here.

Conclusion: with `rig-core` pinned at 0.30, there is **no off-the-shelf path**
that parses Mistral's reasoning response. The codebase's own rule
(`registry.rs` test `reasoning_aware_providers_use_dedicated_protocol_not_openai_compat`)
says reasoning-aware providers must avoid `OpenAiCompletions` — but for Mistral
the dedicated rig client is also a dead end at 0.30.

## 3. Evidence (reproducible)

- `scripts/test-mistral-reasoning.sh` — raw Mistral API, with vs. without
  `reasoning_effort`. **PASS**: `reasoning_effort=high` returns
  `content` as an array containing a `thinking` part; without it, `content` is a
  string. Confirms Mistral honors the field.
- Driving the real agent against the Mistral backend with reasoning on
  originally **FAILED** with `JsonError: did not match any variant of untagged
  enum ApiResponse`, confirming IronClaw's receive path could not parse the
  array-shaped response (the bug this work fixes). That end-to-end check now
  lives as the Live-tier test `tests/e2e_live_mistral_reasoning.rs` (run with
  `IRONCLAW_LIVE_TEST=1 LLM_BACKEND=mistral`), which must PASS post-fix; it
  superseded the earlier `scripts/test-mistral-reasoning-ironclaw.sh` bash harness.
- Both scripts read `MISTRAL_API_KEY` from the environment; set/export it before
  running (sourced from your own secret manager). No vault reference is committed.

## 3a. FIRST STEP before any build — resolve build-vs-upgrade

**Before committing to a custom provider, check whether a newer `rig-core`
version natively supports Mistral reasoning** (array `content` parsing +
round-tripping the `ThinkChunk`). This is a gating decision — it changes the
entire shape of the work:

- If a newer `rig-core` handles it → the fix may be a **dependency bump** (small,
  but verify it doesn't regress the other providers pinned to 0.30's behavior).
- If not → build the **custom Mistral provider** (Section 4).

How to check: review `rig-core` releases > 0.30 on crates.io / its repo
changelog and `providers/mistral/completion.rs` in the newer version; look for
array-`content` deserialization and removal of the
`"Reasoning ... not ... supported on Mistral via Rig"` `panic!`s. Validate any
candidate version with the live acceptance test
`tests/e2e_live_mistral_reasoning.rs` before adopting.

## 4. Implications for the design (inputs to the plan)

To use `reasoning_effort=high` properly, IronClaw must own the Mistral
request/response, not delegate to rig-core 0.30. Likely shape (to be decided in
the architectural plan):

- A **custom Mistral provider** (sibling to `nearai_chat.rs`, the Codex
  providers, etc.) implementing `LlmProvider` with its own HTTP client and
  JSON model that:
  - sends `reasoning_effort` on the request (gated to small/medium models);
  - parses array `content` → IronClaw's `reasoning` field (from the
    `thinking` chunk) + `content` (from the `text` chunk), for both
    `complete` and `complete_with_tools`;
  - on the next turn, reconstructs the assistant message **including** the
    thinking content (multi-turn requirement) — IronClaw already round-trips a
    `reasoning` field on `ChatMessage` (see `rig_adapter.rs` reasoning handling
    and `provider.rs`), so there is an existing channel to reuse;
  - maps Mistral error bodies to `LlmError` at the channel boundary.
- A new `ProviderProtocol::Mistral` variant + `providers.json` switch from
  `open_ai_completions` to `mistral`, and factory dispatch in `lib.rs`.
- Config: a **boolean-ish** reasoning toggle (high/off), not a 3-level enum.
  Env read stays in the binary's `src/config/llm.rs` (crate stays env-agnostic).
- **Gating decision first:** resolve build-vs-upgrade (Section 3a) before
  building the custom provider — a newer `rig-core` may handle this natively.

## 5. Decisions already locked by the user

- Goal is to use Mistral **to the fullest → `reasoning_effort=high`** (Mistral is
  the largest EU provider; treat it as a first-class, properly-supported path).
- Do it **properly** (not a minimal carry). Architecture + code plan to be made
  in a fresh conversation.
