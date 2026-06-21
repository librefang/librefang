import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  reloadChannels,
  saveSidecarConfig,
  removeSidecarConfig,
  sendCommsMessage,
  postCommsTask,
} from "../http/client";
import { channelKeys, commsKeys } from "../queries/keys";

export function useReloadChannels() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: reloadChannels,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: channelKeys.all });
    },
  });
}

// Save a sidecar channel's schema-driven config (Phase 5,
// sidecar-channel-configure). Invalidates the whole `channelKeys.all`
// subtree because a successful save flips the channel from "discovery"
// to "configured" — both the top-level list AND any per-channel detail
// view need to re-fetch.
export function useSaveSidecarConfig() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      name,
      values,
    }: {
      name: string;
      values: Record<string, string>;
    }) => saveSidecarConfig(name, values),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: channelKeys.all });
    },
  });
}

// Remove a configured sidecar channel. Invalidates the whole channelKeys.all
// subtree because removal flips the channel back to "discovery".
export function useRemoveSidecarConfig() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => removeSidecarConfig(name),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: channelKeys.all });
    },
  });
}

export function useSendCommsMessage() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (payload: {
      from_agent_id: string;
      to_agent_id: string;
      message: string;
    }) => sendCommsMessage(payload),
    onSuccess: () => {
      // Sending a message changes the events feed and may shift the
      // topology graph (new edge appears when two agents first
      // converse). Both live under `commsKeys.lists()`; per-event
      // detail caches are unaffected.
      qc.invalidateQueries({ queryKey: commsKeys.lists() });
    },
  });
}

export function usePostCommsTask() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (payload: {
      title: string;
      description?: string;
      assigned_to?: string;
    }) => postCommsTask(payload),
    onSuccess: () => {
      // Posting a task emits a comms event; same invalidation scope as
      // useSendCommsMessage.
      qc.invalidateQueries({ queryKey: commsKeys.lists() });
    },
  });
}
