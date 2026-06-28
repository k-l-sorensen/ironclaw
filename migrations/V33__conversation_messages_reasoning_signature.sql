-- Persist the opaque reasoning-block signature (Mistral ThinkChunk `signature`)
-- on each assistant/tool_calls message so the full ThinkChunk — not just the
-- text-flattened trace — is replayed into the LLM on the next user turn. Mistral
-- returns a `signature` on every reasoning block and expects it echoed back so
-- it can verify the replayed block. The value is an opaque token and is NOT
-- leak-scanned. NULL for user messages and for rows written before this column
-- existed. See SIG-1 (sibling of the V32 `reasoning` column / CTR-1).
ALTER TABLE conversation_messages ADD COLUMN reasoning_signature TEXT;
