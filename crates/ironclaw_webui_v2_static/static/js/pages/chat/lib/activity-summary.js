export function summarizeActivity(activity) {
  let reasoning = 0;
  const tools = {
    total: 0,
    files: 0,
    searches: 0,
    commands: 0,
    other: 0,
  };
  let failed = 0;
  let running = 0;

  for (const item of activity) {
    if (item.role === "thinking") reasoning += 1;
    if (item.role === "tool_activity") {
      const summary = summarizeToolItems([item]);
      mergeToolSummary(tools, summary);
      failed += summary.failed;
      running += summary.running;
    }
    if (hasToolCalls(item)) {
      const summary = summarizeToolItems(item.toolCalls);
      mergeToolSummary(tools, summary);
      failed += summary.failed;
      running += summary.running;
    }
  }

  const parts = [];
  const toolLabel = toolActivityLabel(tools);
  if (toolLabel) parts.push(toolLabel);
  if (reasoning) parts.push(reasoning === 1 ? "reasoning note" : `${reasoning} reasoning notes`);

  let label = parts.length ? parts.join(" · ") : "Activity";
  if (failed) label = `${failed} ${failed === 1 ? "tool failed" : "tools failed"}`;
  else if (running && tools.total)
    label = `Checking ${tools.total} tool ${tools.total === 1 ? "step" : "steps"}...`;
  else if (running) label = "Working...";

  return {
    hasError: failed > 0,
    isComplete: tools.total > 0 && failed === 0 && running === 0,
    isRunning: running > 0,
    toolCount: tools.total,
    label,
  };
}

function summarizeToolItems(items) {
  let failed = 0;
  let running = 0;
  let files = 0;
  let searches = 0;
  let commands = 0;
  let other = 0;

  for (const item of items) {
    if (item.toolStatus === "error") failed += 1;
    if (item.toolStatus === "running") running += 1;
    const kind = toolKind(item);
    if (kind === "file") files += 1;
    else if (kind === "search") searches += 1;
    else if (kind === "command") commands += 1;
    else other += 1;
  }

  return { total: items.length, files, searches, commands, other, failed, running };
}

function mergeToolSummary(target, summary) {
  target.total += summary.total;
  target.files += summary.files;
  target.searches += summary.searches;
  target.commands += summary.commands;
  target.other += summary.other;
}

function toolActivityLabel(summary) {
  if (!summary.total) return "";
  const activeKinds = [summary.files, summary.searches, summary.commands, summary.other].filter(
    Boolean,
  ).length;
  if (activeKinds > 1) {
    return `Checked ${summary.total} tool ${summary.total === 1 ? "step" : "steps"}`;
  }
  if (summary.files) return `Read ${summary.files} ${summary.files === 1 ? "file" : "files"}`;
  if (summary.searches)
    return `Searched ${summary.searches} ${summary.searches === 1 ? "time" : "times"}`;
  if (summary.commands)
    return `Ran ${summary.commands} ${summary.commands === 1 ? "command" : "commands"}`;
  return `Used ${summary.other} ${summary.other === 1 ? "tool" : "tools"}`;
}

function toolKind(item) {
  const name = String(item?.toolName || "").toLowerCase();
  if (/(grep|search|find|lookup|query)/.test(name)) return "search";
  if (/(bash|shell|exec|run|command|terminal|spawn|process)/.test(name)) return "command";
  if (/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(name)) {
    return "file";
  }
  return "other";
}

function hasToolCalls(item) {
  return item.toolCalls && item.toolCalls.length > 0;
}
