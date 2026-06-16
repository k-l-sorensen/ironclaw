import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const dir = path.dirname(fileURLToPath(import.meta.url));
const hooksDir = path.resolve(dir, '..', 'pages', 'chat', 'hooks');

// The sidebar thread list must distinguish "still loading", "failed to load",
// and "genuinely empty" — collapsing all three into "No conversations yet" is a
// fake-readiness lie (it claims the user has no chats when the fetch is pending
// or errored). Design Law: "No fake readiness."
test('useThreads surfaces load failure + refetch (not just isLoading)', async () => {
  const src = await readFile(path.join(hooksDir, 'useThreads.js'), 'utf8');
  assert.match(src, /isError: query\.isError/);
  assert.match(src, /refetch: query\.refetch/);
});

test('sidebar passes thread loading/error/retry state into SidebarThreads', async () => {
  const src = await readFile(path.join(dir, 'sidebar.js'), 'utf8');
  assert.match(src, /isLoading=\$\{threadsState\.isLoading\}/);
  assert.match(src, /isError=\$\{threadsState\.isError\}/);
  assert.match(src, /onRetry=\$\{threadsState\.refetch\}/);
});

// Cursor pagination: the thread list must page through next_cursor so old work
// stays reachable from the sidebar, search, and palette — not capped at page 1.
test('useThreads pages through next_cursor (infinite query), exposing load-more + has-more', async () => {
  const src = await readFile(path.join(hooksDir, 'useThreads.js'), 'utf8');
  assert.match(src, /useInfiniteQuery/);
  assert.match(src, /getNextPageParam: \(lastPage\) => lastPage\?\.next_cursor/);
  assert.match(src, /\.pages \|\| \[\]\)\.flatMap/);
  assert.match(src, /hasMoreThreads: Boolean\(query\.hasNextPage\)/);
  assert.match(src, /loadMoreThreads: query\.fetchNextPage/);
  // The dead, unconsumed nextCursor export is gone.
  assert.doesNotMatch(src, /nextCursor: query\.data\?\.next_cursor/);
});

test('sidebar wires pagination into SidebarThreads, which renders a Load-older affordance + search-miss auto-load', async () => {
  const sidebar = await readFile(path.join(dir, 'sidebar.js'), 'utf8');
  assert.match(sidebar, /hasMore=\$\{threadsState\.hasMoreThreads\}/);
  assert.match(sidebar, /onLoadMore=\$\{threadsState\.loadMoreThreads\}/);

  const list = await readFile(path.join(dir, 'sidebar-threads.js'), 'utf8');
  // A "Load older conversations" row exists (localized), gated on hasMore.
  assert.match(list, /t\("chat\.loadOlder"\)/);
  // Search-miss auto-load pages through older threads while a search finds none.
  assert.match(list, /onLoadMore\(\)/);
  assert.match(list, /t\("chat\.searchingOlder"\)/);
});

test('SidebarThreads renders loading + error branches, not a false empty state', async () => {
  const src = await readFile(path.join(dir, 'sidebar-threads.js'), 'utf8');

  // The component accepts the new state.
  assert.match(src, /isLoading = false/);
  assert.match(src, /isError = false/);

  // A loading skeleton and an error+Retry path exist (copy is localized via useT).
  assert.match(src, /v2-skeleton/);
  assert.match(src, /t\("chat\.loadFailed"\)/);
  assert.match(src, /onClick=\$\{\(\) => onRetry\(\)\}/);

  // The empty state must be the fallback of the three-way branch — the
  // skeleton and error copy both precede it in source order.
  const skeletonIdx = src.indexOf('v2-skeleton');
  const errorIdx = src.indexOf('t("chat.loadFailed")');
  const emptyIdx = src.indexOf('t("chat.noConversations")');
  assert.ok(
    skeletonIdx > -1 && errorIdx > -1 && emptyIdx > skeletonIdx && emptyIdx > errorIdx,
    'the loading + error branches must precede the empty-state copy'
  );
});
