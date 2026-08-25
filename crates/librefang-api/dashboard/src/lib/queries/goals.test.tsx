import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { getGoalRun } from "../http/client";
import { createQueryClientWrapper } from "../test/query-client";
import { goalQueries, useGoalRun } from "./goals";

vi.mock("../http/client", () => ({
  listGoals: vi.fn(),
  listGoalTemplates: vi.fn(),
  getGoalRun: vi.fn(),
}));

describe("goal run queries", () => {
  it("cannot be enabled without a goal id", () => {
    const { wrapper } = createQueryClientWrapper();

    renderHook(() => useGoalRun("", { enabled: true }), { wrapper });

    expect(getGoalRun).not.toHaveBeenCalled();
  });

  it("polls only while the run is active", () => {
    const refetchInterval = goalQueries.run("goal-1").refetchInterval;

    expect(typeof refetchInterval).toBe("function");
    if (typeof refetchInterval !== "function") return;

    expect(refetchInterval({ state: { data: { running: true } } } as never)).toBe(4_000);
    expect(refetchInterval({ state: { data: { running: false } } } as never)).toBe(false);
    expect(refetchInterval({ state: { data: undefined } } as never)).toBe(false);
  });
});
