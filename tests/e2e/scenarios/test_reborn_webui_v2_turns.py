"""Reborn WebUI v2: turn idempotency and inline attachment landing.

Covers two chat-turn behaviours the smoke/tool suites don't: client-action-id
idempotency (a replayed create-thread/send must not duplicate state) and inline
base64 attachment landing (an uploaded file is decoded, budgeted, stored, and
surfaced as a structured attachment ref on the user timeline message).

Tracks the turn-idempotency / attachment smoke gap for nearai/ironclaw#4632.
"""

import base64

import httpx

from helpers import REBORN_V2_AUTH_TOKEN
from reborn_v2_support import create_thread, get_timeline

_BEARER = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}


def _client(base_url: str) -> httpx.AsyncClient:
    return httpx.AsyncClient(base_url=base_url, headers=_BEARER, timeout=30)


async def test_reborn_v2_create_thread_is_idempotent_on_client_action_id(reborn_v2_server):
    """Replaying create-thread with the same client_action_id returns one thread."""
    async with _client(reborn_v2_server) as client:
        cid = "idempotent-create-thread"
        first = await client.post(
            "/api/webchat/v2/threads", json={"client_action_id": cid}
        )
        second = await client.post(
            "/api/webchat/v2/threads", json={"client_action_id": cid}
        )
        assert first.status_code == 200, first.text
        assert second.status_code == 200, second.text
        assert (
            first.json()["thread"]["thread_id"] == second.json()["thread"]["thread_id"]
        ), "a replayed client_action_id must not mint a second thread"


async def test_reborn_v2_duplicate_send_does_not_double_post(reborn_v2_server):
    """A replayed send (same client_action_id) does not append a second user message."""
    async with _client(reborn_v2_server) as client:
        thread_id = await create_thread(client, reborn_v2_server)
        cid = "idempotent-send"
        content = "echo idempotency check"

        first = await client.post(
            f"/api/webchat/v2/threads/{thread_id}/messages",
            json={"client_action_id": cid, "content": content},
        )
        assert first.status_code in (200, 202), first.text

        second = await client.post(
            f"/api/webchat/v2/threads/{thread_id}/messages",
            json={"client_action_id": cid, "content": content},
        )
        # The replay is rejected as a duplicate/conflict rather than accepted as
        # a fresh turn.
        assert second.status_code in (200, 202, 409), second.text

        timeline = await get_timeline(client, reborn_v2_server, thread_id)
        user_messages = [
            m
            for m in timeline["messages"]
            if m.get("kind") == "user" and (m.get("content") or "") == content
        ]
        assert len(user_messages) == 1, (
            f"a replayed client_action_id must land exactly one user message, "
            f"got {len(user_messages)}"
        )


async def test_reborn_v2_inline_attachment_lands_on_timeline(reborn_v2_server):
    """An inline base64 attachment is decoded, stored, and projected on the message."""
    async with _client(reborn_v2_server) as client:
        thread_id = await create_thread(client, reborn_v2_server)
        payload = b"hello attachment from e2e"
        response = await client.post(
            f"/api/webchat/v2/threads/{thread_id}/messages",
            json={
                "client_action_id": "attachment-turn",
                "content": "here is a file",
                "attachments": [
                    {
                        "mime_type": "text/plain",
                        "filename": "note.txt",
                        "data_base64": base64.b64encode(payload).decode(),
                    }
                ],
            },
        )
        assert response.status_code in (200, 202), response.text

        timeline = await get_timeline(client, reborn_v2_server, thread_id)
        user_messages = [m for m in timeline["messages"] if m.get("kind") == "user"]
        assert user_messages, "the attachment turn must record a user message"
        attachments = user_messages[-1].get("attachments") or []
        assert len(attachments) == 1, f"expected one attachment ref, got {attachments}"
        attachment = attachments[0]
        assert attachment["mime_type"] == "text/plain", attachment
        assert attachment["filename"] == "note.txt", attachment
        assert attachment.get("size_bytes") == len(payload), attachment
