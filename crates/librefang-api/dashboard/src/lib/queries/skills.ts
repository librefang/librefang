import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listSkills,
  getSkillDetail,
  getSupportingFile,
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

const STALE_MS = 30_000;
const REFRESH_MS = 30_000;
const BROWSE_STALE_MS = 60_000;

type UseSkillOptions = {
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?: number | false;
};

export const skillQueries = {
  list: () =>
    queryOptions({
      queryKey: skillKeys.lists(),
      queryFn: listSkills,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
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
};

export function useSkills(options: UseSkillOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...skillQueries.list(),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useSkillDetail(name: string, options: UseSkillOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...skillQueries.detail(name),
    enabled: enabled ?? Boolean(name),
    staleTime: staleTime ?? STALE_MS,
    refetchInterval: refetchInterval ?? false,
  });
}

export function useSupportingFile(
  name: string,
  path: string,
  options: UseSkillOptions = {},
) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...skillQueries.supportingFile(name, path),
    enabled: enabled ?? (Boolean(name) && Boolean(path)),
    staleTime: staleTime ?? STALE_MS,
    refetchInterval: refetchInterval ?? false,
  });
}

export function useClawHubBrowse(sort?: string, limit?: number, cursor?: string, options: UseSkillOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...skillQueries.clawhubBrowse(sort, limit, cursor),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useClawHubSearch(query: string, options: UseSkillOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...skillQueries.clawhubSearch(query),
    enabled: enabled ?? !!query,
    staleTime,
    refetchInterval,
  });
}

export function useClawHubSkill(slug: string, options: UseSkillOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...skillQueries.clawhubSkill(slug),
    enabled: enabled ?? !!slug,
    staleTime,
    refetchInterval,
  });
}

export function useSkillHubBrowse(sort?: string, options: UseSkillOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...skillQueries.skillhubBrowse(sort),
    enabled,
    staleTime,
    refetchInterval,
  });
}

export function useSkillHubSearch(query: string, options: UseSkillOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...skillQueries.skillhubSearch(query),
    enabled: enabled ?? !!query,
    staleTime,
    refetchInterval,
  });
}

export function useSkillHubSkill(slug: string, options: UseSkillOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...skillQueries.skillhubSkill(slug),
    enabled: enabled ?? !!slug,
    staleTime,
    refetchInterval,
  });
}

export function useFangHubSkills(options: UseSkillOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  return useQuery({
    ...skillQueries.fanghubList(),
    enabled,
    staleTime,
    refetchInterval,
  });
}
