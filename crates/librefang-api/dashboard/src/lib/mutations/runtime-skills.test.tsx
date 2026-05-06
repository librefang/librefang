import { describe, it, expect, vi } from "vitest";
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
  useClawHubCnInstall,
  useSkillHubInstall,
  useReloadSkills,
} from "./skills";
import {
  runtimeKeys,
  overviewKeys,
  skillKeys,
  fanghubKeys,
  clawhubKeys,
  clawhubCnKeys,
  skillhubKeys,
  sessionKeys,
} from "../queries/keys";
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
  clawhubCnInstall: vi.fn().mockResolvedValue({ status: "ok" }),
  skillhubInstall: vi.fn().mockResolvedValue({ status: "ok" }),
  uninstallSkill: vi.fn().mockResolvedValue({ status: "ok" }),
  reloadSkills: vi.fn().mockResolvedValue({ status: "ok" }),
  createSkill: vi.fn().mockResolvedValue({ status: "ok" }),
  evolveUpdateSkill: vi.fn().mockResolvedValue({ status: "ok" }),
  evolvePatchSkill: vi.fn().mockResolvedValue({ status: "ok" }),
  evolveRollbackSkill: vi.fn().mockResolvedValue({ status: "ok" }),
  evolveDeleteSkill: vi.fn().mockResolvedValue({ status: "ok" }),
  evolveWriteFile: vi.fn().mockResolvedValue({ status: "ok" }),
  evolveRemoveFile: vi.fn().mockResolvedValue({ status: "ok" }),
}));

// Every install / uninstall / reload mutation must invalidate every hub
// surface — see #4689. Keep this list in sync with
// `invalidateAllSkillSurfaces` in mutations/skills.ts.
const ALL_SKILL_SURFACE_KEYS = [
  skillKeys.all,
  fanghubKeys.all,
  clawhubKeys.all,
  clawhubCnKeys.all,
  skillhubKeys.all,
] as const;

function expectAllSurfacesInvalidated(spy: ReturnType<typeof vi.spyOn>) {
  for (const key of ALL_SKILL_SURFACE_KEYS) {
    expect(spy).toHaveBeenCalledWith({ queryKey: key });
  }
}

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
  it("invalidates every skill surface (#4689)", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useFangHubInstall(), { wrapper });

    result.current.mutate({ name: "test-skill" });
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    expectAllSurfacesInvalidated(invalidateSpy);
  });

  it("invalidates every skill surface with hand parameter", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useFangHubInstall(), { wrapper });

    result.current.mutate({ name: "test-skill", hand: "test-hand" });
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    expectAllSurfacesInvalidated(invalidateSpy);
  });
});

describe("useCreateBackup", () => {
  it("invalidates correct keys", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useCreateBackup(), { wrapper });

    result.current.mutate();
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: runtimeKeys.backups() });
  });
});

describe("useDeleteBackup", () => {
  it("invalidates correct keys", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useDeleteBackup(), { wrapper });

    result.current.mutate("backup-1");
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: runtimeKeys.backups() });
  });
});

describe.each([
  {
    name: "useDeleteTask",
    hook: useDeleteTask,
    id: "task-1",
    invalidateKeys: [runtimeKeys.tasks(), runtimeKeys.taskStatus(), runtimeKeys.queueStatus()],
  },
  {
    name: "useRetryTask",
    hook: useRetryTask,
    id: "task-2",
    invalidateKeys: [runtimeKeys.tasks(), runtimeKeys.taskStatus(), runtimeKeys.queueStatus()],
  },
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

describe("useUninstallSkill", () => {
  it("invalidates every skill surface (#4689)", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useUninstallSkill(), { wrapper });

    result.current.mutate("skill-1");
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    expectAllSurfacesInvalidated(invalidateSpy);
  });
});

describe("useClawHubInstall", () => {
  it("invalidates every skill surface (#4689)", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useClawHubInstall(), { wrapper });

    result.current.mutate({ slug: "test-skill", version: "1.0.0", hand: "test-hand" });
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    expectAllSurfacesInvalidated(invalidateSpy);
  });
});

describe("useClawHubCnInstall", () => {
  it("invalidates every skill surface (#4689)", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useClawHubCnInstall(), { wrapper });

    result.current.mutate({ slug: "test-skill", version: "1.0.0" });
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    expectAllSurfacesInvalidated(invalidateSpy);
  });
});

describe("useSkillHubInstall", () => {
  it("invalidates every skill surface (#4689)", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useSkillHubInstall(), { wrapper });

    result.current.mutate({ slug: "test-skill", hand: "test-hand" });
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    expectAllSurfacesInvalidated(invalidateSpy);
  });
});

describe("useReloadSkills", () => {
  it("invalidates every skill surface (#4689)", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useReloadSkills(), { wrapper });

    result.current.mutate();
    await vi.waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalled();
    });

    expectAllSurfacesInvalidated(invalidateSpy);
  });
});
