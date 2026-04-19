import { describe, it, expect, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useRunSchedule } from "./schedules";
import { useSetConfigValue, useReloadConfig } from "./config";
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
  it("invalidates configKeys.all", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSetConfigValue(), { wrapper });

    await result.current.mutateAsync({ path: "kernel.max_agents", value: 10 });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });
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
