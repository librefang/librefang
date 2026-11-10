import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  getContextEngineChain,
  getContextEngineConfig,
  getContextEngineHealth,
  getContextEngineMetrics,
  getContextEngineSandboxPolicy,
  getContextEngineTraces,
} from "../http/client";
import { contextEngineKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const REFRESH_MS = {
  health: 30_000,
  chain: 60_000,
  metrics: 15_000,
  traces: 15_000,
  config: 60_000,
  sandboxPolicy: 60_000,
} as const;

export const contextEngineQueries = {
  health: () =>
    queryOptions({
      queryKey: contextEngineKeys.health(),
      queryFn: getContextEngineHealth,
      staleTime: REFRESH_MS.health,
      refetchInterval: REFRESH_MS.health,
      refetchIntervalInBackground: false,
    }),
  chain: () =>
    queryOptions({
      queryKey: contextEngineKeys.chain(),
      queryFn: getContextEngineChain,
      staleTime: REFRESH_MS.chain,
      refetchInterval: REFRESH_MS.chain,
      refetchIntervalInBackground: false,
    }),
  metrics: () =>
    queryOptions({
      queryKey: contextEngineKeys.metrics(),
      queryFn: getContextEngineMetrics,
      staleTime: REFRESH_MS.metrics,
      refetchInterval: REFRESH_MS.metrics,
      refetchIntervalInBackground: false,
    }),
  traces: () =>
    queryOptions({
      queryKey: contextEngineKeys.traces(),
      queryFn: getContextEngineTraces,
      staleTime: REFRESH_MS.traces,
      refetchInterval: REFRESH_MS.traces,
      refetchIntervalInBackground: false,
    }),
  config: () =>
    queryOptions({
      queryKey: contextEngineKeys.config(),
      queryFn: getContextEngineConfig,
      staleTime: REFRESH_MS.config,
      refetchInterval: REFRESH_MS.config,
      refetchIntervalInBackground: false,
    }),
  sandboxPolicy: () =>
    queryOptions({
      queryKey: contextEngineKeys.sandboxPolicy(),
      queryFn: getContextEngineSandboxPolicy,
      staleTime: REFRESH_MS.sandboxPolicy,
      refetchInterval: REFRESH_MS.sandboxPolicy,
      refetchIntervalInBackground: false,
    }),
};

export function useContextEngineHealth(options: QueryOverrides = {}) {
  return useQuery(withOverrides(contextEngineQueries.health(), options));
}

export function useContextEngineChain(options: QueryOverrides = {}) {
  return useQuery(withOverrides(contextEngineQueries.chain(), options));
}

export function useContextEngineMetrics(options: QueryOverrides = {}) {
  return useQuery(withOverrides(contextEngineQueries.metrics(), options));
}

export function useContextEngineTraces(options: QueryOverrides = {}) {
  return useQuery(withOverrides(contextEngineQueries.traces(), options));
}

export function useContextEngineConfig(options: QueryOverrides = {}) {
  return useQuery(withOverrides(contextEngineQueries.config(), options));
}

export function useContextEngineSandboxPolicy(options: QueryOverrides = {}) {
  return useQuery(withOverrides(contextEngineQueries.sandboxPolicy(), options));
}
