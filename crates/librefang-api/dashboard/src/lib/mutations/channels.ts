import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  configureChannel,
  createChannelInstance,
  updateChannelInstance,
  deleteChannelInstance,
  testChannel,
  reloadChannels,
  sendCommsMessage,
  postCommsTask,
} from "../http/client";
import { channelKeys, commsKeys } from "../queries/keys";

export function useConfigureChannel() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      channelName,
      config,
    }: {
      channelName: string;
      config: Record<string, unknown>;
    }) => configureChannel(channelName, config),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: channelKeys.all });
    },
  });
}

// Per-instance mutations (#4837). Each one invalidates the entire
// `channelKeys.all` subtree because every CRUD changes both the per-channel
// instance list AND the top-level channel list's `instance_count` /
// `configured` fields.

export function useCreateChannelInstance() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      channelName,
      fields,
    }: {
      channelName: string;
      fields: Record<string, unknown>;
    }) => createChannelInstance(channelName, fields),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: channelKeys.all });
    },
  });
}

export function useUpdateChannelInstance() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      channelName,
      index,
      fields,
      signature,
      clearSecrets,
    }: {
      channelName: string;
      index: number;
      fields: Record<string, unknown>;
      /** CAS token from the list response. The server rejects the PUT with
       *  409 if a concurrent edit shifted indices or modified this row. */
      signature: string;
      /** Field keys whose secret env-var ref should be actively dropped
       *  (and the env-var line removed from secrets.env, if no sibling
       *  instance still references it). */
      clearSecrets?: string[];
    }) => updateChannelInstance(channelName, index, fields, signature, clearSecrets),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: channelKeys.all });
    },
  });
}

export function useDeleteChannelInstance() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      channelName,
      index,
      signature,
    }: {
      channelName: string;
      index: number;
      /** CAS token from the list response — see `useUpdateChannelInstance`. */
      signature: string;
    }) => deleteChannelInstance(channelName, index, signature),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: channelKeys.all });
    },
  });
}

// Fire-and-forget: one-shot probe, test result returned to caller, no cache to invalidate.
export function useTestChannel() {
  return useMutation({
    mutationFn: (channelName: string) => testChannel(channelName),
  });
}

export function useReloadChannels() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: reloadChannels,
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
