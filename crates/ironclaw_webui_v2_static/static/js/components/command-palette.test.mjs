import assert from 'node:assert/strict';
import test from 'node:test';

import {
  buildCommandPaletteActions,
  visibleCommandRoutes
} from './command-palette.js';

test('command palette mirrors the simplified visible desktop routes', () => {
  assert.deepEqual(
    visibleCommandRoutes().map((route) => route.id),
    ['chat', 'work', 'extensions', 'settings']
  );
});

test('command palette actions do not expose hidden product surfaces', () => {
  const actions = buildCommandPaletteActions({
    navigate: () => {},
    onNewChat: () => {},
    onToggleTheme: () => {}
  });
  const labels = actions.map((action) => action.label);

  assert.equal(labels.includes('Go to Chat'), true);
  assert.equal(labels.includes('Go to Work'), true);
  assert.equal(labels.includes('Go to Automations'), false);
  assert.equal(labels.includes('Go to Connections'), true);
  assert.equal(labels.includes('Go to Settings'), true);

  for (const hidden of ['Automations', 'Projects', 'Missions', 'Jobs', 'Routines', 'Admin']) {
    assert.equal(
      labels.some((label) => label.includes(hidden)),
      false,
      `${hidden} should not appear in the command palette`
    );
  }
});

test('command palette route actions navigate to the registered route paths', () => {
  const navigated = [];
  const actions = buildCommandPaletteActions({
    navigate: (path) => navigated.push(path),
    onNewChat: () => {},
    onToggleTheme: () => {}
  });

  actions.find((action) => action.id === 'go-extensions').run();
  actions.find((action) => action.id === 'go-settings').run();

  assert.deepEqual(navigated, ['/extensions', '/settings']);
});

test('command palette surfaces saved work items by title and opens them in Work', () => {
  const navigated = [];
  const actions = buildCommandPaletteActions({
    navigate: (path) => navigated.push(path),
    onNewChat: () => {},
    onToggleTheme: () => {},
    workItems: [
      { id: 'w1', title: 'Services agreement draft', artifacts: [{ id: 'a1' }] },
      { id: 'w2', title: 'Q3 board update', artifacts: [] },
    ],
  });

  const savedWork = actions.filter((a) => a.group === 'Saved work');
  assert.deepEqual(
    savedWork.map((a) => a.label),
    ['Services agreement draft', 'Q3 board update'],
  );

  // Opening a work item deep-links into Work with the item (and first artifact).
  savedWork.find((a) => a.id === 'work-w1').run();
  savedWork.find((a) => a.id === 'work-w2').run();
  assert.deepEqual(navigated, ['/work?item=w1&artifact=a1', '/work?item=w2']);
});

test('command palette caps injected saved-work rows so a large store cannot flood it', () => {
  const many = Array.from({ length: 25 }, (_, i) => ({ id: `w${i}`, title: `Work ${i}` }));
  const actions = buildCommandPaletteActions({
    navigate: () => {},
    onNewChat: () => {},
    onToggleTheme: () => {},
    workItems: many,
  });
  assert.equal(actions.filter((a) => a.group === 'Saved work').length, 8);
});

test('command palette without saved work omits the Saved work group entirely', () => {
  const actions = buildCommandPaletteActions({
    navigate: () => {},
    onNewChat: () => {},
    onToggleTheme: () => {},
  });
  assert.equal(
    actions.some((a) => a.group === 'Saved work'),
    false,
  );
});
