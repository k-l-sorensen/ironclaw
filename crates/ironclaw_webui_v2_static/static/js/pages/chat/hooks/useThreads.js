import { useInfiniteQuery } from "@tanstack/react-query";
import { React } from "../../../lib/html.js";
import {
  createThread as createThreadRequest,
  deleteThread as deleteThreadRequest,
  listThreads,
} from "../../../lib/api.js";
import { queryClient } from "../../../lib/query-client.js";

export function useThreads() {
  // No polling: the sidebar refreshes via `queryClient.invalidateQueries`
  // after a local `createThread` succeeds, and the v2 deployment has no
  // out-of-band thread producers (no Telegram channel, no background
  // routine) in this binary. The fork's 5s poll was inherited from a v1
  // multi-channel context that doesn't apply here.
  //
  // Cursor pagination: listThreads returns one page + a `next_cursor`. We page
  // through it on demand (Load older / search-miss) so every thread stays
  // reachable from the sidebar, thread search, and command palette — the page-1
  // hard cap was the real reason old work went missing.
  const query = useInfiniteQuery({
    queryKey: ["threads"],
    queryFn: ({ pageParam }) => listThreads(pageParam ? { cursor: pageParam } : {}),
    initialPageParam: null,
    getNextPageParam: (lastPage) => lastPage?.next_cursor || undefined,
  });

  const [activeThreadId, setActiveThreadId] = React.useState(null);
  const [isCreating, setIsCreating] = React.useState(false);
  const createInFlightRef = React.useRef(null);

  const handleCreateThread = React.useCallback(async () => {
    if (createInFlightRef.current) return createInFlightRef.current;

    setIsCreating(true);
    const createPromise = (async () => {
      try {
        const data = await createThreadRequest();
        queryClient.invalidateQueries({ queryKey: ["threads"] });
        // RebornCreateThreadResponse → { thread: SessionThreadRecord }.
        // SessionThreadRecord uses `thread_id`, not `id`.
        const threadId = data?.thread?.thread_id;
        if (threadId) setActiveThreadId(threadId);
        return threadId;
      } finally {
        setIsCreating(false);
        createInFlightRef.current = null;
      }
    })();

    createInFlightRef.current = createPromise;
    return createPromise;
  }, []);

  const handleDeleteThread = React.useCallback(
    async (threadId) => {
      await deleteThreadRequest({ threadId });
      if (activeThreadId === threadId) {
        setActiveThreadId(null);
      }
      queryClient.invalidateQueries({ queryKey: ["threads"] });
    },
    [activeThreadId]
  );

  // Normalize v2 SessionThreadRecord → fork's expected shape, flattened across
  // every loaded page:
  // - v2 carries `thread_id`; fork's thread-sidebar reads `thread.id`
  // - v2 has no `state`, `turn_count`, `updated_at` fields
  //   (those are v1 metadata). Fill safe defaults so the UI's
  //   "Processing" pip and turn count never spuriously render.
  const threads = React.useMemo(() => {
    const records = (query.data?.pages || []).flatMap((page) => page?.threads || []);
    return records.map((record) => ({
      ...record,
      id: record.thread_id,
      state: record.state || null,
      turn_count: record.turn_count || 0,
      updated_at: record.updated_at || null,
    }));
  }, [query.data]);

  return {
    threads,
    activeThreadId,
    setActiveThreadId,
    isLoading: query.isLoading,
    // Surface load failure + a refetch so the sidebar can tell "still loading"
    // and "couldn't load" apart from a genuinely empty list — instead of all
    // three collapsing into a false "No conversations yet".
    isError: query.isError,
    refetch: query.refetch,
    // Cursor pagination controls consumed by the sidebar's "Load older" row and
    // its search-miss auto-load.
    hasMoreThreads: Boolean(query.hasNextPage),
    isLoadingMore: query.isFetchingNextPage,
    loadMoreThreads: query.fetchNextPage,
    isCreating,
    createThread: handleCreateThread,
    deleteThread: handleDeleteThread,
  };
}
