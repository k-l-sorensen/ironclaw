"""Reborn WebUI v2 tier-4 coverage: SSO login surface and session auth scope.

Drives the public `/auth/*` login surface mounted by `ironclaw-reborn serve`
when an OAuth provider is configured (see the `reborn_v2_sso_server` fixture):
the provider list, the login redirect that constructs the OIDC + PKCE
authorization request, one-time login-ticket exchange rejection, logout, and the
public-vs-bearer boundary (SSO routes are reachable without a session while the
v2 API stays bearer-gated, and the env-bearer caller remains an operator).

What this module does NOT do over the live binary: complete the OAuth
callback -> token-exchange -> session-mint -> multi-user-isolation flow. The
`serve` CLI builds `GoogleProvider`/`GitHubProvider` against the real,
hardcoded Google/GitHub token endpoints (no mock-endpoint override is exposed),
and Google login verifies the ID token against Google's JWKS — so a mock IdP
cannot mint an accepted session through the live listener. That full flow,
including two distinct OAuth users and per-user thread isolation, is covered
in-process by
`crates/ironclaw_reborn_webui_ingress/tests/signed_session_multi_user.rs`
(two users reach the facade as distinct callers; SSO sessions stay
non-operator). The placeholder test below documents that boundary.

Tracks nearai/ironclaw#4636.
"""

from urllib.parse import parse_qs, urlparse

import httpx
import pytest

from helpers import REBORN_V2_AUTH_TOKEN

_BEARER = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}


async def test_reborn_v2_auth_providers_lists_configured_provider(reborn_v2_sso_server):
    """The public providers route advertises the configured Google provider."""
    async with httpx.AsyncClient(base_url=reborn_v2_sso_server) as client:
        # Public route: reachable without a bearer (the whole point of login).
        response = await client.get("/auth/providers", timeout=15)
        response.raise_for_status()
        body = response.json()
        assert "google" in body.get("providers", []), body


async def test_reborn_v2_auth_login_builds_oidc_pkce_authorization_request(
    reborn_v2_sso_server,
):
    """`/auth/login/google` redirects to Google with the OIDC + PKCE parameters."""
    async with httpx.AsyncClient(
        base_url=reborn_v2_sso_server, follow_redirects=False
    ) as client:
        response = await client.get("/auth/login/google", timeout=15)
        assert response.status_code in (302, 303, 307), response.status_code

        location = response.headers["location"]
        parsed = urlparse(location)
        assert parsed.scheme == "https"
        assert parsed.netloc == "accounts.google.com", location
        query = parse_qs(parsed.query)
        assert query.get("response_type") == ["code"], query
        assert query.get("client_id") == ["e2e-google-client-id"], query
        # The redirect URI is the v2 callback on this listener.
        assert query["redirect_uri"][0].endswith("/auth/callback/google"), query
        assert "openid" in query["scope"][0], query
        # PKCE: an S256 challenge and CSRF state are minted server-side.
        assert query.get("code_challenge_method") == ["S256"], query
        assert query.get("code_challenge"), query
        assert query.get("state"), query


async def test_reborn_v2_session_exchange_rejects_unknown_ticket(reborn_v2_sso_server):
    """An unknown/expired login ticket cannot be exchanged for a bearer."""
    async with httpx.AsyncClient(base_url=reborn_v2_sso_server) as client:
        response = await client.post(
            "/auth/session/exchange",
            json={"ticket": "bogus-login-ticket"},
            timeout=15,
        )
        assert response.status_code == 401, response.text


async def test_reborn_v2_auth_callback_rejects_unknown_state(reborn_v2_sso_server):
    """A callback with an unrecognized CSRF state is rejected before any exchange.

    State is validated against the server-minted pending-login set first, so a
    forged/expired `state` is refused without ever contacting the IdP — no real
    OAuth provider is needed to exercise this guard.
    """
    async with httpx.AsyncClient(
        base_url=reborn_v2_sso_server, follow_redirects=False
    ) as client:
        response = await client.get(
            "/auth/callback/google",
            params={"state": "forged-unknown-state", "code": "irrelevant"},
            timeout=15,
        )
        # Rejected outright (4xx) or bounced to an error landing — never a 200
        # success and never a 5xx (the guard must fail cleanly, not crash).
        assert response.status_code != 200, response.status_code
        assert response.status_code < 500, response.status_code
        if response.status_code in (302, 303, 307):
            location = response.headers.get("location", "")
            assert "accounts.google.com" not in location, (
                f"a forged state must not proceed to the IdP: {location}"
            )


async def test_reborn_v2_logout_is_idempotent_without_session(reborn_v2_sso_server):
    """Logout without a session is a no-op success (no bearer to revoke)."""
    async with httpx.AsyncClient(base_url=reborn_v2_sso_server) as client:
        response = await client.post("/auth/logout", timeout=15)
        assert response.status_code == 204, response.text


async def test_reborn_v2_sso_login_surface_is_public_but_api_stays_bearer_gated(
    reborn_v2_sso_server,
):
    """SSO routes are public; the v2 API stays bearer-gated and env-bearer is operator.

    Configuring SSO mounts the public `/auth/*` surface without weakening the v2
    API auth: the env-bearer caller still authenticates as an operator
    (CompositeAuthenticator keeps the env token alongside SSO sessions), while an
    unauthenticated v2 API call is rejected.
    """
    async with httpx.AsyncClient(base_url=reborn_v2_sso_server) as client:
        # SSO login surface is reachable with no credentials.
        providers = await client.get("/auth/providers", timeout=15)
        assert providers.status_code == 200, providers.text

        # The v2 API rejects an unauthenticated caller.
        anon = await client.get("/api/webchat/v2/threads", timeout=15)
        assert anon.status_code == 401, anon.text

        # The env-bearer operator still works (and stays an operator) under SSO.
        session = await client.get(
            "/api/webchat/v2/session", headers=_BEARER, timeout=15
        )
        session.raise_for_status()
        assert session.json()["capabilities"]["operator_webui_config"] is True


@pytest.mark.skip(
    reason="The full OAuth callback -> token-exchange -> session -> multi-user "
    "isolation flow is not reachable over the live `ironclaw-reborn serve` "
    "binary: the serve CLI builds GoogleProvider/GitHubProvider against the real, "
    "hardcoded provider token endpoints (no mock-endpoint override is exposed) "
    "and Google verifies the ID token against Google's JWKS, so a mock IdP cannot "
    "mint an accepted session through the listener. This flow — two OAuth users "
    "reaching the facade as distinct callers, per-user thread isolation, and SSO "
    "sessions staying non-operator — is covered in-process by "
    "crates/ironclaw_reborn_webui_ingress/tests/signed_session_multi_user.rs."
)
async def test_reborn_v2_two_sso_users_have_isolated_threads(reborn_v2_sso_server):
    """Placeholder: two SSO users must not see each other's threads.

    When a mock-OAuth endpoint override is available to the `serve` binary, this
    would: log in user A and user B through the mock IdP, exchange both login
    tickets for bearers, create a thread as A, and assert B's
    `GET /api/webchat/v2/threads/{a_thread}/timeline` returns 404 (existence is
    hidden, not 403) while A still reads it.
    """
    raise AssertionError("unreachable: skipped above")
