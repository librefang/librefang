import { describe, it, expect, vi, type MockedFunction } from "vitest";
import { renderHook } from "@testing-library/react";
import {
  useRestoreBackup,
  useCreateBackup,
  useDeleteBackup,
  useDeleteTask,
  useRetryTask,
  useCleanupSessions,
  useShutdownServer,
} from "./runtime";
import {
  useFangHubInstall,
  useUninstallSkill,
  useClawHubInstall,
  useSkillHubInstall,
} from "./skills";
import { runtimeKeys, overviewKeys, skillKeys, fanghubKeys, sessionKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../../api", () => ({
  restoreBackup: vi.fn().mockResolvedValue({ message: "ok" }),
  createBackup: vi.fn().mockResolvedValue({ message: "ok" }),
  deleteBackup: vi.fn().mockResolvedValue({ message: "ok" }),
  deleteTaskFromQueue: vi.fn().mockResolvedValue({ message: "ok" }),
  retryTask: vi.fn().mockResolvedValue({ message: "ok" }),
  cleanupSessions: vi.fn().mockResolvedValue({ message: "ok" }),
  shutdownServer: vi.fn().mockResolvedValue({ status: "ok" }),
}));

vi.mock("../http/client", () => ({
  installSkill: vi.fn().mockResolvedValue({ status: "ok" }),
  clawhubInstall: vi.fn().mockResolvedValue({ status: "ok" }),
  skillhubInstall: vi.fn().mockResolvedValue({ status: "ok" }),
  uninstallSkill: vi.fn().mockResolvedValue({ status: "ok" }),
}));

describe("useRestoreBackup", () => {
  it("invalidates runtimeKeys.backups() and overviewKeys.snapshot()", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useRestoreBackup(), { wrapper });

    result.current.mutate("backup-1");
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: runtimeKeys.backups(),
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: overviewKeys.snapshot(),
    });
  });
});

describe("useFangHubInstall", () => {
  it("invalidates skillKeys.all and fanghubKeys.all", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useFangHubInstall(), { wrapper });

    result.current.mutate({ name: "test-skill" });
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: skillKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: fanghubKeys.all,
    });
  });

  it("invalidates skillKeys.all and fanghubKeys.all with hand parameter", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useFangHubInstall(), { wrapper });

    result.current.mutate({ name: "test-skill", hand: "test-hand" });
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: skillKeys.all,
    });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: fanghubKeys.all,
    });
  });
});

describe.each([
  { name: "useCreateBackup", hook: useCreateBackup, mutator: () => undefined, invalidateKeys: [runtimeKeys.backups()] },
  { name: "useDeleteBackup", hook: useDeleteBackup, mutator: () => "backup-1", invalidateKeys: [runtimeKeys.backups()] },
] as const)("$name", ({ hook, mutator, invalidateKeys }) => {
  it("invalidates correct keys", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const typedHook = hook as MockedFunction<typeof hook>;
    const { result } = renderHook(() => typedHook(), { wrapper });

    result.current.mutate(mutator() as never);
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    for (const key of invalidateKeys) {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: key });
    }
  });
});

describe.each([
  { name: "useDeleteTask", hook: useDeleteTask, id: "task-1", invalidateKeys: [runtimeKeys.tasks()] },
  { name: "useRetryTask", hook: useRetryTask, id: "task-2", invalidateKeys: [runtimeKeys.tasks()] },
] as const)("$name", ({ hook, id, invalidateKeys }) => {
  it("invalidates correct keys", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => hook(), { wrapper });

    result.current.mutate(id);
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    for (const key of invalidateKeys) {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: key });
    }
  });
});

describe("useCleanupSessions", () => {
  it("invalidates sessionKeys.all", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useCleanupSessions(), { wrapper });

    result.current.mutate();
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: sessionKeys.all,
    });
  });
});

describe("useShutdownServer", () => {
  it("calls shutdownServer without invalidating queries", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useShutdownServer(), { wrapper });

    result.current.mutate();
    await vi.waitFor(() => {
      expect(result.current.isSuccess).toBe(true);
    });

    expect(invalidateSpy).not.toHaveBeenCalled();
  });
});

describe.each([
  { name: "useUninstallSkill", hook: useUninstallSkill, input: "skill-1" },
  { name: "useClawHubInstall", hook: useClawHubInstall, input: { slug: "test-skill", version: "1.0.0", hand: "test-hand" } },
  { name: "useSkillHubInstall", hook: useSkillHubInstall, input: { slug: "test-skill", hand: "test-hand" } },
] as const)("$name", ({ hook, input }) => {
  it("invalidates skillKeys.all", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => hook(), { wrapper });

    result.current.mutate(input as any);
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: skillKeys.all,
    });
  });
});
