// Audit-trail queries.
//
// `useAuditQuery` wires the dashboard layer to the searchable `/api/audit/query`
// endpoint that ships with M5 / #3203. Until that endpoint exists the daemon
// returns 404; the AuditPage component renders a placeholder rather than
// surfacing the failure. When M5 lands, only the placeholder swap is needed.

import { queryOptions, useQuery } from "@tanstack/react-query";
import { queryAudit, type AuditQueryFilters } from "../http/client";
import { auditKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 15_000;

export const auditQueries = {
  query: (filters: AuditQueryFilters = {}) =>
    queryOptions({
      queryKey: auditKeys.query(filters),
      queryFn: () => queryAudit(filters),
      staleTime: STALE_MS,
    }),
};

export function useAuditQuery(
  filters: AuditQueryFilters = {},
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(auditQueries.query(filters), options));
}
