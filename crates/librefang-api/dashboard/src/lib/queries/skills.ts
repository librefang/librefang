import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listSkills,
  clawhubBrowse,
  clawhubSearch,
  clawhubGetSkill,
  skillhubBrowse,
  skillhubSearch,
  skillhubGetSkill,
  fanghubListSkills,
} from "../http/client";
import { skillKeys, clawhubKeys, skillhubKeys, fanghubKeys } from "./keys";

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
    }),
  fanghubList: () =>
    queryOptions({
      queryKey: fanghubKeys.lists(),
      queryFn: fanghubListSkills,
      staleTime: BROWSE_STALE_MS,
    }),
};

export function useSkills() {
  return useQuery(skillQueries.list());
}

export function useClawHubBrowse(sort?: string, limit?: number, cursor?: string) {
  return useQuery(skillQueries.clawhubBrowse(sort, limit, cursor));
}

export function useClawHubSearch(query: string) {
  return useQuery(skillQueries.clawhubSearch(query));
}

export function useClawHubSkill(slug: string) {
  return useQuery(skillQueries.clawhubSkill(slug));
}

export function useSkillHubBrowse(sort?: string) {
  return useQuery(skillQueries.skillhubBrowse(sort));
}

export function useSkillHubSearch(query: string) {
  return useQuery(skillQueries.skillhubSearch(query));
}

export function useSkillHubSkill(slug: string) {
  return useQuery(skillQueries.skillhubSkill(slug));
}

export function useFangHubSkills() {
  return useQuery(skillQueries.fanghubList());
}
