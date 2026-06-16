import { React } from "../../../lib/html.js";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { fetchTools, updateToolPermission } from "../lib/settings-api.js";

export function useTools() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["settings-tools"],
    queryFn: fetchTools,
  });

  const tools = query.data?.tools || [];
  // No v2 tools endpoint exists yet: `fetchTools` returns `{ todo: true }` and
  // `updateToolPermission` is a `{ success: false }` stub. `status:'todo'` lets
  // the tab gate the permission controls behind a real backend so a select never
  // implies a saved change that never reached the gateway ("No fake readiness").
  const status = query.data?.todo ? 'todo' : 'ready';

  const [savedTools, setSavedTools] = React.useState({});

  const mutation = useMutation({
    mutationFn: ({ name, state }) => updateToolPermission(name, state),
    onSuccess: (data, { name, state }) => {
      // The stub resolves without persisting. Only reflect the change and flash
      // "saved" when the gateway actually confirms the write — otherwise the row
      // and the saved indicator would assert a permission change that never
      // happened.
      if (data?.success === false || data?.todo) {
        return;
      }

      queryClient.setQueryData(["settings-tools"], (old) => {
        if (!old) return old;
        return {
          ...old,
          tools: old.tools.map((t) => (t.name === name ? { ...t, state } : t)),
        };
      });
      setSavedTools((prev) => ({ ...prev, [name]: true }));
      setTimeout(() => setSavedTools((prev) => ({ ...prev, [name]: false })), 2000);
    },
  });

  const setPermission = React.useCallback(
    (name, state) => mutation.mutate({ name, state }),
    [mutation]
  );

  return { tools, query, status, setPermission, savedTools, error: mutation.error };
}
