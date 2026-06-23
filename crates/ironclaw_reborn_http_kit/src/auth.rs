use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use ironclaw_webui_v2::WebUiV2Capabilities;

use crate::operator_auth::OperatorWebuiConfigRouteState;

/// Authentication contract the Reborn binary supplies. The composition
/// layer is intentionally agnostic about WHERE bearer tokens come from
/// — env vars, the host's `SecretStore`, OIDC JWTs verified by the
/// caller — so the same `webui_v2_app` works for the CLI binary and
/// for any future ingress fronting the same routes.
///
/// Implementations return `Some(UserId)` on success and `None` to
/// reject. Concrete failure reasons stay inside the implementation
/// (the gateway emits a generic 401), per the
/// `docs/reborn/how-to-port-channel-to-reborn.md` Path A guidance that
/// auth evidence is host-owned and never leaks to clients.
#[async_trait::async_trait]
pub trait WebuiAuthenticator: Send + Sync + 'static {
    /// Authenticate a bearer and return the caller identity plus capabilities
    /// that apply to this exact token.
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication>;

    /// Whether bearer tokens accepted by this authenticator represent a
    /// single trusted operator. Operator-wide WebUI config routes mutate
    /// shared host configuration such as provider catalogs, secrets, active
    /// models, or Slack channel routes, so host composition only mounts them
    /// for authenticators that explicitly opt in.
    fn mounts_operator_webui_config_routes(&self) -> bool {
        #[allow(deprecated)]
        self.allows_operator_webui_config()
    }

    #[deprecated(
        since = "0.1.0",
        note = "Use `mounts_operator_webui_config_routes`; this asks about route availability, not per-token capability."
    )]
    fn allows_operator_webui_config(&self) -> bool {
        #[allow(deprecated)]
        self.allows_operator_llm_config()
    }

    #[deprecated(
        since = "0.1.0",
        note = "Use `mounts_operator_webui_config_routes`; this asks about route availability, not per-token capability."
    )]
    fn allows_operator_llm_config(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebuiAuthentication {
    pub user_id: UserId,
    pub capabilities: WebUiV2Capabilities,
}

impl WebuiAuthentication {
    pub fn new(user_id: UserId, capabilities: WebUiV2Capabilities) -> Self {
        Self {
            user_id,
            capabilities,
        }
    }

    pub fn user(user_id: UserId) -> Self {
        Self::new(user_id, WebUiV2Capabilities::default())
    }

    pub fn operator(user_id: UserId) -> Self {
        Self::new(
            user_id,
            WebUiV2Capabilities {
                operator_webui_config: true,
            },
        )
    }
}

#[derive(Clone)]
pub(crate) struct AuthLayerState {
    pub(crate) tenant_id: TenantId,
    pub(crate) default_agent_id: Option<AgentId>,
    pub(crate) default_project_id: Option<ProjectId>,
    pub(crate) authenticator: Arc<dyn WebuiAuthenticator>,
    pub(crate) operator_routes: OperatorWebuiConfigRouteState,
}

/// Resolve `Authorization: Bearer <token>` for any v2 route, OR the
/// `?token=…` query parameter only on the v2 SSE stream endpoint
/// (mirrors the browser's `EventSource` limitation — it cannot set
/// custom headers). On success, insert a [`WebUiAuthenticatedCaller`]
/// extension built from the host-installation tenant + the
/// authenticated user. On failure, return 401 before the v2 handler
/// runs.
pub(crate) async fn authenticate_request(
    State(state): State<AuthLayerState>,
    mut request: Request,
    next: Next,
) -> Response {
    let token = match extract_bearer_token(&request) {
        Some(token) => token,
        None => return unauthorized(),
    };

    let auth = match state.authenticator.authenticate(&token).await {
        Some(auth) => auth,
        None => return unauthorized(),
    };
    if state
        .operator_routes
        .requires_operator_webui_config(&request)
        && !auth.capabilities.operator_webui_config
    {
        return forbidden();
    }

    // Stamp the trusted agent/project from host installation config
    // onto every authenticated caller. The downstream facade builds
    // `ThreadScope` from `caller.agent_id` and 400s if it's missing,
    // so a binary that fails to thread agent_id through here would
    // authenticate users only to reject every v2 mutation/read. The
    // browser body cannot influence either of these identifiers — by
    // contract `WebuiServeConfig` is host-owned.
    let caller = WebUiAuthenticatedCaller::new(
        state.tenant_id.clone(),
        auth.user_id,
        state.default_agent_id.clone(),
        state.default_project_id.clone(),
    );
    request.extensions_mut().insert(caller);
    request.extensions_mut().insert(auth.capabilities);
    next.run(request).await
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, "Invalid or missing auth token").into_response()
}

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        "Operator WebUI configuration privileges required",
    )
        .into_response()
}

fn extract_bearer_token(request: &Request) -> Option<String> {
    if let Some(value) = request.headers().get(header::AUTHORIZATION)
        && let Ok(text) = value.to_str()
        // `text.get(..7)` returns `None` when 7 is past the end OR
        // lands inside a multi-byte UTF-8 sequence; both cases mean
        // the value cannot be `Bearer <token>`. A direct byte slice
        // would panic on a value whose first 7 bytes split a multi-byte
        // character, which is forbidden for user-supplied data.
        && let Some(prefix) = text.get(..7)
        && prefix.eq_ignore_ascii_case("Bearer ")
    {
        // Safe: `prefix.eq_ignore_ascii_case("Bearer ")` matched, so
        // the first 7 bytes are pure ASCII and byte 7 is a char
        // boundary.
        return Some(text[7..].to_string());
    }
    // `?token=` shim — only honored on the v2 SSE stream endpoint
    // because `EventSource` cannot set request headers. Mutations and
    // timeline reads stay bearer-only so a query-token leak in a
    // referer chain cannot authenticate a state change.
    //
    // **Operational warning:** the token-as-URL-parameter pattern is
    // a documented industry trade-off (SSE has no header-supplying
    // client primitive). The token value appears in the URL and will
    // therefore land in any HTTP access log, intermediate proxy log,
    // or analytics pipeline that sees the request line. Composition
    // emits `Referrer-Policy: no-referrer` on every response as
    // defense in depth, but operators MUST still scrub
    // `?token=<value>` from any log destination that retains URLs.
    // The acceptance check is narrowed to GET on the exact
    // `…/threads/{id}/events` path by `is_v2_sse_event_request` so
    // the leak surface is one route, not the whole gateway.
    if is_v2_sse_event_request(request) {
        return query_token(request);
    }
    None
}

/// Returns `true` if the request is `GET /api/webchat/v2/threads/{id}/events`.
/// The thread id must be a single non-empty path segment.
pub(crate) fn is_v2_sse_event_request(request: &Request) -> bool {
    if request.method() != Method::GET {
        return false;
    }
    let path = request.uri().path();
    let Some(rest) = path.strip_prefix("/api/webchat/v2/threads/") else {
        return false;
    };
    let Some(thread_id) = rest.strip_suffix("/events") else {
        return false;
    };
    !thread_id.is_empty() && !thread_id.contains('/')
}

fn query_token(request: &Request) -> Option<String> {
    let query = request.uri().query()?;
    url_query_value(query, "token")
}

fn url_query_value(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let candidate_key = parts.next()?;
        if candidate_key != key {
            continue;
        }
        let raw_value = parts.next().unwrap_or("");
        // Decode minimally: `+` → space, `%XX` → byte. Tokens are
        // almost always opaque ASCII so we accept the value as-is and
        // only handle the percent-decoded form when present. Empty or
        // whitespace-only values count as absent so a stray `?token=`
        // does not override a missing bearer header.
        let decoded = percent_decode(raw_value);
        let trimmed = decoded.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(trimmed.to_string());
    }
    None
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_value(bytes[i + 1]);
                let lo = hex_value(bytes[i + 2]);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi << 4) | lo);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
