-- Persist the provider-emitted reasoning trace (already leak-scanned) on each
-- assistant/tool_calls message so it can be replayed into the LLM on the next
-- user turn. Mistral requires the prior ThinkChunk, and DeepSeek/Gemini reject
-- the follow-up request without the echo. NULL for user messages and for rows
-- written before this column existed. See CTR-1.
ALTER TABLE conversation_messages ADD COLUMN reasoning TEXT;
