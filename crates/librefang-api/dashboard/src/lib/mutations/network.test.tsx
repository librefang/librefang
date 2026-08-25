import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import * as http from "../http/client";
import { a2aKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import { useDiscoverA2AAgent, useSendA2ATask } from "./network";

vi.mock("../http/client", () => ({
  discoverA2AAgent: vi.fn().mockResolvedValue({}),
  sendA2ATask: vi.fn().mockResolvedValue({ task_id: "task-1" }),
}));

describe("A2A mutations", () => {
  it("invalidates discovered agents", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useDiscoverA2AAgent(), { wrapper });

    await result.current.mutateAsync("https://agent.example");

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: a2aKeys.agents() });
  });

  it("routes task sends through the shared HTTP client hook", async () => {
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useSendA2ATask(), { wrapper });
    const payload = { agent_url: "https://agent.example", message: "run" };

    await expect(result.current.mutateAsync(payload)).resolves.toEqual({ task_id: "task-1" });
    expect(http.sendA2ATask).toHaveBeenCalledWith(payload, expect.any(Object));
  });
});
