import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  reloadChannels,
  saveSidecarConfig,
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

export function useSendCommsMessage() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (payload: {
      from_agent_id: string;
      to_agent_id: string;
      message: string;
    }) => sendCommsMessage(payload),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: commsKeys.all });
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
      qc.invalidateQueries({ queryKey: commsKeys.all });
    },
  });
}
