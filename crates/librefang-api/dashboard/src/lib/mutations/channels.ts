import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  configureChannel,
  testChannel,
  reloadChannels,
  sendCommsMessage,
  postCommsTask,
} from "../http/client";
import { channelKeys } from "../queries/keys";

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
  return useMutation({
    mutationFn: (payload: {
      from_agent_id: string;
      to_agent_id: string;
      message: string;
    }) => sendCommsMessage(payload),
  });
}

export function usePostCommsTask() {
  return useMutation({
    mutationFn: (payload: {
      title: string;
      description?: string;
      assigned_to?: string;
    }) => postCommsTask(payload),
  });
}
