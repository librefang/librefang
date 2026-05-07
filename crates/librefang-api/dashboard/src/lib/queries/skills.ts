import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listSkills,
  getSkillDetail,
  getSupportingFile,
  listPendingCandidates,
  getPendingCandidate,
  clawhubBrowse,
  clawhubSearch,
  clawhubGetSkill,
  clawhubCnBrowse,
  clawhubCnSearch,
  clawhubCnGetSkill,
  skillhubBrowse,
  skillhubSearch,
  skillhubGetSkill,
  fanghubListSkills,
} from "../http/client";
import { skillKeys, clawhubKeys, clawhubCnKeys, skillhubKeys, fanghubKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 30_000;
const REFRESH_MS = 30_000;
const BROWSE_STALE_MS = 60_000;

export const skillQueries = {
  list: () =>
    queryOptions({
      queryKey: skillKeys.lists(),
      queryFn: listSkills,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  detail: (name: string) =>
    queryOptions({
      queryKey: skillKeys.detail(name),
      queryFn: () => getSkillDetail(name),
      enabled: !!name,
      staleTime: STALE_MS,
    }),
  supportingFile: (name: string, path: string) =>
    queryOptions({
      queryKey: skillKeys.supportingFile(name, path),
      queryFn: () => getSupportingFile(name, path),
      enabled: !!name && !!path,
      staleTime: STALE_MS,
    }),
  clawhubBrowse: (sort?: string, limit?: number, cursor?: string) =>
    queryOptions({
      queryKey: clawhubKeys.browse({ sort, limit, cursor }),
      queryFn: () => clawhubBrowse(sort, limit, cursor),
      staleTime: BROWSE_STALE_MS,
    }),
  clawhubSearch: (query: string) =>
    queryOptions({
      queryKey: clawhubKeys.search(query),
      queryFn: () => clawhubSearch(query),
      enabled: !!query,
      staleTime: STALE_MS,
    }),
  clawhubSkill: (slug: string) =>
    queryOptions({
      queryKey: clawhubKeys.detail(slug),
      queryFn: () => clawhubGetSkill(slug),
      enabled: !!slug,
      staleTime: BROWSE_STALE_MS,
    }),
  clawhubCnBrowse: (sort?: string, limit?: number, cursor?: string) =>
    queryOptions({
      queryKey: clawhubCnKeys.browse({ sort, limit, cursor }),
      queryFn: () => clawhubCnBrowse(sort, limit, cursor),
      staleTime: BROWSE_STALE_MS,
    }),
  clawhubCnSearch: (query: string) =>
    queryOptions({
      queryKey: clawhubCnKeys.search(query),
      queryFn: () => clawhubCnSearch(query),
      enabled: !!query,
      staleTime: STALE_MS,
    }),
  clawhubCnSkill: (slug: string) =>
    queryOptions({
      queryKey: clawhubCnKeys.detail(slug),
      queryFn: () => clawhubCnGetSkill(slug),
      enabled: !!slug,
    }),
  skillhubBrowse: (sort?: string) =>
    queryOptions({
      queryKey: skillhubKeys.browse(sort),
      queryFn: () => skillhubBrowse(sort),
      staleTime: BROWSE_STALE_MS,
    }),
  skillhubSearch: (query: string) =>
    queryOptions({
      queryKey: skillhubKeys.search(query),
      queryFn: () => skillhubSearch(query),
      enabled: !!query,
      staleTime: STALE_MS,
    }),
  skillhubSkill: (slug: string) =>
    queryOptions({
      queryKey: skillhubKeys.detail(slug),
      queryFn: () => skillhubGetSkill(slug),
      enabled: !!slug,
      staleTime: BROWSE_STALE_MS,
    }),
  fanghubList: () =>
    queryOptions({
      queryKey: fanghubKeys.lists(),
      queryFn: fanghubListSkills,
      staleTime: BROWSE_STALE_MS,
    }),
  // Skill workshop (#3328) — passive after-turn capture review.
  pendingList: (agent?: string | null) =>
    queryOptions({
      queryKey: skillKeys.pendingList(agent ?? null),
      queryFn: () => listPendingCandidates(agent ?? undefined),
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false,
    }),
  pendingDetail: (id: string) =>
    queryOptions({
      queryKey: skillKeys.pendingDetail(id),
      queryFn: () => getPendingCandidate(id),
      enabled: !!id,
      staleTime: STALE_MS,
    }),
};

export function useSkills(options: QueryOverrides = {}) {
  return useQuery(withOverrides(skillQueries.list(), options));
}

export function useSkillDetail(name: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(skillQueries.detail(name), options));
}

export function useSupportingFile(
  name: string,
  path: string,
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(skillQueries.supportingFile(name, path), options));
}

export function useClawHubBrowse(sort?: string, limit?: number, cursor?: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(skillQueries.clawhubBrowse(sort, limit, cursor), options));
}

export function useClawHubSearch(query: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(skillQueries.clawhubSearch(query), options));
}

export function useClawHubSkill(slug: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(skillQueries.clawhubSkill(slug), options));
}

export function useSkillHubBrowse(sort?: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(skillQueries.skillhubBrowse(sort), options));
}

export function useSkillHubSearch(query: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(skillQueries.skillhubSearch(query), options));
}

export function useSkillHubSkill(slug: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(skillQueries.skillhubSkill(slug), options));
}

export function useFangHubSkills(options: QueryOverrides = {}) {
  return useQuery(withOverrides(skillQueries.fanghubList(), options));
}

export function usePendingSkillCandidates(
  agent?: string | null,
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(skillQueries.pendingList(agent), options));
}

export function usePendingSkillCandidate(id: string, options: QueryOverrides = {}) {
  return useQuery(withOverrides(skillQueries.pendingDetail(id), options));
}
