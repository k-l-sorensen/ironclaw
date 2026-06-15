"""Reborn WebUI v2 tier-1 coverage: tool turns, run cancellation, and gates.

Companion to `test_reborn_webui_v2_smoke.py`, which proves text turns over the
real `ironclaw-reborn serve` binary. This module exercises the capability path
the smoke suite intentionally skipped: a tool-triggering turn that dispatches a
builtin capability, mid-run cancellation, and the approval/auth gate
round-trips.

These run against the same module-scoped `reborn_v2_server` fixture (conftest).
The `local-dev` boot profile wires the local-dev capability policy, so
`builtin.echo` is model-visible and completes without a gate, while
`builtin.shell` (spawn/execute effects) is routed through the `ask_writes`
approval gate — both deterministic with the canned mock LLM.

Tracks nearai/ironclaw#4633.
"""

import asyncio
import json

import aiohttp
import httpx
import pytest

from helpers import REBORN_V2_AUTH_TOKEN
from reborn_v2_support import (
    client_action_id,
    create_thread,
    finalized_assistant_messages,
    get_timeline,
    send_message,
    wait_for_assistant_message,
)

_BEARER = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}


def _events_url(base_url: str, thread_id: str) -> str:
    # The browser EventSource cannot set an Authorization header, so the events
    # route accepts the `?token=` shim (the only v2 route that does).
    return (
        f"{base_url}/api/webchat/v2/threads/{thread_id}/events"
        f"?token={REBORN_V2_AUTH_TOKEN}"
    )


async def _read_sse_frames(response, *, match, timeout: float = 45.0):
    """Read an open aiohttp SSE response until a parsed `data:` frame matches.

    `match(event_type, payload)` is called for every JSON data frame; the first
    payload it accepts is returned. Raises AssertionError on stream close or
    timeout so failures point at the missing event rather than hanging.
    """
    async with asyncio.timeout(timeout):
        while True:
            raw = await response.content.readline()
            assert raw, "SSE stream closed before a matching frame arrived"
            line = raw.decode("utf-8", errors="replace").strip()
            if not line.startswith("data:"):
                continue
            data = line[len("data:") :].strip()
            if not data:
                continue
            try:
                payload = json.loads(data)
            except json.JSONDecodeError:
                continue
            if match(payload.get("type"), payload):
                return payload


async def test_reborn_v2_tool_turn_dispatches_and_references_result(reborn_v2_server):
    """A tool-triggering prompt dispatches a builtin and records a tool result.

    The canned mock returns a `builtin.echo` tool call for an "echo ..." prompt;
    the agent loop dispatches it, the timeline gains a `tool_result_reference`
    message, and a finalized assistant message lands after the tool result is
    fed back to the model.
    """
    async with httpx.AsyncClient(headers=_BEARER) as client:
        thread_id = await create_thread(client, reborn_v2_server)
        await send_message(client, reborn_v2_server, thread_id, "echo hello world")

        # The finalized assistant reply only lands after the tool result is fed
        # back, so waiting for it also proves the tool turn did not stall.
        await wait_for_assistant_message(client, reborn_v2_server, thread_id, timeout=60)

        timeline = await get_timeline(client, reborn_v2_server, thread_id)
        messages = timeline["messages"]

        tool_results = [m for m in messages if m.get("kind") == "tool_result_reference"]
        assert tool_results, (
            "tool turn must record a tool_result_reference message in the timeline; "
            f"got kinds {[m.get('kind') for m in messages]}"
        )
        # The tool-result message carries the opaque host-issued result ref.
        assert any(m.get("tool_result_ref") for m in tool_results), (
            f"tool_result_reference message must expose tool_result_ref: {tool_results}"
        )

        assert finalized_assistant_messages(timeline), (
            "tool turn must finalize an assistant message after the tool result"
        )


async def test_reborn_v2_cancel_active_run(reborn_v2_server):
    """Cancelling a parked (approval-blocked) run transitions it out of running.

    A `builtin.shell` turn parks at an approval gate (a non-terminal, genuinely
    active run), giving a deterministic window to exercise
    `POST .../runs/{run_id}/cancel` without racing a fast mock completion. The
    cancel must be accepted against a live run (`already_terminal == False`) and
    the stream must report the run cancelled.
    """
    async with httpx.AsyncClient(headers=_BEARER) as client:
        thread_id = await create_thread(client, reborn_v2_server)

    async with aiohttp.ClientSession(
        timeout=aiohttp.ClientTimeout(total=60, sock_read=60)
    ) as session:
        async with session.get(
            _events_url(reborn_v2_server, thread_id),
            headers={"Accept": "text/event-stream"},
        ) as stream:
            assert stream.status == 200, stream.status

            async with httpx.AsyncClient(headers=_BEARER) as client:
                await send_message(
                    client, reborn_v2_server, thread_id, "make shell approval"
                )

            gate = await _read_sse_frames(
                stream, match=lambda t, _p: t == "gate", timeout=45
            )
            run_id = gate["prompt"]["turn_run_id"]

            async with httpx.AsyncClient(headers=_BEARER) as client:
                cancelled = await client.post(
                    f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}"
                    f"/runs/{run_id}/cancel",
                    json={"client_action_id": client_action_id(), "reason": "user_requested"},
                    timeout=15,
                )
            assert cancelled.status_code == 200, cancelled.text
            body = cancelled.json()
            assert body["run_id"] == run_id, body
            assert body["already_terminal"] is False, (
                f"cancel must act on a live parked run, got {body}"
            )

            # The stream reports the run reaching a cancelled state — either a
            # typed `cancelled` frame or a projection update whose run_status
            # carries the cancelled lifecycle status.
            def _is_cancelled(event_type, payload):
                if event_type == "cancelled":
                    return True
                if event_type in ("projection_snapshot", "projection_update"):
                    for item in payload.get("state", {}).get("items", []):
                        status = item.get("run_status", {}).get("status")
                        if status in ("cancelled", "cancel_requested"):
                            return True
                return False

            await _read_sse_frames(stream, match=_is_cancelled, timeout=30)


async def test_reborn_v2_approval_gate_resume(reborn_v2_server):
    """An approval-gated tool blocks, then resumes after an `approved` resolution."""
    async with httpx.AsyncClient(headers=_BEARER) as client:
        thread_id = await create_thread(client, reborn_v2_server)

    async with aiohttp.ClientSession(
        timeout=aiohttp.ClientTimeout(total=90, sock_read=90)
    ) as session:
        async with session.get(
            _events_url(reborn_v2_server, thread_id),
            headers={"Accept": "text/event-stream"},
        ) as stream:
            assert stream.status == 200, stream.status

            async with httpx.AsyncClient(headers=_BEARER) as client:
                await send_message(
                    client, reborn_v2_server, thread_id, "make shell approval"
                )

            gate = await _read_sse_frames(
                stream, match=lambda t, _p: t == "gate", timeout=45
            )
            prompt = gate["prompt"]
            run_id = prompt["turn_run_id"]
            gate_ref = prompt["gate_ref"]
            # The approval context names the capability under review.
            assert prompt.get("approval_context", {}).get("tool_name"), prompt

            async with httpx.AsyncClient(headers=_BEARER) as client:
                resolved = await client.post(
                    f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}"
                    f"/runs/{run_id}/gates/{gate_ref}/resolve",
                    json={"client_action_id": client_action_id(), "resolution": "approved", "always": False},
                    timeout=15,
                )
            assert resolved.status_code == 200, resolved.text
            assert resolved.json()["outcome"] == "resumed", resolved.text

        # After approval the run resumes and finalizes an assistant reply.
        async with httpx.AsyncClient(headers=_BEARER) as client:
            await wait_for_assistant_message(
                client, reborn_v2_server, thread_id, timeout=60
            )


async def test_reborn_v2_approval_gate_deny_cancels_run(reborn_v2_server):
    """Denying an approval gate cancels the parked run instead of resuming it."""
    async with httpx.AsyncClient(headers=_BEARER) as client:
        thread_id = await create_thread(client, reborn_v2_server)

    async with aiohttp.ClientSession(
        timeout=aiohttp.ClientTimeout(total=60, sock_read=60)
    ) as session:
        async with session.get(
            _events_url(reborn_v2_server, thread_id),
            headers={"Accept": "text/event-stream"},
        ) as stream:
            assert stream.status == 200, stream.status

            async with httpx.AsyncClient(headers=_BEARER) as client:
                await send_message(
                    client, reborn_v2_server, thread_id, "make shell approval"
                )

            gate = await _read_sse_frames(
                stream, match=lambda t, _p: t == "gate", timeout=45
            )
            prompt = gate["prompt"]
            run_id = prompt["turn_run_id"]
            gate_ref = prompt["gate_ref"]

            async with httpx.AsyncClient(headers=_BEARER) as client:
                resolved = await client.post(
                    f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}"
                    f"/runs/{run_id}/gates/{gate_ref}/resolve",
                    json={"client_action_id": client_action_id(), "resolution": "denied"},
                    timeout=15,
                )
            assert resolved.status_code == 200, resolved.text
            assert resolved.json()["outcome"] == "cancelled", resolved.text


@pytest.mark.skip(
    reason="No turn-blocking auth_required gate is deterministically reachable on "
    "the local-dev capability surface: every granted builtin completes without a "
    "use_secret effect, and the mock LLM's gmail install->auth script targets the "
    "v1 engine and does not surface auth_required on the Reborn v2 capability "
    "surface. A real auth gate needs a credential-requiring extension wired into "
    "local-dev (or a builtin that declares use_secret); tracked for follow-up. "
    "The auth resolve_gate contract (credential_provided / cancelled) is covered "
    "in-process by crates/ironclaw_reborn_composition/tests/manual_tokens.rs."
)
async def test_reborn_v2_auth_gate_manual_token_resume(reborn_v2_server):
    """Auth-gate round-trip: capability needs a credential -> manual token -> resume.

    Placeholder for the credential-gate path. When a credential-requiring
    capability is reachable in local-dev, the flow is:

    1. Send a prompt that dispatches the credential-requiring capability.
    2. Read the `auth_required` SSE frame; capture `prompt.auth_request_ref`.
    3. POST the secret to `/api/reborn/product-auth/manual-token/submit` to get a
       `credential_ref`.
    4. POST `resolve_gate` with `{"resolution": "credential_provided",
       "credential_ref": ...}` and assert the run resumes.
    """
    raise AssertionError("unreachable: skipped above")
