import { isDesktopRuntime } from "../../../lib/api.js";
import { modelExecutionReadiness } from "../../../lib/model-readiness.js";

export function buildRuntimeContext({ gatewayStatus, activeThread }) {
  const turnCount = activeThread?.turn_count || 0;
  const connections = gatewayStatus?.total_connections;
  const engineLabel =
    gatewayStatus?.engine_v2_enabled === false ? "Engine v1" : "Engine v2";

  const context = {
    mode: "Auto-review",
    runtime: "Work locally",
    workspace: "ironclaw",
    model: gatewayStatus?.llm_model,
    backend: gatewayStatus?.llm_backend,
    threadLabel: activeThread?.title || "New thread",
    turnCountLabel: `${turnCount} ${turnCount === 1 ? "turn" : "turns"}`,
    engineLabel,
    connectionLabel:
      typeof connections === "number"
        ? `${connections} live ${
            connections === 1 ? "connection" : "connections"
          }`
        : null,
  };

  // Desktop-only model-execution readiness. The desktop runtime must verify
  // NEAR AI Cloud before it can run model work, and surfaces a sendBlocked
  // composer state until it does. On web this gating must NEVER apply: the
  // gateway owns model execution and `modelExecutionReadiness(undefined)`
  // returns `CHECKING_GATEWAY` with `sendBlocked: true`, which would falsely
  // disable the web composer before/without a gateway-status fetch. Gate the
  // whole block behind `isDesktopRuntime()` so the web contract is unchanged
  // (no `sendBlocked` key, composer never blocked here).
  if (isDesktopRuntime()) {
    const modelReadiness = modelExecutionReadiness(gatewayStatus);
    context.modelReadiness = modelReadiness;
    context.sendBlocked = modelReadiness.sendBlocked === true;
    context.sendBlockReason = modelReadiness.sendBlockReason || "";
  }

  return context;
}
