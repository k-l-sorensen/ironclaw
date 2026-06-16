import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const testDir = path.dirname(fileURLToPath(import.meta.url));
const settingsRoot = path.resolve(testDir, '..');
const inferenceTabPath = path.join(settingsRoot, 'components', 'inference-tab.js');
const useSettingsPath = path.join(settingsRoot, 'hooks', 'useSettings.js');
const settingsApiPath = path.join(settingsRoot, 'lib', 'settings-api.js');

// Design Law: "No fake readiness. A surface may not imply a capability the
// gateway cannot prove." The settings-write endpoint (`updateSetting`) is a
// permanent stub today, so the embeddings/sampling field cards on the (visible,
// default) AI-setup tab would silently fail to persist. These assertions lock in
// the gate so the lie cannot be reintroduced without also wiring a real backend.

test('settings-write API is still a stub (gate precondition)', async () => {
  // If this ever flips to a real endpoint, the gate below can be removed — this
  // test is the canary that says "the gate is still load-bearing".
  const source = await readFile(settingsApiPath, 'utf8');
  assert.match(
    source,
    /export function updateSetting[\s\S]*?TODO: requires v2 settings endpoint/,
    'updateSetting must remain a stub for the AI-setup gate to be required'
  );
});

test('useSettings exposes a todo status and never fakes a save', async () => {
  const source = await readFile(useSettingsPath, 'utf8');

  // Status is derived from the stub payload so consumers can gate write controls.
  assert.match(source, /const status = query\.data\?\.todo \? 'todo' : 'ready';/);
  assert.match(source, /\bstatus,/);

  // The save mutation must bail out before marking "Saved" / needsRestart when
  // the gateway did not actually persist the change (stub resolves success:false
  // or todo). Without this, react-query treats the resolved-but-failed promise as
  // success and the UI asserts a change that never reached the backend.
  assert.match(source, /if \(data\?\.success === false \|\| data\?\.todo\) \{\s*return;/);
});

test('inference tab gates stub field cards behind a writable settings backend', async () => {
  const source = await readFile(inferenceTabPath, 'utf8');

  // The gate variable and its use to suppress the INFERENCE_FIELDS sections.
  // Quote style is allowed to vary (mono uses double quotes; the desktop fork
  // used single) — the gate semantics are what matter.
  assert.match(source, /const settingsWritable = settingsStatus !== ['"]todo['"];/);
  assert.match(
    source,
    /const sections = settingsWritable\s*\?\s*filterSettingsSections\(INFERENCE_FIELDS[\s\S]*?:\s*\[\];/
  );

  // The honest, gateway-backed controls must NOT be gated away with the stub
  // fields — the provider summary and ProviderManagement stay on the surface.
  assert.match(source, /showProviderSummary &&/);
  assert.match(source, /<\$\{ProviderManagement\}/);
});
