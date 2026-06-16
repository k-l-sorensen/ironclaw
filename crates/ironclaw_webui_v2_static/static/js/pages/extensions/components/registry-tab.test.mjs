import assert from "node:assert/strict";
import test from "node:test";

import {
  ACCEPTANCE_WORKFLOWS,
  CORE_CONNECTIONS,
  acceptanceWorkflowStatus,
  coreConnectionButtonState,
  coreConnectionKindLabel,
  workflowCatalogStatus,
} from "./registry-tab.js";
import { connectorIconKind } from "./extension-card.js";

const ISSUE_4775_USE_CASES = [
  {
    name: "Daily news digest",
    surfaces: ["telegram", "web-http", "routines"],
  },
  {
    name: "Calendar prep assistant",
    surfaces: ["gmail", "google-calendar", "google-drive", "web-http", "routines"],
  },
  {
    name: "Deployment health watcher",
    surfaces: ["slack", "web-http", "routines"],
  },
  {
    name: "Competitor release tracker",
    surfaces: ["gmail", "github", "routines"],
  },
  {
    name: "AMA in Slack",
    surfaces: ["slack", "google-drive"],
  },
  {
    name: "CRM inbound tracker",
    surfaces: ["gmail", "google-sheets", "routines"],
  },
  {
    name: "Slack to Sheet bug logger",
    surfaces: ["slack", "google-sheets", "routines"],
  },
  {
    name: "HN keyword monitor",
    surfaces: ["slack", "web-http", "routines"],
  },
];

function coreEntry(id) {
  const entry = CORE_CONNECTIONS.find((connection) => connection.id === id);
  assert.ok(entry, `missing core connection fixture ${id}`);
  return entry;
}

test("core connection fallbacks expose the expected catalog refs only", () => {
  assert.deepEqual(
    CORE_CONNECTIONS.filter((entry) => entry.package_ref).map((entry) => entry.package_ref.id),
    [
      "tools/gmail",
      "tools/google_calendar",
      "tools/google_drive",
      "tools/google_sheets",
      "mcp-servers/notion",
      "channels/slack",
      "channels/telegram",
      "tools/github",
    ]
  );
  assert.equal(CORE_CONNECTIONS.find((entry) => entry.id === "workspace")?.package_ref, null);
});

test("core connection fallbacks cover the issue 4775 QA acceptance surface", () => {
  const surfaced = new Set(CORE_CONNECTIONS.map((entry) => entry.id));

  for (const useCase of ISSUE_4775_USE_CASES) {
    for (const surface of useCase.surfaces) {
      assert.ok(surfaced.has(surface), `${useCase.name} missing ${surface}`);
    }
  }
});

test("acceptance workflows map every issue 4775 scenario to connector surfaces", () => {
  const workflowsByTitle = Object.fromEntries(
    ACCEPTANCE_WORKFLOWS.map((workflow) => [workflow.title, workflow])
  );

  for (const useCase of ISSUE_4775_USE_CASES) {
    const workflow = workflowsByTitle[useCase.name];
    assert.ok(workflow, `${useCase.name} is not exposed as a workflow recipe`);
    assert.deepEqual(workflow.surfaces, useCase.surfaces);
    assert.ok(workflow.prompt.length >= 80, `${useCase.name} needs a useful chat draft`);
  }
});

test("acceptance workflows only reference surfaced connection ids", () => {
  const surfaced = new Set(CORE_CONNECTIONS.map((entry) => entry.id));

  for (const workflow of ACCEPTANCE_WORKFLOWS) {
    for (const surface of workflow.surfaces) {
      assert.ok(surfaced.has(surface), `${workflow.title} references unknown surface ${surface}`);
    }
  }
});

test("acceptance workflow status stays honest for offline and empty-catalog states", () => {
  assert.equal(
    acceptanceWorkflowStatus({ gatewayOffline: true, catalogUnavailable: false }),
    "Gateway offline"
  );
  assert.equal(
    acceptanceWorkflowStatus({ gatewayOffline: false, catalogUnavailable: true }),
    "Waiting on app catalog"
  );
  assert.equal(
    acceptanceWorkflowStatus({ gatewayOffline: false, catalogUnavailable: false }),
    "Connect required apps"
  );
  assert.equal(
    acceptanceWorkflowStatus({
      gatewayOffline: false,
      catalogUnavailable: false,
      availableEntries: [coreEntry("gmail")],
    }),
    "Catalog loaded"
  );
});

test("workflowCatalogStatus reports missing apps for partial catalogs without counting built-ins", () => {
  const workflow = ACCEPTANCE_WORKFLOWS.find((entry) => entry.title === "Calendar prep assistant");

  assert.deepEqual(
    workflowCatalogStatus(workflow, {
      availableEntries: [coreEntry("gmail"), coreEntry("google-calendar")],
    }),
    {
      label: "1 app missing from catalog",
      tone: "warning",
      missingSurfaces: ["google-drive"],
    }
  );
});

test("workflowCatalogStatus marks fully catalog-backed workflows ready to connect", () => {
  const workflow = ACCEPTANCE_WORKFLOWS.find((entry) => entry.title === "Daily news digest");

  assert.deepEqual(
    workflowCatalogStatus(workflow, {
      availableEntries: [coreEntry("telegram")],
    }),
    {
      label: "Ready to connect",
      tone: "positive",
      missingSurfaces: [],
    }
  );
});

test("core connection fallbacks resolve to connector app favicons", () => {
  const iconsById = Object.fromEntries(
    CORE_CONNECTIONS.map((entry) => [entry.id, connectorIconKind(entry)])
  );

  assert.deepEqual(iconsById, {
    gmail: "gmail",
    "google-calendar": "google-calendar",
    "google-drive": "google-drive",
    "google-sheets": "google-sheets",
    notion: "notion",
    slack: "slack",
    telegram: "telegram",
    github: "github",
    "web-http": "web",
    routines: "routine",
    workspace: "workspace",
  });
});

test("core connection fallback category pills match the product surface", () => {
  const labelsById = Object.fromEntries(
    CORE_CONNECTIONS.map((entry) => [entry.id, coreConnectionKindLabel(entry)])
  );

  assert.equal(labelsById["web-http"], "Web");
  assert.equal(labelsById.routines, "Routine");
  assert.equal(labelsById.workspace, "Files");
  assert.equal(labelsById.notion, "Knowledge");
  assert.equal(labelsById.slack, "Messaging");
});

test("core connection fallbacks are not installable when catalog is empty", () => {
  const gmail = CORE_CONNECTIONS.find((entry) => entry.id === "gmail");

  assert.deepEqual(
    coreConnectionButtonState({
      entry: gmail,
      gatewayOffline: false,
      catalogUnavailable: true,
      isBusy: false,
    }),
    { disabled: true, label: "Not available" }
  );
});

test("core connection fallbacks distinguish offline gateway from unavailable catalog", () => {
  const notion = CORE_CONNECTIONS.find((entry) => entry.id === "notion");

  assert.deepEqual(
    coreConnectionButtonState({
      entry: notion,
      gatewayOffline: true,
      catalogUnavailable: false,
      isBusy: false,
    }),
    { disabled: true, label: "Gateway offline" }
  );
});
