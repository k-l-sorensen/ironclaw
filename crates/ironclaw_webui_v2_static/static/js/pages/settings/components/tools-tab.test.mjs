import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function sourceForTest(path, exportNames) {
  const source = readFileSync(new URL(path, import.meta.url), "utf8");
  const lines = [];
  let skippingImport = false;
  for (const line of source.split("\n")) {
    if (!skippingImport && line.startsWith("import ")) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ${exportNames.join(", ")} };`;
}

function html(strings, ...values) {
  return { strings: Array.from(strings), values };
}

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn);
    return;
  }
  if (!node || typeof node !== "object") return;
  fn(node);
  if (Array.isArray(node.values)) {
    for (const value of node.values) visit(value, fn);
  }
}

function collectTemplateText(root) {
  const text = [];
  visit(root, (node) => {
    if (Array.isArray(node.strings)) text.push(...node.strings);
  });
  return text.join("");
}

function findComponentNode(root, component) {
  let found = null;
  visit(root, (node) => {
    if (!found && Array.isArray(node.values) && node.values.includes(component)) {
      found = node;
    }
  });
  return found;
}

function componentProps(node, component) {
  const props = {};
  const start = node.values.indexOf(component);
  for (let index = start + 1; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9]*)=\s*$/)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}

function renderToolsModule({ tools = [] } = {}) {
  const saved = [];
  const context = {
    Badge: "Badge",
    Card: "Card",
    Icon: "Icon",
    globalThis: {},
    html,
    matchesSearch: (query, values) =>
      !query || values.some((value) => String(value || "").includes(query)),
    useT: () => (key) => key,
    useTools: () => ({
      tools,
      query: { isLoading: false, error: null },
      setPermission: () => {},
      savedTools: {},
    }),
  };
  vm.runInNewContext(
    sourceForTest("./tools-tab.js", ["ToolsTab", "AutoApproveCard", "Switch"]),
    context
  );
  return { exports: context.globalThis.__testExports, saved };
}

test("Tools tab renders global auto-approve control and saves the operator key", () => {
  const { exports, saved } = renderToolsModule();
  const rendered = exports.AutoApproveCard({
    settings: { "agent.auto_approve_tools": false },
    savedKeys: {},
    onSave: (key, value) => saved.push({ key, value }),
  });

  assert.match(collectTemplateText(exports.Switch({ checked: false, label: "x", onChange: () => {} })), /role="switch"/);
  const switchNode = findComponentNode(rendered, exports.Switch);
  assert.ok(switchNode, "expected auto-approve card to render a switch");

  componentProps(switchNode, exports.Switch).onChange(true);
  assert.deepEqual(saved, [{ key: "agent.auto_approve_tools", value: true }]);
});
