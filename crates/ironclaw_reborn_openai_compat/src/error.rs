use ironclaw_product_adapters::{ProductAdapterError, ProductWorkflowRejectionKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiCompatErrorKind {
    Validation,
    Authentication,
    PermissionDenied,
    NotFound,
    Conflict,
    RateLimited,
    ServiceUnavailable,
    Unsupported,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiCompatErrorType {
    InvalidRequestError,
    AuthenticationError,
    PermissionError,
    NotFoundError,
    ConflictError,
    RateLimitError,
    ServerError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiCompatErrorCode {
    InvalidRequest,
    AuthenticationRequired,
    PermissionDenied,
    NotFound,
    Conflict,
    RateLimited,
    ServiceUnavailable,
    Unsupported,
    InternalError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiCompatErrorResponse {
    pub error: OpenAiCompatError,
}

impl OpenAiCompatErrorResponse {
    pub fn new(error: OpenAiCompatError) -> Self {
        Self { error }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiCompatError {
    message: String,
    #[serde(rename = "type")]
    error_type: OpenAiCompatErrorType,
    param: Option<String>,
    code: Option<OpenAiCompatErrorCode>,
}

impl OpenAiCompatError {
    pub fn from_kind(kind: OpenAiCompatErrorKind, param: Option<String>) -> Self {
        let spec = ErrorSpec::for_kind(kind);
        Self {
            message: spec.message.to_string(),
            error_type: spec.error_type,
            param: clean_param(param),
            code: Some(spec.code),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn error_type(&self) -> OpenAiCompatErrorType {
        self.error_type
    }

    pub fn param(&self) -> Option<&str> {
        self.param.as_deref()
    }

    pub fn code(&self) -> Option<OpenAiCompatErrorCode> {
        self.code
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiCompatHttpError {
    status_code: u16,
    retryable: bool,
    body: OpenAiCompatErrorResponse,
}

impl OpenAiCompatHttpError {
    pub fn from_kind(
        status_code: u16,
        retryable: bool,
        kind: OpenAiCompatErrorKind,
        param: Option<String>,
    ) -> Self {
        Self {
            status_code: sanitize_status_code(status_code),
            retryable,
            body: OpenAiCompatErrorResponse::new(OpenAiCompatError::from_kind(kind, param)),
        }
    }

    pub fn invalid_request(param: Option<String>) -> Self {
        Self::from_kind(400, false, OpenAiCompatErrorKind::Validation, param)
    }

    pub fn not_wired() -> Self {
        Self::from_kind(501, false, OpenAiCompatErrorKind::Unsupported, None)
    }

    pub fn from_workflow_rejection(
        kind: ProductWorkflowRejectionKind,
        status_code: u16,
        retryable: bool,
        param: Option<String>,
    ) -> Self {
        let error_kind = match kind {
            ProductWorkflowRejectionKind::ThreadBusy
            | ProductWorkflowRejectionKind::AdmissionRejected => OpenAiCompatErrorKind::RateLimited,
            ProductWorkflowRejectionKind::ScopeNotFound => OpenAiCompatErrorKind::NotFound,
            ProductWorkflowRejectionKind::Unauthorized => OpenAiCompatErrorKind::PermissionDenied,
            ProductWorkflowRejectionKind::InvalidRequest => OpenAiCompatErrorKind::Validation,
            ProductWorkflowRejectionKind::Unavailable => OpenAiCompatErrorKind::ServiceUnavailable,
            ProductWorkflowRejectionKind::Conflict => OpenAiCompatErrorKind::Conflict,
        };
        Self::from_kind(status_code, retryable, error_kind, param)
    }

    pub fn from_product_adapter_error(error: ProductAdapterError) -> Self {
        match error {
            ProductAdapterError::InvalidIdentifier { .. }
            | ProductAdapterError::MalformedInboundPayload { .. } => Self::invalid_request(None),
            ProductAdapterError::Authentication(_) => {
                Self::from_kind(401, false, OpenAiCompatErrorKind::Authentication, None)
            }
            ProductAdapterError::WorkflowRejected {
                kind,
                status_code,
                retryable,
                ..
            } => Self::from_workflow_rejection(kind, status_code, retryable, None),
            ProductAdapterError::WorkflowTransient { .. }
            | ProductAdapterError::EgressTransient { .. } => {
                Self::from_kind(503, true, OpenAiCompatErrorKind::ServiceUnavailable, None)
            }
            ProductAdapterError::EgressDenied { .. }
            | ProductAdapterError::EgressUndeclaredHost { .. }
            | ProductAdapterError::Internal { .. } => {
                Self::from_kind(500, false, OpenAiCompatErrorKind::Internal, None)
            }
        }
    }

    pub fn internal() -> Self {
        Self::from_kind(500, false, OpenAiCompatErrorKind::Internal, None)
    }

    pub fn status_code(&self) -> u16 {
        self.status_code
    }

    pub fn retryable(&self) -> bool {
        self.retryable
    }

    pub fn body(&self) -> &OpenAiCompatErrorResponse {
        &self.body
    }
}

impl From<ProductAdapterError> for OpenAiCompatHttpError {
    fn from(error: ProductAdapterError) -> Self {
        Self::from_product_adapter_error(error)
    }
}

#[cfg(feature = "openai-compat-beta")]
impl axum::response::IntoResponse for OpenAiCompatHttpError {
    fn into_response(self) -> axum::response::Response {
        use axum::Json;
        use axum::http::StatusCode;

        let status = StatusCode::from_u16(self.status_code).unwrap_or_else(|_| {
            tracing::error!(
                target = "ironclaw_reborn_openai_compat::error",
                status_code = self.status_code,
                "OpenAI-compatible error carried a non-HTTP status; coercing to 500"
            );
            StatusCode::INTERNAL_SERVER_ERROR
        });
        (status, Json(self.body)).into_response()
    }
}

#[derive(Debug, Clone, Copy)]
struct ErrorSpec {
    message: &'static str,
    error_type: OpenAiCompatErrorType,
    code: OpenAiCompatErrorCode,
}

impl ErrorSpec {
    fn for_kind(kind: OpenAiCompatErrorKind) -> Self {
        match kind {
            OpenAiCompatErrorKind::Validation => Self {
                message: "The request is invalid.",
                error_type: OpenAiCompatErrorType::InvalidRequestError,
                code: OpenAiCompatErrorCode::InvalidRequest,
            },
            OpenAiCompatErrorKind::Authentication => Self {
                message: "Authentication is required.",
                error_type: OpenAiCompatErrorType::AuthenticationError,
                code: OpenAiCompatErrorCode::AuthenticationRequired,
            },
            OpenAiCompatErrorKind::PermissionDenied => Self {
                message: "The caller is not allowed to access this resource.",
                error_type: OpenAiCompatErrorType::PermissionError,
                code: OpenAiCompatErrorCode::PermissionDenied,
            },
            OpenAiCompatErrorKind::NotFound => Self {
                message: "The requested resource was not found.",
                error_type: OpenAiCompatErrorType::NotFoundError,
                code: OpenAiCompatErrorCode::NotFound,
            },
            OpenAiCompatErrorKind::Conflict => Self {
                message: "The request conflicts with the current resource state.",
                error_type: OpenAiCompatErrorType::ConflictError,
                code: OpenAiCompatErrorCode::Conflict,
            },
            OpenAiCompatErrorKind::RateLimited => Self {
                message: "The request is temporarily rate limited.",
                error_type: OpenAiCompatErrorType::RateLimitError,
                code: OpenAiCompatErrorCode::RateLimited,
            },
            OpenAiCompatErrorKind::ServiceUnavailable => Self {
                message: "The service is temporarily unavailable.",
                error_type: OpenAiCompatErrorType::ServerError,
                code: OpenAiCompatErrorCode::ServiceUnavailable,
            },
            OpenAiCompatErrorKind::Unsupported => Self {
                message: "This OpenAI-compatible Reborn route is not wired yet.",
                error_type: OpenAiCompatErrorType::InvalidRequestError,
                code: OpenAiCompatErrorCode::Unsupported,
            },
            OpenAiCompatErrorKind::Internal => Self {
                message: "An internal error occurred.",
                error_type: OpenAiCompatErrorType::ServerError,
                code: OpenAiCompatErrorCode::InternalError,
            },
        }
    }
}

fn sanitize_status_code(status_code: u16) -> u16 {
    if (400..=599).contains(&status_code) {
        status_code
    } else {
        500
    }
}

fn clean_param(param: Option<String>) -> Option<String> {
    let value = param?;
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed != value
        || trimmed.len() > 128
        || trimmed.chars().any(|ch| ch == '\0' || ch.is_control())
        || contains_no_exposure_sentinel(trimmed)
    {
        return None;
    }
    Some(value)
}

fn contains_no_exposure_sentinel(value: &str) -> bool {
    const NO_EXPOSURE_SENTINELS: &[&str] = &[
        "RAW_PROMPT_SENTINEL",
        "SECRET_SENTINEL",
        "secret-token",
        "sk-live",
        "/host/path",
        "/Users/",
    ];
    NO_EXPOSURE_SENTINELS
        .iter()
        .any(|sentinel| value.contains(sentinel))
}
