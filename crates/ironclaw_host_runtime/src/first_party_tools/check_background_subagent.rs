use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode};

use crate::FirstPartyCapabilityError;

use super::input_error;

pub const CHECK_BACKGROUND_SUBAGENT_CAPABILITY_ID: &str = "builtin.check_background_subagent";

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    super::first_party_capability_manifest(
        CHECK_BACKGROUND_SUBAGENT_CAPABILITY_ID,
        "Poll a previously spawned background subagent run for its result",
        // Read-only inspection of an already-authorized child run: no spawn,
        // no filesystem, no network. `Allow` because a parent that already
        // holds the spawn grant should be able to check on its own children
        // without a fresh approval prompt.
        vec![EffectKind::DispatchCapability],
        PermissionMode::Allow,
        super::resource_profile(),
    )
}

/// Poll handler for background subagent completion.
///
/// Issue #4084 Gap 1 wakes an idle parent by delivering the background
/// subagent's result as an inbound message the moment the child terminates
/// (see `SubagentCompletionObserver::notify_parent_of_background_completion`).
/// That closes the stranded-result bug for the common case.
///
/// Gap 2's mid-turn pull model — reading the populated `LoopResultRef` back
/// out of the capability result store keyed by `child_run_id` — needs a
/// non-consuming result reader plus a durable parent/child index that
/// survives gate/goal-record deletion. None of that infrastructure exists
/// yet, and the staged-result APIs that do exist consume the result on read,
/// which would violate the "LLM data is never deleted" invariant.
///
/// Rather than ship a racey half-read, this capability is wired through the
/// full first-party dispatch pipeline (manifest, schema, handler, registry)
/// and returns a structured, machine-readable response that tells the model
/// where the result is actually delivered. The read-side index is tracked as
/// follow-up work; the dispatch seam is already in place for it.
pub(super) fn dispatch(
    input: &serde_json::Value,
) -> Result<serde_json::Value, FirstPartyCapabilityError> {
    let child_run_id = input
        .get("child_run_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(input_error)?;

    Ok(serde_json::json!({
        "child_run_id": child_run_id,
        "output_available": false,
        "status": "poll_not_supported",
        "delivery": "inbound_notification",
        "detail": "Background subagent results are delivered to this thread as an \
                   inbound message when the child terminates; mid-turn polling is not \
                   yet supported.",
    }))
}
