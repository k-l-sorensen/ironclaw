use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};

use crate::OAuthRuntime;

#[derive(Debug, Clone, Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: Option<String>,
    pub state: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallbackResponse {
    pub provider_id: String,
    pub success: bool,
}

pub fn router(runtime: OAuthRuntime) -> Router {
    Router::new()
        .route("/auth/callback/{provider_id}", get(callback_handler))
        .with_state(runtime)
}

async fn callback_handler(
    State(runtime): State<OAuthRuntime>,
    Path(provider_id): Path<String>,
    Query(query): Query<OAuthCallbackQuery>,
) -> Result<Json<CallbackResponse>, (StatusCode, Json<CallbackResponse>)> {
    if query.error.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(CallbackResponse {
                provider_id,
                success: false,
            }),
        ));
    }
    let Some(code) = query.code else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(CallbackResponse {
                provider_id,
                success: false,
            }),
        ));
    };
    runtime
        .exchange(&provider_id, code, query.state)
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(CallbackResponse {
                    provider_id: provider_id.clone(),
                    success: false,
                }),
            )
        })?;
    Ok(Json(CallbackResponse {
        provider_id,
        success: true,
    }))
}
