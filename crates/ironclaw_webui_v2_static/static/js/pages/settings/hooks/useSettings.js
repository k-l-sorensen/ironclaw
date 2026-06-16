import { React } from "../../../lib/html.js";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  fetchSettingsExport,
  importSettings as importSettingsPayload,
  updateSetting,
} from "../lib/settings-api.js";
import { throwIfApiFailed } from "../lib/api-result.js";
import { RESTART_REQUIRED_KEYS } from "../lib/settings-schema.js";

export function useSettings() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["settings-export"],
    queryFn: fetchSettingsExport,
    staleTime: 30_000,
  });

  const settings = query.data?.settings || {};
  // No v2 settings-write endpoint exists yet: `fetchSettingsExport` may resolve
  // with `{ todo: true }`. `status:'todo'` lets the consuming write tabs (agent,
  // networking, inference) gate their controls behind a real backend instead of
  // implying a capability the gateway cannot prove ("No fake readiness"). The
  // save path itself stays guarded by `throwIfApiFailed`, so a `{ success: false }`
  // write never flashes a fake "Saved" indicator.
  const status = query.data?.todo ? 'todo' : 'ready';

  const [savedKeys, setSavedKeys] = React.useState({});
  const [needsRestart, setNeedsRestart] = React.useState(false);

  const mutation = useMutation({
    // A resolved response with `success: false` is a failed save, not a
    // success — surface it so the UI shows the error rather than a fake
    // "Saved" indicator (and never flips `needsRestart`).
    mutationFn: async ({ key, value }) =>
      throwIfApiFailed(await updateSetting(key, value), "Save failed"),
    onSuccess: (_data, { key, value }) => {
      queryClient.setQueryData(["settings-export"], (old) => {
        if (!old) return old;
        const next = { ...old, settings: { ...old.settings } };
        if (value === null || value === undefined) {
          delete next.settings[key];
        } else {
          next.settings[key] = value;
        }
        return next;
      });

      setSavedKeys((prev) => ({ ...prev, [key]: true }));
      setTimeout(() => setSavedKeys((prev) => ({ ...prev, [key]: false })), 2000);

      if (RESTART_REQUIRED_KEYS.has(key)) {
        setNeedsRestart(true);
      }
    },
  });

  const save = React.useCallback(
    (key, value) => mutation.mutate({ key, value }),
    [mutation]
  );

  const importMutation = useMutation({
    mutationFn: importSettingsPayload,
    onSuccess: (_data, payload) => {
      queryClient.invalidateQueries({ queryKey: ["settings-export"] });
      const importedKeys = Object.keys(payload?.settings || {});
      if (importedKeys.some((key) => RESTART_REQUIRED_KEYS.has(key))) {
        setNeedsRestart(true);
      }
    },
  });

  const importSettings = React.useCallback(
    (payload) => importMutation.mutateAsync(payload),
    [importMutation]
  );

  return {
    settings,
    query,
    status,
    save,
    savedKeys,
    needsRestart,
    importSettings,
    isImporting: importMutation.isPending,
    saveError: mutation.error || importMutation.error,
  };
}
