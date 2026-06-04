use ironclaw_product_adapters::{
    ProductAdapterError, ProductWorkflowRejectionKind, RedactedString,
};
use ironclaw_reborn_openai_compat::{
    OpenAiCompatErrorCode, OpenAiCompatErrorKind, OpenAiCompatErrorResponse, OpenAiCompatErrorType,
    OpenAiCompatHttpError,
};
use serde_json::json;

#[test]
fn workflow_rejection_maps_to_stable_openai_error_envelope() {
    let error = OpenAiCompatHttpError::from_workflow_rejection(
        ProductWorkflowRejectionKind::Unauthorized,
        403,
        false,
        Some("response_id".to_string()),
    );

    assert_eq!(error.status_code(), 403);
    assert!(!error.retryable());
    assert_eq!(
        error.body().error.error_type(),
        OpenAiCompatErrorType::PermissionError
    );
    assert_eq!(
        error.body().error.code(),
        Some(OpenAiCompatErrorCode::PermissionDenied)
    );
    assert_eq!(error.body().error.param(), Some("response_id"));

    let serialized = serde_json::to_value(error.body()).expect("serialize error");
    assert_eq!(serialized["error"]["type"], "permission_error");
    assert_eq!(serialized["error"]["code"], "permission_denied");
}

#[test]
fn busy_and_transient_failures_keep_retryable_status_mapping() {
    let busy = OpenAiCompatHttpError::from_workflow_rejection(
        ProductWorkflowRejectionKind::ThreadBusy,
        429,
        true,
        None,
    );
    assert_eq!(busy.status_code(), 429);
    assert!(busy.retryable());
    assert_eq!(
        busy.body().error.code(),
        Some(OpenAiCompatErrorCode::RateLimited)
    );

    let transient =
        OpenAiCompatHttpError::from_product_adapter_error(ProductAdapterError::WorkflowTransient {
            reason: RedactedString::new("store down /host/path secret-token"),
        });
    assert_eq!(transient.status_code(), 503);
    assert!(transient.retryable());
    assert_eq!(
        transient.body().error.code(),
        Some(OpenAiCompatErrorCode::ServiceUnavailable)
    );
}

#[test]
fn error_mapping_does_not_serialize_backend_or_secret_details() {
    let error = OpenAiCompatHttpError::from_product_adapter_error(ProductAdapterError::Internal {
        detail: RedactedString::new(
            "RAW_PROMPT_SENTINEL provider stack /host/path /Users/alice secret-token sk-live",
        ),
    });
    let rendered = serde_json::to_string(error.body()).expect("serialize error");

    for forbidden in [
        "RAW_PROMPT_SENTINEL",
        "provider stack",
        "/host/path",
        "/Users/alice",
        "secret-token",
        "sk-live",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "error body leaked forbidden detail {forbidden:?}: {rendered}"
        );
    }
}

#[test]
fn suspicious_error_params_are_dropped_instead_of_normalized() {
    let error = OpenAiCompatHttpError::from_kind(
        400,
        false,
        OpenAiCompatErrorKind::Validation,
        Some(" RAW_PROMPT_SENTINEL ".to_string()),
    );
    assert_eq!(error.body().error.param(), None);
}

#[test]
fn error_envelope_rejects_unknown_fields() {
    let err = serde_json::from_value::<OpenAiCompatErrorResponse>(json!({
        "error": {
            "message": "The request is invalid.",
            "type": "invalid_request_error",
            "param": null,
            "code": "invalid_request",
            "debug": "must reject"
        }
    }))
    .expect_err("unknown fields must reject");
    assert!(err.to_string().contains("unknown field"));
}
