import { useMutation, useQueryClient, type QueryKey } from "@tanstack/react-query";
import {
  reloadChannels,
  saveSidecarConfig,
  removeSidecarConfig,
  sendCommsMessage,
  postCommsTask,
} from "../http/client";
import { channelKeys, commsKeys } from "../queries/keys";

function useInvalidatingMutation<TVariables, TResult>(
  mutationFn: (variables: TVariables) => Promise<TResult>,
  queryKey: QueryKey,
) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn,
    onSuccess: () => qc.invalidateQueries({ queryKey }),
  });
}

export function useReloadChannels() {
  return useInvalidatingMutation(reloadChannels, channelKeys.all);
}

// Save a sidecar channel's schema-driven config (Phase 5,
// sidecar-channel-configure). Invalidates the whole `channelKeys.all`
// subtree because a successful save flips the channel from "discovery"
// to "configured". This refreshes the channel list and any open QR poll.
export function useSaveSidecarConfig() {
  return useInvalidatingMutation(
    ({
      name,
      values,
    }: {
      name: string;
      values: Record<string, string>;
    }) => saveSidecarConfig(name, values),
    channelKeys.all,
  );
}

// Remove a configured sidecar channel. Invalidates the whole channelKeys.all
// subtree because removal flips the channel back to "discovery".
export function useRemoveSidecarConfig() {
  return useInvalidatingMutation(removeSidecarConfig, channelKeys.all);
}

export function useSendCommsMessage() {
  return useInvalidatingMutation(
    (payload: {
      from_agent_id: string;
      to_agent_id: string;
      message: string;
    }) => sendCommsMessage(payload),
    commsKeys.lists(),
  );
}

export function usePostCommsTask() {
  return useInvalidatingMutation(
    (payload: {
      title: string;
      description?: string;
      assigned_to?: string;
    }) => postCommsTask(payload),
    commsKeys.lists(),
  );
}
