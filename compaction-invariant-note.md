# Compaction stage — the one invariant to design around first

Quick note before the compaction executor stage gets built, because it's much cheaper to bake this in than to retrofit it.

## The invariant: LLM data is never deleted

This is a load-bearing IronClaw rule (see root `CLAUDE.md`):

> **All LLM output — context fed to the model, reasoning, tool calls, messages, events, steps — is the most valuable data in the system. Never strip, truncate, or delete it from the database. Mark with timestamps, make filterable, but always retain. In-memory HashMaps are caches; the database (via Workspace) is the source of truth. "Cleanup" means evicting from in-memory caches, never deleting database rows.**

So compaction must NOT be "drop/trim old turns." It must be **summarize + evict-from-working-context**, with the full history left durable.

## What that means concretely for the stage

1. **Compaction operates on the in-memory working context, not the durable store.** The stage reads the durable turn/event/step history, produces a summary, and replaces the *working-context window* fed to the model with `summary + recent tail`. The DB rows for the compacted span stay exactly where they are.

2. **The compaction summary is itself durable LLM output.** Persist it (as its own record/event, tenant- and thread-scoped), don't just hold it in memory — it's the thing future turns build on, and QA needs to see it.

3. **Mark, don't remove.** Tag the compacted span (a `compacted_at` timestamp / boundary marker / summary-ref) so you can filter "full history" vs "active window." Nothing gets `DELETE`d. This is exactly what makes it safe for QA: QA can always reconstruct the full, uncompacted trace.

4. **Don't break deterministic resume.** The turns model resumes deterministically from the durable record. A resumed/replayed turn must still be reconstructable from the authoritative history — the summary is *additive forward context*, never the source of truth. Make sure replay reads the durable rows, not the compacted window.

5. **Idempotency / re-compaction.** Compacting an already-compacted span should be deterministic and not lose the earlier boundary (chain summaries, or recompute from the durable span). Key the compaction boundary off something stable, not a wall-clock or random seq.

## Why this matters for QA specifically

QA's whole value is inspecting what the model actually saw and did. If compaction deletes or truncates the durable record, QA loses the evidence and you can't diff "what we sent" against "what happened." Keeping the DB complete + filterable means QA inspects the full trace while the live loop runs on the compacted window — you get both.

## Suggested shape

- New executor stage: `read durable history → summarize compactable span → persist summary record → swap working-context to (summary + tail)`.
- Boundary/trigger: token-budget or turn-count threshold; emit a durable compaction-boundary marker.
- The summary generation is an LLM call → its input/output are themselves retained (recursively respecting the invariant).

Happy to co-design the stage or review the approach once you have a sketch — and to make sure it threads through the turns/workspace source-of-truth model cleanly.
