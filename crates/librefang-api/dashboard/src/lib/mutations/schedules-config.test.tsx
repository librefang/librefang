import { describe, it, expect, vi } from "vitest";
import * as httpClient from "../http/client";
import { renderHook, waitFor } from "@testing-library/react";
import { useRunSchedule } from "./schedules";
import {
  useBatchSetConfigValues,
  useSetConfigValue,
  useReloadConfig,
} from "./config";
import { scheduleKeys, cronKeys, configKeys, overviewKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  runSchedule: vi.fn().mockResolvedValue({}),
  setConfigValue: vi.fn().mockResolvedValue({}),
  reloadConfig: vi.fn().mockResolvedValue({}),
}));

describe("useRunSchedule", () => {
  it("invalidates scheduleKeys.all and cronKeys.all", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useRunSchedule(), { wrapper });

    await result.current.mutateAsync("schedule-1");

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: scheduleKeys.all });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: cronKeys.all });
  });
});

describe("useSetConfigValue", () => {
  it("invalidates configKeys.all after a config write", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSetConfigValue(), { wrapper });

    await result.current.mutateAsync({ path: "kernel.max_agents", value: 10 });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });
    expect(invalidateSpy).toHaveBeenCalledTimes(1);
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: configKeys.all });
  });

  it("calls options.onSuccess after invalidation", async () => {
    const { wrapper } = createQueryClientWrapper();
    const onSuccess = vi.fn();

    const { result } = renderHook(
      () => useSetConfigValue({ onSuccess }),
      { wrapper },
    );

    await result.current.mutateAsync({ path: "kernel.max_agents", value: 10 });

    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalled();
    });
  });
});

describe("useBatchSetConfigValues", () => {
  it("invalidates configKeys.all once after batch save", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useBatchSetConfigValues(), { wrapper });

    await result.current.mutateAsync([
      { path: "kernel.max_agents", value: 10 },
      { path: "kernel.max_memory", value: 20 },
    ]);

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });
    expect(invalidateSpy).toHaveBeenCalledTimes(1);
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: configKeys.all });
  });

  it("calls options.onSuccess after invalidation", async () => {
    const { wrapper } = createQueryClientWrapper();
    const onSuccess = vi.fn();

    const { result } = renderHook(
      () => useBatchSetConfigValues({ onSuccess }),
      { wrapper },
    );

    await result.current.mutateAsync([{ path: "kernel.max_agents", value: 10 }]);

    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalled();
    });
  });

  it("returns mixed success and error results for partial failure", async () => {
    const { wrapper } = createQueryClientWrapper();
    vi.mocked(httpClient.setConfigValue)
      .mockResolvedValueOnce({ status: "saved" })
      .mockRejectedValueOnce(new Error("boom"));

    const { result } = renderHook(() => useBatchSetConfigValues(), { wrapper });

    await expect(result.current.mutateAsync([
      { path: "kernel.max_agents", value: 10 },
      { path: "kernel.max_memory", value: 20 },
    ])).resolves.toEqual([
      {
        path: "kernel.max_agents",
        value: 10,
        data: { status: "saved" },
      },
      {
        path: "kernel.max_memory",
        value: 20,
        error: expect.any(Error),
      },
    ]);
  });
});

describe("useReloadConfig", () => {
  it("invalidates configKeys.all and overviewKeys.snapshot()", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useReloadConfig(), { wrapper });

    await result.current.mutateAsync();

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: configKeys.all });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: overviewKeys.snapshot() });
  });

  it("calls options.onSuccess after invalidation", async () => {
    const { wrapper } = createQueryClientWrapper();
    const onSuccess = vi.fn();

    const { result } = renderHook(
      () => useReloadConfig({ onSuccess }),
      { wrapper },
    );

    await result.current.mutateAsync();

    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalled();
    });
  });
});
