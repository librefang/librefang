import { describe, it, expect, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import { useRunSchedule } from "./schedules";
import { useSetConfigValue, useReloadConfig } from "./config";
import { scheduleKeys, cronKeys, configKeys, overviewKeys } from "../queries/keys";

vi.mock("../http/client", () => ({
  runSchedule: vi.fn().mockResolvedValue({}),
  setConfigValue: vi.fn().mockResolvedValue({}),
  reloadConfig: vi.fn().mockResolvedValue({}),
}));

describe("useRunSchedule", () => {
  it("invalidates scheduleKeys.all and cronKeys.all", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const spy = vi.spyOn(queryClient, "invalidateQueries");

    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => useRunSchedule(), { wrapper });

    await result.current.mutateAsync("schedule-1");

    await waitFor(() => {
      expect(spy).toHaveBeenCalled();
    });
    expect(spy).toHaveBeenCalledWith({ queryKey: scheduleKeys.all });
    expect(spy).toHaveBeenCalledWith({ queryKey: cronKeys.all });
  });
});

describe("useSetConfigValue", () => {
  it("invalidates configKeys.all", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const spy = vi.spyOn(queryClient, "invalidateQueries");

    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => useSetConfigValue(), { wrapper });

    await result.current.mutateAsync({ path: "kernel.max_agents", value: 10 });

    await waitFor(() => {
      expect(spy).toHaveBeenCalled();
    });
    expect(spy).toHaveBeenCalledWith({ queryKey: configKeys.all });
  });

  it("calls options.onSuccess after invalidation", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const onSuccess = vi.fn();

    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

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
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const spy = vi.spyOn(queryClient, "invalidateQueries");

    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => useReloadConfig(), { wrapper });

    await result.current.mutateAsync();

    await waitFor(() => {
      expect(spy).toHaveBeenCalled();
    });
    expect(spy).toHaveBeenCalledWith({ queryKey: configKeys.all });
    expect(spy).toHaveBeenCalledWith({ queryKey: overviewKeys.snapshot() });
  });

  it("calls options.onSuccess after invalidation", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const onSuccess = vi.fn();

    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

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
