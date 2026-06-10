import { useQuery } from "@tanstack/react-query";
import { fetchTraceCredits } from "../lib/settings-api.js";

export function useTraceCredits() {
  const query = useQuery({
    queryKey: ["trace-credits"],
    queryFn: fetchTraceCredits,
    // Credits change slowly (capture -> score gate -> submit -> server
    // accept), but the sidebar card and Settings tab should reflect new
    // accepted submissions without a manual reload. A 60s poll plus a
    // focus refetch is cheap and keeps both surfaces live.
    refetchInterval: 60_000,
    refetchOnWindowFocus: true,
  });

  return {
    credits: query.data || null,
    query,
  };
}
