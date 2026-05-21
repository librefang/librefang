import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listChannels,
  getChannelQr,
  getCommsTopology,
  listCommsEvents,
} from "../http/client";
import { channelKeys, commsKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 30_000;
const REFRESH_MS = 30_000;
const TOPOLOGY_REFRESH_MS = 60_000;
const EVENTS_STALE_MS = 10_000;
// QR polling cadence — fast enough that scan / confirm transitions
// surface within one frame of human latency, slow enough not to hammer
// the daemon on an idle dialog. Matches the sidecar's own internal
// retry rhythm in wechat.py (0.5–5s exponential).
const QR_POLL_MS = 2_000;

export const channelQueries = {
  list: () =>
    queryOptions({
      queryKey: channelKeys.lists(),
      queryFn: listChannels,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  // QR-login state for a sidecar that publishes one (WeChat,
  // WhatsApp). Replaces the four pre-migration WhatsApp / WeChat QR
  // endpoints with a single poll against the daemon's projection of
  // the sidecar-published `qr_ready` / `qr_status` events. `staleTime`
  // is 0 because every state transition matters; refetch defaults are
  // applied at the hook layer so callers can opt out of polling
  // (e.g. when the details modal closes).
  qr: (name: string) =>
    queryOptions({
      queryKey: channelKeys.qr(name),
      queryFn: () => getChannelQr(name),
      staleTime: 0,
    }),
};

export const commsQueries = {
  topology: () =>
    queryOptions({
      queryKey: commsKeys.topology(),
      queryFn: getCommsTopology,
      staleTime: STALE_MS,
      refetchInterval: TOPOLOGY_REFRESH_MS,
      refetchIntervalInBackground: false, // #3393
    }),
  events: (limit = 200) =>
    queryOptions({
      queryKey: commsKeys.events(limit),
      queryFn: () => listCommsEvents(limit),
      staleTime: EVENTS_STALE_MS,
    }),
};

export function useChannels(options: QueryOverrides = {}) {
  return useQuery(withOverrides(channelQueries.list(), options));
}

/** Poll the current QR-login state for `name`. Disabled by default
 * (`enabled: false`) because the daemon's projection is meaningful only
 * while the operator has the details modal open; consumers flip
 * `enabled` to `true` on mount. `refetchInterval` is `QR_POLL_MS`
 * unless the caller overrides it (closing the modal passes `false`
 * to stop the poll loop). */
export function useChannelQr(
  name: string,
  options: { enabled?: boolean; refetchInterval?: number | false } = {},
) {
  const base = channelQueries.qr(name);
  return useQuery({
    ...base,
    enabled: (options.enabled ?? false) && Boolean(name),
    refetchInterval: options.refetchInterval ?? QR_POLL_MS,
    refetchIntervalInBackground: false, // #3393 — tab-aware polling.
  });
}

export function useCommsTopology(options: QueryOverrides = {}) {
  return useQuery(withOverrides(commsQueries.topology(), options));
}

export function useCommsEvents(
  limit = 200,
  options: { enabled?: boolean; refetchInterval?: number | false } = {},
) {
  return useQuery({
    ...commsQueries.events(limit),
    enabled: options.enabled,
    refetchInterval: options.refetchInterval ?? REFRESH_MS,
    refetchIntervalInBackground: false, // #3393
  });
}
