import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { INFERENCE_FIELDS } from './settings-schema.js';

function fieldByKey(key) {
  for (const group of INFERENCE_FIELDS) {
    const field = group.fields.find((entry) => entry.key === key);
    if (field) return field;
  }
  return null;
}

// Merged behavior: on web (this test runs in node, where `isDesktopRuntime()`
// is false) the embeddings provider keeps the full choice — equal to today's
// mono. The desktop NEAR-only narrowing is gated behind `isDesktopRuntime()`
// (asserted from source below), so it never trims the web catalog.
test('web inference settings keep the full embeddings provider choice', () => {
  const field = fieldByKey('embeddings.provider');

  assert.ok(field, 'embeddings.provider field should exist');
  assert.deepEqual(field.options, ['openai', 'nearai']);
});

test('desktop NEAR-only embeddings narrowing is gated behind isDesktopRuntime()', () => {
  const source = readFileSync(new URL('./settings-schema.js', import.meta.url), 'utf8');

  // The narrowing branch must be reached only under the desktop runtime gate,
  // and it must carry the NEAR-only option + relabel + fixed-single-option flag.
  assert.match(source, /isDesktopRuntime\(\)/);
  assert.match(source, /options: \["nearai"\]/);
  assert.match(source, /optionLabels: \{ nearai: "NEAR AI Cloud" \}/);
  assert.match(source, /allowDefault: false/);
});

// The Trace Commons tab is a web feature (real `/traces/*` endpoints). The
// desktop fork dropped it; the merge keeps it (hidden on desktop at the
// runtime gate).
test('the traces tab is present in the settings schema', async () => {
  const { SETTINGS_TABS } = await import('./settings-schema.js');
  assert.ok(
    SETTINGS_TABS.some((tab) => tab.id === 'traces'),
    'the Trace Commons tab must remain in the web settings schema'
  );
});
