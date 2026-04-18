import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import * as http from "../http/client";
import type { ApiActionResponse, ScheduleItem } from "../../api";
import { useCreateSchedule, useUpdateSchedule, useDeleteSchedule, useUpdateTrigger, useDeleteTrigger } from "./schedules";
import { cronKeys, scheduleKeys, triggerKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  createSchedule: vi.fn(),
  updateSchedule: vi.fn(),
  deleteSchedule: vi.fn(),
  updateTrigger: vi.fn(),
  deleteTrigger: vi.fn(),
}));

const actionResponse: ApiActionResponse = { status: "ok" };
const scheduleResponse: ScheduleItem = {
  id: "sched-1",
  name: "test schedule",
  cron: "0 * * * *",
  agent_id: "agent-1",
  enabled: true,
};

describe("useCreateSchedule", () => {
  beforeEach(() => {
    vi.mocked(http.createSchedule).mockResolvedValue(scheduleResponse);
  });

  it("invalidates scheduleKeys.all and cronKeys.all", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useCreateSchedule(), { wrapper });

    result.current.mutate({ name: "test schedule", agent_id: "agent-1", cron: "0 * * * *" });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(2);
    });
    expect(invalidateSpy).toHaveBeenNthCalledWith(1, {
      queryKey: scheduleKeys.all,
    });
    expect(invalidateSpy).toHaveBeenNthCalledWith(2, {
      queryKey: cronKeys.all,
    });
  });
});

describe("useUpdateSchedule", () => {
  beforeEach(() => {
    vi.mocked(http.updateSchedule).mockResolvedValue(actionResponse);
  });

  it("invalidates scheduleKeys.all and cronKeys.all", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useUpdateSchedule(), { wrapper });

    result.current.mutate({ id: "sched-1", data: { enabled: false } });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(2);
    });
    expect(invalidateSpy).toHaveBeenNthCalledWith(1, {
      queryKey: scheduleKeys.all,
    });
    expect(invalidateSpy).toHaveBeenNthCalledWith(2, {
      queryKey: cronKeys.all,
    });
  });
});

describe("useDeleteSchedule", () => {
  beforeEach(() => {
    vi.mocked(http.deleteSchedule).mockResolvedValue(actionResponse);
  });

  it("invalidates scheduleKeys.all and cronKeys.all", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useDeleteSchedule(), { wrapper });

    result.current.mutate("sched-1");

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledTimes(2);
    });
    expect(invalidateSpy).toHaveBeenNthCalledWith(1, {
      queryKey: scheduleKeys.all,
    });
    expect(invalidateSpy).toHaveBeenNthCalledWith(2, {
      queryKey: cronKeys.all,
    });
  });
});

describe("useUpdateTrigger", () => {
  beforeEach(() => {
    vi.mocked(http.updateTrigger).mockResolvedValue(actionResponse);
  });

  it("invalidates triggerKeys.all", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useUpdateTrigger(), { wrapper });

    await result.current.mutateAsync({ id: "trig-1", data: { enabled: true } });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: triggerKeys.all,
    });
  });
});

describe("useDeleteTrigger", () => {
  beforeEach(() => {
    vi.mocked(http.deleteTrigger).mockResolvedValue(actionResponse);
  });

  it("invalidates triggerKeys.all", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useDeleteTrigger(), { wrapper });

    await result.current.mutateAsync("trig-1");

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: triggerKeys.all,
    });
  });
});
