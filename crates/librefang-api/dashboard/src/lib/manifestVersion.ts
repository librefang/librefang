import type { QueryClient } from "@tanstack/react-query";
import { agentQueries } from "./queries/agents";

/**
 * Re-read the manifest ETag after a write the manifest form did not make
 * (#8424).
 *
 * The grant panels and their modals write through their own endpoints, so the
 * token the form was seeded with no longer describes the server's manifest.
 * Reading it back keeps the next form save from failing with 409 on a write
 * the user did make.
 *
 * The `staleTime: 0` override is the whole point: `createDashboardQueryClient`
 * defaults every query to a 30 s `staleTime`, and `fetchQuery` inside that
 * window returns the cached snapshot without touching the server. That made
 * this refresh a no-op — the form kept its pre-panel ETag and the next save
 * 409'd. Pinning the fetch stale forces the network read regardless of the
 * cache's freshness.
 */
export async function fetchManifestVersion(
  qc: QueryClient,
  agentId: string,
): Promise<string | null> {
  const fresh = await qc.fetchQuery({
    ...agentQueries.manifest(agentId),
    staleTime: 0,
  });
  return fresh.version;
}
