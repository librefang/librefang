import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  listMediaProviders,
  pollVideo,
  type MediaVideoStatus,
} from "../http/client";
import { mediaKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 60_000;
const REFRESH_MS = 60_000;
const VIDEO_TASK_STALE_MS = 5_000;
const VIDEO_TASK_REFETCH_MS = 5_000;

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

export function useMediaProviders(options: QueryOverrides = {}) {
  return useQuery(withOverrides(mediaQueries.providers(), options));
}

function shouldPollVideoTask(status?: MediaVideoStatus) {
  if (!status) return true;
  return status.status !== "completed" && status.status !== "failed" && !status.error;
}

export function useVideoTask(params: VideoTaskParams | null, options: QueryOverrides = {}) {
  return useQuery({
    ...withOverrides(mediaQueries.videoTask(params ?? { taskId: "", provider: "" }), options),
    enabled: Boolean(params) && options.enabled !== false,
    refetchIntervalInBackground: true,
    refetchInterval: (query) => {
      const data = query.state.data;
      return shouldPollVideoTask(data) ? VIDEO_TASK_REFETCH_MS : false;
    },
  });
}
