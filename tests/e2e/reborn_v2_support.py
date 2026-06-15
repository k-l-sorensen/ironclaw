"""Shared scaffolding for the Reborn WebUI v2 E2E scenarios.

The v2 smoke suite (`test_reborn_webui_v2_smoke.py`) and the tier-1/2/3/4
coverage modules all drive the same `ironclaw-reborn serve` surface over
`/api/webchat/v2/*`. The server/browser fixtures live in `conftest.py` so they
are shared (and module-scoped) across every v2 file; the pure helpers below are
plain functions imported wherever a scenario needs them.

Keeping these out of any single scenario file means a new tier module never has
to copy the thread/timeline plumbing or risk drifting from the canonical config
the fixture boots with.
"""

import asyncio
import os
import signal
import socket
import uuid
from pathlib import Path

import httpx

# The env-bearer user the local-dev config maps the WebUI caller to. Every v2
# thread the smoke caller creates is owned by this user id.
USER_ID = "reborn-v2-e2e-user"


def find_free_port() -> int:
    """Ask the OS for an available loopback port (startup hint; bind is retried)."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def read_log(path: Path, limit: int = 8192) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace")[-limit:]
    except OSError:
        return ""


def forward_coverage_env(env: dict[str, str]) -> None:
    for key, value in os.environ.items():
        if key.startswith(("CARGO_LLVM_COV", "LLVM_")) or key in {
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_INCREMENTAL",
        }:
            env[key] = value


async def stop_process(proc, *, sig=signal.SIGINT, timeout: float = 10) -> None:
    """Signal a subprocess and wait for exit without re-reading stdio pipes."""
    if proc.returncode is not None:
        return
    try:
        proc.send_signal(sig)
    except ProcessLookupError:
        return
    try:
        await asyncio.wait_for(proc.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        await asyncio.wait_for(proc.wait(), timeout=5)


def write_config_toml(path: Path, mock_llm_server: str, *, user_id: str = USER_ID) -> None:
    """Seed a sparse Reborn config that selects the mock LLM via the `openai` provider.

    The built-in `openai` provider speaks the OpenAI Chat Completions wire shape
    (`/v1/chat/completions`) that `mock_llm.py` serves. The `base_url` override
    points it at the mock; `api_key_env` names an env var the server fixture sets.
    Secrets stay out of the file — only the env-var NAME is referenced.
    """
    path.write_text(
        f"""api_version = "ironclaw.runtime/v1"

[boot]
profile = "local-dev"

[identity]
default_owner = "{user_id}"
tenant = "reborn-v2-e2e"
default_agent = "reborn-v2-e2e-agent"

[webui]
env_token_var = "IRONCLAW_REBORN_WEBUI_TOKEN"
env_user_id_var = "IRONCLAW_REBORN_WEBUI_USER_ID"

[llm.default]
provider_id = "openai"
model = "mock-model"
api_key_env = "MOCK_LLM_API_KEY"
base_url = "{mock_llm_server}/v1"
""",
        encoding="utf-8",
    )


def client_action_id() -> str:
    """Idempotency key accepted by `webui_inbound::parse_client_action_id`."""
    return str(uuid.uuid4())


async def create_thread(client: httpx.AsyncClient, base_url: str) -> str:
    response = await client.post(
        f"{base_url}/api/webchat/v2/threads",
        json={"client_action_id": client_action_id()},
        timeout=15,
    )
    response.raise_for_status()
    return response.json()["thread"]["thread_id"]


async def send_message(
    client: httpx.AsyncClient, base_url: str, thread_id: str, content: str
) -> httpx.Response:
    response = await client.post(
        f"{base_url}/api/webchat/v2/threads/{thread_id}/messages",
        json={"client_action_id": client_action_id(), "content": content},
        timeout=30,
    )
    assert response.status_code in (200, 202), response.text
    return response


def finalized_assistant_messages(timeline: dict) -> list[dict]:
    """Return the finalized, non-empty assistant messages from a timeline body."""
    return [
        message
        for message in timeline.get("messages", [])
        if message.get("kind") == "assistant"
        and message.get("status") == "finalized"
        and (message.get("content") or "").strip()
    ]


async def get_timeline(client: httpx.AsyncClient, base_url: str, thread_id: str) -> dict:
    response = await client.get(
        f"{base_url}/api/webchat/v2/threads/{thread_id}/timeline", timeout=15
    )
    response.raise_for_status()
    return response.json()


async def wait_for_assistant_message(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
) -> dict:
    """Poll the timeline until a finalized assistant message appears."""
    last_timeline: dict = {}
    for _ in range(int(timeout * 2)):
        last_timeline = await get_timeline(client, base_url, thread_id)
        finalized = finalized_assistant_messages(last_timeline)
        if finalized:
            return finalized[-1]
        await asyncio.sleep(0.5)

    raise AssertionError(
        f"Timed out waiting for a finalized assistant message in thread {thread_id}. "
        f"Last timeline: {last_timeline}"
    )
