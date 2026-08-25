import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as http from "../http/client";
import {
  clawhubCnKeys,
  clawhubKeys,
  fanghubKeys,
  skillhubKeys,
  skillKeys,
} from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import {
  useCreateSkill,
  useEvolvePatchSkill,
  useEvolveRemoveFile,
  useEvolveRollbackSkill,
  useEvolveUpdateSkill,
  useEvolveWriteFile,
} from "./skills";

vi.mock("../http/client", () => ({
  installSkill: vi.fn(),
  uninstallSkill: vi.fn(),
  clawhubInstall: vi.fn(),
  clawhubCnInstall: vi.fn(),
  skillhubInstall: vi.fn(),
  createSkill: vi.fn(),
  reloadSkills: vi.fn(),
  evolveUpdateSkill: vi.fn(),
  evolvePatchSkill: vi.fn(),
  evolveRollbackSkill: vi.fn(),
  evolveDeleteSkill: vi.fn(),
  evolveWriteFile: vi.fn(),
  evolveRemoveFile: vi.fn(),
  proposeSkillToRegistry: vi.fn(),
  approvePendingCandidate: vi.fn(),
  rejectPendingCandidate: vi.fn(),
  proposePendingToRegistry: vi.fn(),
}));

const allSkillSurfaces = [
  skillKeys.all,
  fanghubKeys.all,
  clawhubKeys.all,
  clawhubCnKeys.all,
  skillhubKeys.all,
] as const;

describe("skill mutation caches", () => {
  beforeEach(() => {
    const result = { success: true, message: "ok", skill_name: "skill-a" };
    vi.mocked(http.createSkill).mockReset().mockResolvedValue(result);
    vi.mocked(http.evolveUpdateSkill).mockReset().mockResolvedValue(result);
    vi.mocked(http.evolvePatchSkill).mockReset().mockResolvedValue(result);
    vi.mocked(http.evolveRollbackSkill).mockReset().mockResolvedValue(result);
    vi.mocked(http.evolveWriteFile).mockReset().mockResolvedValue(result);
    vi.mocked(http.evolveRemoveFile).mockReset().mockResolvedValue(result);
  });

  it("refreshes installed state on every hub after local skill creation", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useCreateSkill(), { wrapper });

    await result.current.mutateAsync({
      name: "new-skill",
      description: "description",
      prompt_context: "context",
    });

    for (const queryKey of allSkillSurfaces) {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey });
    }
  });

  it.each([
    ["update", useEvolveUpdateSkill, { name: "skill-a", params: { prompt_context: "new", changelog: "update" } }],
    ["patch", useEvolvePatchSkill, { name: "skill-a", params: { old_string: "old", new_string: "new", changelog: "patch", replace_all: false } }],
    ["rollback", useEvolveRollbackSkill, { name: "skill-a" }],
  ] as const)("refreshes detail, list, and file tree after evolve %s", async (_label, hook, variables) => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => hook(), { wrapper });

    await result.current.mutateAsync(variables as never);

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: skillKeys.detail("skill-a") });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: skillKeys.lists() });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: skillKeys.supportingFiles("skill-a") });
  });

  it("refreshes the file tree and written file", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useEvolveWriteFile(), { wrapper });

    await result.current.mutateAsync({
      name: "skill-a",
      params: { path: "references/new.md", content: "new" },
    });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: skillKeys.supportingFiles("skill-a") });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: skillKeys.supportingFile("skill-a", "references/new.md"),
    });
  });

  it("refreshes the file tree and removes the deleted file cache", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const removeSpy = vi.spyOn(queryClient, "removeQueries");
    const { result } = renderHook(() => useEvolveRemoveFile(), { wrapper });

    await result.current.mutateAsync({ name: "skill-a", path: "references/old.md" });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: skillKeys.supportingFiles("skill-a") });
    expect(removeSpy).toHaveBeenCalledWith({
      queryKey: skillKeys.supportingFile("skill-a", "references/old.md"),
    });
  });
});
