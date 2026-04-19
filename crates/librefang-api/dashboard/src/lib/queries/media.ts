import { queryOptions, skipToken, useQuery } from "@tanstack/react-query";
import {
  listMediaProviders,
  pollVideo,
  type MediaVideoStatus,
} from "../http/client";
import { mediaKeys } from "./keys";

const STALE_MS = 60_000;
const REFRESH_MS = 60_000;
const VIDEO_TASK_STALE_MS = 1_000;
const VIDEO_TASK_REFETCH_MS = 5_000;

type UseMediaProvidersOptions = {
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?: number | false;
};

type UseVideoTaskOptions = {
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?: number | false;
};

type VideoTaskParams = {
  taskId: string;
  provider: string;
};

export const mediaQueries = {
  providers: () =>
    queryOptions({
      queryKey: mediaKeys.providers(),
      queryFn: listMediaProviders,
      staleTime: STALE_MS,
      refetchInterval: REFRESH_MS,
    }),
  videoTask: ({ taskId, provider }: VideoTaskParams) =>
    queryOptions({
      queryKey: mediaKeys.videoTask(taskId, provider),
      queryFn: () => pollVideo(taskId, provider),
      staleTime: VIDEO_TASK_STALE_MS,
      gcTime: 0,
    }),
};

export function useMediaProviders(options: UseMediaProvidersOptions = {}) {
  const { enabled, staleTime, refetchInterval } = options;
  const query = mediaQueries.providers();

  return useQuery({
    ...query,
    ...(enabled !== undefined ? { enabled } : {}),
    ...(staleTime !== undefined ? { staleTime } : {}),
    ...(refetchInterval !== undefined ? { refetchInterval } : {}),
  });
}

function shouldPollVideoTask(status?: MediaVideoStatus) {
  if (!status) return true;
  return status.status !== "completed" && status.status !== "failed" && !status.error;
}

export function useVideoTask(
  params: VideoTaskParams | null,
  options: UseVideoTaskOptions = {},
) {
  const { enabled, staleTime, refetchInterval } = options;
  const isEnabled = Boolean(enabled ?? true) && Boolean(params?.taskId) && Boolean(params?.provider);

  return useQuery({
    queryKey: params ? mediaKeys.videoTask(params.taskId, params.provider) : mediaKeys.videoTask("pending", "pending"),
    queryFn: params ? () => pollVideo(params.taskId, params.provider) : skipToken,
    gcTime: 0,
    enabled: isEnabled,
    staleTime: staleTime ?? VIDEO_TASK_STALE_MS,
    refetchInterval: (query) => {
      const resolvedInterval = refetchInterval ?? VIDEO_TASK_REFETCH_MS;
      if (resolvedInterval === false) return false;
      return shouldPollVideoTask(query.state.data as MediaVideoStatus | undefined)
        ? resolvedInterval
        : false;
    },
    refetchIntervalInBackground: true,
  });
}
