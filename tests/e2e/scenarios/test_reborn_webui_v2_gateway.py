"""Reborn WebUI v2: gateway hardening — security headers, CORS, static SPA, token shim.

Covers the transport-layer guarantees of the composed `webui_v2_app` over a real
listener: the static security headers on every response, the fail-closed CORS
allowlist, the embedded SPA bundle being served (including client-routed deep
links), and the `?token=` query shim being scoped strictly to the SSE events
route (every other route ignores it and falls through to bearer auth).

These are asserted in-process by `webui_v2_serve.rs`; this module re-checks them
over TCP, where header/middleware ordering and the static asset handler actually
run end to end.

Tracks the gateway-hardening smoke gap for nearai/ironclaw#4632.
"""

import httpx

from helpers import REBORN_V2_AUTH_TOKEN

_BEARER = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}


async def test_reborn_v2_responses_carry_static_security_headers(reborn_v2_server):
    """Every response carries the nosniff / DENY / CSP hardening headers."""
    async with httpx.AsyncClient(base_url=reborn_v2_server) as client:
        response = await client.get("/api/health", timeout=15)
        assert response.headers.get("x-content-type-options") == "nosniff", response.headers
        assert response.headers.get("x-frame-options") == "DENY", response.headers
        csp = response.headers.get("content-security-policy", "")
        assert "default-src 'self'" in csp, csp
        assert "frame-ancestors 'none'" in csp, csp


async def test_reborn_v2_cors_is_fail_closed_for_cross_origin(reborn_v2_server):
    """A cross-origin request is not granted an `Access-Control-Allow-Origin` echo.

    The serve binary configures an empty/self-only CORS allowlist, so an
    attacker origin must never be reflected back as allowed.
    """
    async with httpx.AsyncClient(base_url=reborn_v2_server) as client:
        response = await client.get(
            "/api/webchat/v2/session",
            headers={**_BEARER, "Origin": "http://evil.example"},
            timeout=15,
        )
        allow_origin = response.headers.get("access-control-allow-origin")
        assert allow_origin != "http://evil.example", response.headers
        assert allow_origin in (None, "null", reborn_v2_server), response.headers


async def test_reborn_v2_serves_spa_shell_and_client_routed_deep_link(reborn_v2_server):
    """The embedded SPA bundle is served at `/v2/` and on client-routed deep links."""
    async with httpx.AsyncClient(base_url=reborn_v2_server) as client:
        root = await client.get("/v2/", timeout=15)
        assert root.status_code == 200, root.status_code
        assert "text/html" in root.headers.get("content-type", ""), root.headers
        assert "<" in root.text and "html" in root.text.lower(), root.text[:200]

        # A client-routed deep link (no server route) still serves the SPA shell
        # so the browser router can take over.
        deep = await client.get("/v2/chat", timeout=15)
        assert deep.status_code == 200, deep.status_code
        assert "text/html" in deep.headers.get("content-type", ""), deep.headers


async def test_reborn_v2_token_shim_is_scoped_to_events_route(reborn_v2_server):
    """`?token=` authenticates only `GET /events`; other routes stay bearer-only.

    A stale referer link carrying `?token=` must not authenticate a state change
    or a transcript read.
    """
    async with httpx.AsyncClient(base_url=reborn_v2_server) as client:
        # Establish a real thread id with the bearer first.
        created = await client.post(
            "/api/webchat/v2/threads",
            headers=_BEARER,
            json={"client_action_id": "gateway-token-shim"},
            timeout=15,
        )
        thread_id = created.json()["thread"]["thread_id"]

    shim = f"?token={REBORN_V2_AUTH_TOKEN}"
    async with httpx.AsyncClient(base_url=reborn_v2_server) as anon:
        # Timeline read with the shim and no bearer must be rejected.
        timeline = await anon.get(
            f"/api/webchat/v2/threads/{thread_id}/timeline{shim}", timeout=15
        )
        assert timeline.status_code == 401, timeline.status_code

        # Mutation (send message) with the shim and no bearer must be rejected.
        send = await anon.post(
            f"/api/webchat/v2/threads/{thread_id}/messages{shim}",
            json={"client_action_id": "gateway-token-shim-send", "content": "hi"},
            timeout=15,
        )
        assert send.status_code == 401, send.status_code

        # The events route DOES accept the shim (200, real SSE stream opens).
        async with anon.stream(
            "GET",
            f"/api/webchat/v2/threads/{thread_id}/events{shim}",
            headers={"Accept": "text/event-stream"},
            timeout=15,
        ) as events:
            assert events.status_code == 200, events.status_code
