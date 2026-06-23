use super::*;
use axum::body::Body;
use axum::http::{HeaderValue, Method};

fn fake_request(method: Method, path_and_query: &str) -> Request {
    Request::builder()
        .method(method)
        .uri(path_and_query)
        .body(Body::empty())
        .expect("request")
}

#[test]
fn v2_sse_event_request_recognized() {
    let req = fake_request(Method::GET, "/api/webchat/v2/threads/abc/events");
    assert!(is_v2_sse_event_request(&req));
}

#[test]
fn v2_sse_event_request_requires_get() {
    let req = fake_request(Method::POST, "/api/webchat/v2/threads/abc/events");
    assert!(!is_v2_sse_event_request(&req));
}

#[test]
fn v2_sse_event_request_requires_single_thread_segment() {
    assert!(!is_v2_sse_event_request(&fake_request(
        Method::GET,
        "/api/webchat/v2/threads//events"
    )));
    assert!(!is_v2_sse_event_request(&fake_request(
        Method::GET,
        "/api/webchat/v2/threads/abc/events/extra"
    )));
}

#[test]
fn v2_sse_event_request_rejects_other_v2_routes() {
    assert!(!is_v2_sse_event_request(&fake_request(
        Method::GET,
        "/api/webchat/v2/threads/abc/timeline"
    )));
    assert!(!is_v2_sse_event_request(&fake_request(
        Method::POST,
        "/api/webchat/v2/threads"
    )));
}

#[test]
fn query_token_extracts_token_param() {
    let req = fake_request(
        Method::GET,
        "/api/webchat/v2/threads/abc/events?token=abc123",
    );
    assert_eq!(query_token(&req).as_deref(), Some("abc123"));
}

#[test]
fn query_token_decodes_percent_escapes() {
    let req = fake_request(
        Method::GET,
        "/api/webchat/v2/threads/abc/events?token=a%2Bb%3Dc",
    );
    assert_eq!(query_token(&req).as_deref(), Some("a+b=c"));
}

#[test]
fn query_token_treats_empty_as_absent() {
    let req = fake_request(Method::GET, "/api/webchat/v2/threads/abc/events?token=");
    assert!(query_token(&req).is_none());
    let req2 = fake_request(
        Method::GET,
        "/api/webchat/v2/threads/abc/events?token=%20%20",
    );
    assert!(query_token(&req2).is_none());
}

#[test]
fn bearer_header_extraction_is_case_insensitive_on_prefix() {
    let mut req = fake_request(Method::POST, "/api/webchat/v2/threads");
    req.headers_mut().insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("bEaReR mytoken"),
    );
    assert_eq!(extract_bearer_token(&req).as_deref(), Some("mytoken"));
}

#[test]
fn extract_bearer_token_rejects_query_token_on_non_sse_paths() {
    // `?token=` is an EventSource-only escape hatch on the SSE
    // route. Mutations and reads MUST stay bearer-only — a future
    // regression that widens query-token acceptance to other
    // routes would silently downgrade auth on every state change
    // (no bearer header means an attacker only needs the URL).
    // This test pins extract_bearer_token's behavior on every
    // non-SSE shape we care about.
    for (method, path_and_query) in [
        (Method::POST, "/api/webchat/v2/threads?token=stealme"),
        (
            Method::POST,
            "/api/webchat/v2/threads/abc/messages?token=stealme",
        ),
        (
            Method::GET,
            "/api/webchat/v2/threads/abc/timeline?token=stealme",
        ),
        (
            Method::POST,
            "/api/webchat/v2/threads/abc/runs/r/cancel?token=stealme",
        ),
        (
            Method::POST,
            "/api/webchat/v2/threads/abc/runs/r/gates/g/resolve?token=stealme",
        ),
        // Even on the SSE path, the wrong METHOD must reject.
        (
            Method::POST,
            "/api/webchat/v2/threads/abc/events?token=stealme",
        ),
        // List threads shares the same path as create_thread but
        // is read-only; query-token still rejected because no
        // bearer header is present.
        (Method::GET, "/api/webchat/v2/threads?token=stealme"),
    ] {
        let req = fake_request(method.clone(), path_and_query);
        assert!(
            extract_bearer_token(&req).is_none(),
            "extract_bearer_token must NOT accept ?token= on {method} {path_and_query}",
        );
    }
}

#[test]
fn extract_bearer_token_accepts_query_token_only_on_sse_get() {
    // Companion to the rejection test: the one place `?token=` is
    // honored — GET on the SSE events route — must still work.
    let req = fake_request(Method::GET, "/api/webchat/v2/threads/abc/events?token=ok");
    assert_eq!(extract_bearer_token(&req).as_deref(), Some("ok"));
}
