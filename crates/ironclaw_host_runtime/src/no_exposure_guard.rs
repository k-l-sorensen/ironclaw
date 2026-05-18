//! Lean host-composed no-exposure guard.
//!
//! This service is the host-runtime wrapper around `ironclaw_safety` leak
//! detection. Upper host code should depend on this service instead of wiring
//! `LeakDetector` directly, so production egress policy has one composition
//! seam to grow from.

use ironclaw_safety::{LeakDetectionError, LeakDetector};
use serde_json::{Map, Value};
use std::fmt;
use thiserror::Error;

/// Host boundary being protected by [`NoExposureGuard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ExposureBoundary {
    InboundUserText,
    ModelVisibleToolOutput,
    PublicApi,
    SseEvent,
    DurableEvent,
    LogDiagnostic,
}

impl ExposureBoundary {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InboundUserText => "inbound_user_text",
            Self::ModelVisibleToolOutput => "model_visible_tool_output",
            Self::PublicApi => "public_api",
            Self::SseEvent => "sse_event",
            Self::DurableEvent => "durable_event",
            Self::LogDiagnostic => "log_diagnostic",
        }
    }
}

impl fmt::Display for ExposureBoundary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Stable host no-exposure violation.
///
/// This type must never include raw payload text or leak-detector previews.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("no exposure violation at {boundary}: {code}")]
pub struct NoExposureViolation {
    boundary: ExposureBoundary,
    code: &'static str,
}

impl NoExposureViolation {
    pub const CODE: &'static str = "no_exposure_violation";

    fn new(boundary: ExposureBoundary) -> Self {
        Self {
            boundary,
            code: Self::CODE,
        }
    }

    pub fn boundary(&self) -> ExposureBoundary {
        self.boundary
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

/// Host-owned no-exposure service.
pub struct NoExposureGuard {
    detector: LeakDetector,
}

impl NoExposureGuard {
    pub fn new() -> Self {
        Self {
            detector: LeakDetector::new(),
        }
    }

    pub fn with_detector(detector: LeakDetector) -> Self {
        Self { detector }
    }

    /// Check text crossing a host boundary.
    ///
    /// Redactable matches return cleaned text. Blocked matches return a stable
    /// sanitized violation that does not contain the original payload or masked
    /// detector preview.
    pub fn check_text(
        &self,
        boundary: ExposureBoundary,
        text: &str,
    ) -> Result<String, NoExposureViolation> {
        self.detector
            .scan_and_clean(text)
            .map_err(|_| NoExposureViolation::new(boundary))
    }

    /// Recursively check JSON string values and object keys crossing a host boundary.
    pub fn check_json(
        &self,
        boundary: ExposureBoundary,
        value: Value,
    ) -> Result<Value, NoExposureViolation> {
        match value {
            Value::String(text) => self.check_text(boundary, &text).map(Value::String),
            Value::Array(values) => values
                .into_iter()
                .map(|value| self.check_json(boundary, value))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            Value::Object(entries) => {
                let mut checked = Map::with_capacity(entries.len());
                for (key, value) in entries {
                    let key = self.check_text(boundary, &key)?;
                    let value = self.check_json(boundary, value)?;
                    if checked.insert(key, value).is_some() {
                        return Err(NoExposureViolation::new(boundary));
                    }
                }
                Ok(Value::Object(checked))
            }
            value => Ok(value),
        }
    }

    /// Check HTTP egress payloads through the host service wrapper.
    pub fn check_http_request(
        &self,
        boundary: ExposureBoundary,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<(), NoExposureViolation> {
        self.detector
            .scan_http_request(url, headers, body)
            .map_err(|_| NoExposureViolation::new(boundary))
    }
}

impl fmt::Debug for NoExposureGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NoExposureGuard { detector: LeakDetector }")
    }
}

impl Default for NoExposureGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl From<(ExposureBoundary, LeakDetectionError)> for NoExposureViolation {
    fn from((boundary, _): (ExposureBoundary, LeakDetectionError)) -> Self {
        Self::new(boundary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_safety::{LeakAction, LeakPattern, LeakSeverity};
    use regex::Regex;
    use serde_json::json;

    fn guard_with(pattern: &str, action: LeakAction) -> NoExposureGuard {
        NoExposureGuard::with_detector(LeakDetector::with_patterns(vec![LeakPattern {
            name: "sentinel".to_string(),
            regex: Regex::new(pattern).expect("valid regex"),
            severity: LeakSeverity::Critical,
            action,
        }]))
    }

    #[test]
    fn check_text_redacts_without_blocking() {
        let guard = guard_with("SECRET-[0-9]+", LeakAction::Redact);

        let checked = guard
            .check_text(
                ExposureBoundary::ModelVisibleToolOutput,
                "value=SECRET-12345",
            )
            .expect("redactable payload should pass");

        assert_eq!(checked, "value=[REDACTED]");
    }

    #[test]
    fn check_text_blocks_with_sanitized_error() {
        let guard = guard_with("SECRET-[0-9]+", LeakAction::Block);

        let error = guard
            .check_text(ExposureBoundary::PublicApi, "value=SECRET-12345")
            .expect_err("blocked payload should fail");

        assert_eq!(error.code(), NoExposureViolation::CODE);
        assert_eq!(error.boundary(), ExposureBoundary::PublicApi);
        assert!(!error.to_string().contains("SECRET-12345"));
        assert!(!error.to_string().contains("sentinel"));
    }

    #[test]
    fn check_json_recursively_sanitizes_strings_and_keys() {
        let guard = guard_with("SECRET-[0-9]+", LeakAction::Redact);
        let value = json!({
            "safe": ["SECRET-12345", {"SECRET-67890": "ok"}],
            "number": 1
        });

        let checked = guard
            .check_json(ExposureBoundary::DurableEvent, value)
            .expect("redactable json should pass");

        assert_eq!(
            checked,
            json!({
                "safe": ["[REDACTED]", {"[REDACTED]": "ok"}],
                "number": 1
            })
        );
    }

    #[test]
    fn check_json_blocks_secret_values() {
        let guard = guard_with("SECRET-[0-9]+", LeakAction::Block);
        let value = json!({"nested": {"value": "SECRET-12345"}});

        let error = guard
            .check_json(ExposureBoundary::SseEvent, value)
            .expect_err("blocked json should fail");

        assert_eq!(error.code(), NoExposureViolation::CODE);
        assert_eq!(error.boundary(), ExposureBoundary::SseEvent);
        assert!(!error.to_string().contains("SECRET-12345"));
    }
}
