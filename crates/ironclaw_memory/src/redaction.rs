//! Model-visible redaction helpers for memory export/read surfaces.

/// Redact secret-like values from memory content before it is exposed through
/// export/read surfaces.
///
/// This is intentionally non-mutating: persisted memory stays intact, and only
/// model-visible/read/export output is sanitized.
pub fn redact_sensitive_memory_content(content: &str) -> String {
    ironclaw_safety::redact_sensitive_values(content)
}
