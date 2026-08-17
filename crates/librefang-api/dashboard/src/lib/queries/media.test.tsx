import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { pollVideo } from "../http/client";
import { createQueryClientWrapper } from "../test/query-client";
import { mediaQueries, useVideoTask } from "./media";

vi.mock("../http/client", () => ({
  listMediaProviders: vi.fn(),
  pollVideo: vi.fn(),
}));

describe("video task query", () => {
  it("keeps terminal task data cached across short remounts", () => {
    expect(
      mediaQueries.videoTask({ taskId: "task-1", provider: "fal" }).gcTime,
    ).toBe(5 * 60_000);
  });

  it("does not fetch without submitted task parameters", () => {
    const { wrapper } = createQueryClientWrapper();

    renderHook(() => useVideoTask(null, { enabled: true }), { wrapper });

    expect(pollVideo).not.toHaveBeenCalled();
  });

  it("accepts an explicit polling override", () => {
    vi.mocked(pollVideo).mockResolvedValue({ status: "processing" });
    const { queryClient, wrapper } = createQueryClientWrapper();

    renderHook(
      () => useVideoTask(
        { taskId: "task-1", provider: "fal" },
        { refetchInterval: false },
      ),
      { wrapper },
    );

    const query = queryClient.getQueryCache().find({
      queryKey: mediaQueries.videoTask({ taskId: "task-1", provider: "fal" }).queryKey,
    });
    const observers = (query as unknown as {
      observers: Array<{ options: { refetchInterval?: number | false } }>;
    }).observers;
    expect(observers[0]?.options.refetchInterval).toBe(false);
  });

  it("polls active tasks and stops after a terminal status by default", () => {
    vi.mocked(pollVideo).mockResolvedValue({ status: "processing" });
    const { queryClient, wrapper } = createQueryClientWrapper();

    renderHook(
      () => useVideoTask(
        { taskId: "task-1", provider: "fal" },
        { enabled: false },
      ),
      { wrapper },
    );

    const query = queryClient.getQueryCache().find({
      queryKey: mediaQueries.videoTask({ taskId: "task-1", provider: "fal" }).queryKey,
    });
    const observers = (query as unknown as {
      observers: Array<{
        options: {
          refetchInterval?: number | false | ((query: unknown) => number | false);
        };
      }>;
    }).observers;
    const refetchInterval = observers[0]?.options.refetchInterval;
    expect(refetchInterval).toBeTypeOf("function");

    const intervalFor = refetchInterval as (query: unknown) => number | false;
    expect(intervalFor({ state: { data: { status: "processing" } } })).toBe(5_000);
    expect(intervalFor({ state: { data: { status: "completed" } } })).toBe(false);
    expect(intervalFor({ state: { data: { status: "failed", error: "failed" } } })).toBe(false);
  });
});
