import { describe, expect, it } from "vitest";
import { terminalQueries } from "./terminal";

describe("terminal query policy", () => {
  it("uses distinct health freshness and live-window polling cadences", () => {
    const health = terminalQueries.health();
    const windows = terminalQueries.windows();

    expect(health.staleTime).toBe(60_000);
    expect(health.refetchInterval).toBeUndefined();
    expect(windows.staleTime).toBe(10_000);
    expect(windows.refetchInterval).toBe(windows.staleTime);
    expect(windows.refetchIntervalInBackground).toBe(false);
  });
});
