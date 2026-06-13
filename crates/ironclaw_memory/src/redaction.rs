//! Model-visible redaction helpers for memory export/read surfaces.

use std::sync::LazyLock;

use regex::Regex;

const REDACTION: &str = "[REDACTED - sensitive]";

struct RedactionPattern {
    regex: Regex,
    replacement: &'static str,
}

static REDACTION_PATTERNS: LazyLock<Vec<RedactionPattern>> = LazyLock::new(|| {
    let specs = [
        (
            r"(?i)\b([a-z][a-z0-9+.-]*://[^/\s:@]+:)([^@\s/]+)(@[^ \r\n\t]*)",
            "$1[REDACTED - sensitive]$3",
        ),
        (
            r#"(?im)\b(password|passwd|pwd|api[_-]?key|access[_-]?token|auth[_-]?token|secret|client[_-]?secret)\b(\s*[:=]\s*)(["']?)([^"'\s,;]+)(["']?)"#,
            "$1$2$3[REDACTED - sensitive]$5",
        ),
        (r"(?i)\bsk-(?:proj-)?[a-z0-9_-]{12,}\b", REDACTION),
    ];

    let mut patterns = Vec::with_capacity(specs.len());
    for (pattern, replacement) in specs {
        match Regex::new(pattern) {
            Ok(regex) => patterns.push(RedactionPattern { regex, replacement }),
            Err(error) => {
                tracing::debug!(
                    pattern,
                    error = %error,
                    "memory redaction pattern failed to compile"
                );
            }
        }
    }
    patterns
});

/// Redact secret-like values from memory content before it is exposed through
/// export/read surfaces.
///
/// This is intentionally non-mutating: persisted memory stays intact, and only
/// model-visible/read/export output is sanitized.
pub fn redact_sensitive_memory_content(content: &str) -> String {
    let mut redacted = content.to_string();
    for pattern in REDACTION_PATTERNS.iter() {
        redacted = pattern
            .regex
            .replace_all(&redacted, pattern.replacement)
            .into_owned();
    }
    redacted
}
