// Knowledge-base queries (#8327).
//
// Pages MUST consume these hooks rather than calling `api.*` or `fetch`
// directly. The base list carries each base's holder list, so a page showing
// "who can read this" needs one query rather than one per base — the daemon
// already walks the agent registry once to build it.

import { queryOptions, useQuery } from "@tanstack/react-query";
import { listKnowledgeBases, listKnowledgeDocuments } from "../../api";
import { knowledgeKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

// Documents change only when an operator uploads one, so a short stale time
// would spend requests on a surface nobody else writes to.
const STALE_MS = 30_000;

export const knowledgeQueries = {
  list: () =>
    queryOptions({
      queryKey: knowledgeKeys.list(),
      queryFn: listKnowledgeBases,
      staleTime: STALE_MS,
    }),
  documents: (name: string) =>
    queryOptions({
      queryKey: knowledgeKeys.documentsFor(name),
      queryFn: () => listKnowledgeDocuments(name),
      enabled: !!name,
      staleTime: STALE_MS,
    }),
};

export function useKnowledgeBases(overrides: QueryOverrides = {}) {
  return useQuery(withOverrides(knowledgeQueries.list(), overrides));
}

export function useKnowledgeDocuments(name: string, overrides: QueryOverrides = {}) {
  return useQuery(withOverrides(knowledgeQueries.documents(name), overrides));
}
