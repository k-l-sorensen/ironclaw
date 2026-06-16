import { React } from "../../../lib/html.js";

// Gateway restart was a v1 capability (POSTed `/restart` as a chat command via
// `/api/chat/events`). v2 has no equivalent system endpoint yet, so there is no
// path the gateway can actually prove it can restart itself. Per the "No fake
// readiness" design law this hook must NOT hand the banner a working restart
// affordance: `canRestart` stays false until a real v2 admin/system endpoint
// lands, and `restart()` only surfaces an honest "unavailable" reason instead of
// hitting a v1 path that no longer exists.
//
// `gatewayStatus.restart_enabled` reports whether the host *could* host a restart
// at all (true only inside the Tauri desktop shell). It is a necessary but not
// sufficient condition: even when it is true, the v2 endpoint is still missing,
// so the affordance remains gated off. The flag is threaded through so the day
// the endpoint lands the only change needed is dropping the
// `ENDPOINT_AVAILABLE` guard below.
const ENDPOINT_AVAILABLE = false;

export function useGatewayRestart({ gatewayStatus } = {}) {
  const [confirmOpen, setConfirmOpen] = React.useState(false);
  const [isRestarting, setIsRestarting] = React.useState(false);
  const [progress, setProgress] = React.useState(null);
  const [error, setError] = React.useState(null);

  const hostSupportsRestart = Boolean(gatewayStatus?.restart_enabled);
  // Only true when the host can host a restart AND a v2 endpoint exists to run
  // it. Today the endpoint is absent, so this is always false and the banner
  // renders as an informational "restart required" notice with no action.
  const canRestart = hostSupportsRestart && ENDPOINT_AVAILABLE;

  // Honest, muted explanatory line shown when no restart affordance is offered.
  // Reuses an existing i18n key (the en.js pack is locked at 800 keys): the
  // dialog description already states the requirement plainly ("Restart the
  // gateway process to apply pending changes.") without implying the app can do
  // it automatically.
  const unavailableReason = canRestart ? null : "restart.description";

  const openConfirm = React.useCallback(() => {
    if (!canRestart) return;
    setError(null);
    setConfirmOpen(true);
  }, [canRestart]);

  const closeConfirm = React.useCallback(() => {
    if (isRestarting) return;
    setConfirmOpen(false);
  }, [isRestarting]);

  const restart = React.useCallback(async () => {
    // The action is gated behind `canRestart` in the UI, so this is a defensive
    // no-op while no v2 endpoint exists — never fabricate progress or an error
    // from a capability the gateway cannot prove. When the real v2 system
    // endpoint lands, set isRestarting/progress here, call it, and clear or set
    // `error` from the actual result.
    if (!canRestart) return;
    setError(null);
    setIsRestarting(true);
    setProgress("settings.restartStarting");
  }, [canRestart]);

  const confirmRestart = React.useCallback(async () => {
    setConfirmOpen(false);
    await restart();
  }, [restart]);

  return {
    // Honest availability: false today (no v2 endpoint), so the banner shows no
    // restart affordance and explains why via `unavailableReason`.
    canRestart,
    unavailableReason,
    // Confirm-modal state machine (owned here so the banner is a pure consumer).
    confirmOpen,
    openConfirm,
    closeConfirm,
    confirmRestart,
    // Action + in-flight/progress/error state.
    restart,
    isRestarting,
    progress,
    error,
  };
}
