import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as http from "../http/client";
import { agentKeys, promptsKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import { PromptBindRollbackError, useBindPromptVersionToAgent } from "./prompts";

vi.mock("../http/client", () => ({
  createPromptVersion: vi.fn(),
  deletePromptVersion: vi.fn(),
  activatePromptVersion: vi.fn(),
  patchAgent: vi.fn(),
}));

const version = {
  id: "version-2",
  agent_id: "agent-1",
  version: 2,
  content_hash: "hash",
  system_prompt: "new prompt",
  tools: [],
  variables: [],
  created_at: "2026-01-01T00:00:00Z",
  created_by: "dashboard",
  is_active: false,
};

describe("useBindPromptVersionToAgent", () => {
  beforeEach(() => {
    vi.mocked(http.patchAgent).mockReset().mockResolvedValue({});
    vi.mocked(http.activatePromptVersion).mockReset().mockResolvedValue(version);
  });

  it("restores the previous live prompt when activation fails", async () => {
    const activationError = new Error("activation failed");
    vi.mocked(http.activatePromptVersion).mockRejectedValueOnce(activationError);
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useBindPromptVersionToAgent(), { wrapper });

    await expect(result.current.mutateAsync({
      agentId: "agent-1",
      version,
      previousSystemPrompt: "old prompt",
    })).rejects.toBe(activationError);

    expect(http.patchAgent).toHaveBeenNthCalledWith(1, "agent-1", {
      system_prompt: "new prompt",
    });
    expect(http.patchAgent).toHaveBeenNthCalledWith(2, "agent-1", {
      system_prompt: "old prompt",
    });
  });

  it("reports both failures when compensating rollback also fails", async () => {
    const activationError = new Error("activation failed");
    const rollbackError = new Error("rollback failed");
    vi.mocked(http.activatePromptVersion).mockRejectedValueOnce(activationError);
    vi.mocked(http.patchAgent)
      .mockResolvedValueOnce({})
      .mockRejectedValueOnce(rollbackError);
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useBindPromptVersionToAgent(), { wrapper });

    const failure = await result.current.mutateAsync({
      agentId: "agent-1",
      version,
      previousSystemPrompt: "old prompt",
    }).catch((error: unknown) => error);

    expect(failure).toBeInstanceOf(PromptBindRollbackError);
    expect(failure).toMatchObject({ activationError, rollbackError });
  });

  it("reconciles prompt and agent caches after a partial failure", async () => {
    vi.mocked(http.activatePromptVersion).mockRejectedValueOnce(new Error("activation failed"));
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useBindPromptVersionToAgent(), { wrapper });

    await expect(result.current.mutateAsync({
      agentId: "agent-1",
      version,
      previousSystemPrompt: "old prompt",
    })).rejects.toThrow("activation failed");

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: agentKeys.promptVersions("agent-1") });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: promptsKeys.list() });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: promptsKeys.details() });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: agentKeys.detail("agent-1") });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: agentKeys.lists() });
  });
});
