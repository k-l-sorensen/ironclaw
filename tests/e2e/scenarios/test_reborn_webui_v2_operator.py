"""Reborn WebUI v2: operator status/diagnostics surface and outbound preferences.

Covers the operator read projections mounted for the env-bearer operator caller
(`/operator/*`) and the outbound delivery preference/target reads
(`/outbound/*`). These are projection reads that return a stable, sanitized
shape even when the underlying backend is "unavailable"/"unsupported" in
local-dev — the contract under test is the projection envelope, not a wired
platform backend.

Tracks the operator/outbound smoke gap for nearai/ironclaw#4632.
"""

import httpx

from helpers import REBORN_V2_AUTH_TOKEN

_BEARER = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}


def _client(base_url: str) -> httpx.AsyncClient:
    return httpx.AsyncClient(base_url=base_url, headers=_BEARER, timeout=15)


async def test_reborn_v2_operator_status_projection(reborn_v2_server):
    """`/operator/status` returns an availability-tagged readiness projection."""
    async with _client(reborn_v2_server) as client:
        body = (await client.get("/api/webchat/v2/operator/status")).json()
        assert body["area"] == "status", body
        assert body["status"] in ("available", "unavailable"), body
        status = body["operator_status"]
        assert status.get("overall"), status
        assert isinstance(status.get("checks"), list), status
        for check in status["checks"]:
            for field in ("id", "status", "severity", "summary"):
                assert field in check, (field, check)


async def test_reborn_v2_operator_diagnostics_projection(reborn_v2_server):
    """`/operator/diagnostics` returns an availability-tagged diagnostics projection.

    In local-dev the operator config backend is not wired, so this reports
    `unavailable` with a typed diagnostic rather than erroring — the sanitized
    shape is the contract.
    """
    async with _client(reborn_v2_server) as client:
        body = (await client.get("/api/webchat/v2/operator/diagnostics")).json()
        assert body["area"] == "diagnostics", body
        assert body["status"] in ("available", "unavailable"), body
        assert isinstance(body.get("diagnostics"), list), body


async def test_reborn_v2_operator_setup_reports_active_provider(reborn_v2_server):
    """`/operator/setup` projects the active provider/model and setup steps."""
    async with _client(reborn_v2_server) as client:
        body = (await client.get("/api/webchat/v2/operator/setup")).json()
        assert body["area"] == "setup", body
        # The config.toml selects the mock-backed `openai` provider.
        assert body.get("active_provider_id") == "openai", body
        assert isinstance(body.get("steps"), list) and body["steps"], body
        for step in body["steps"]:
            for field in ("name", "status", "message"):
                assert field in step, (field, step)


async def test_reborn_v2_operator_logs_projection(reborn_v2_server):
    """`/operator/logs` returns the log-query projection with a source and entries."""
    async with _client(reborn_v2_server) as client:
        body = (await client.get("/api/webchat/v2/operator/logs")).json()
        assert body["area"] == "logs", body
        if body["status"] == "available":
            logs = body["logs"]
            assert logs.get("source"), logs
            assert isinstance(logs.get("entries"), list), logs


async def test_reborn_v2_operator_service_lifecycle_projection(reborn_v2_server):
    """`/operator/service` returns a lifecycle projection (unsupported in local-dev)."""
    async with _client(reborn_v2_server) as client:
        response = await client.post(
            "/api/webchat/v2/operator/service", json={"action": "status"}
        )
        assert response.status_code == 200, response.text
        body = response.json()
        assert body["area"] == "service_lifecycle", body
        assert body["service_lifecycle"]["action"] == "status", body
        assert body["service_lifecycle"]["state"], body


async def test_reborn_v2_outbound_preferences_and_targets(reborn_v2_server):
    """The outbound preference + delivery-target read projections return their shapes."""
    async with _client(reborn_v2_server) as client:
        prefs = (await client.get("/api/webchat/v2/outbound/preferences")).json()
        assert "final_reply_target_status" in prefs, prefs
        assert "default_modality" in prefs, prefs

        targets = (await client.get("/api/webchat/v2/outbound/targets")).json()
        assert isinstance(targets.get("targets"), list), targets
