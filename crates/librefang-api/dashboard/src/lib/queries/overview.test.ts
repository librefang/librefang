import { describe, expect, it } from "vitest";
import {
  dashboardSnapshotQueryOptions,
  versionInfoQueryOptions,
} from "./overview";

describe("overview query policy", () => {
  it("keeps snapshot freshness aligned with its foreground poll", () => {
    const options = dashboardSnapshotQueryOptions();

    expect(options.staleTime).toBe(5_000);
    expect(options.refetchInterval).toBe(options.staleTime);
    expect(options.refetchIntervalInBackground).toBe(false);
  });

  it("retains version data but eventually permits focus/remount refresh", () => {
    const options = versionInfoQueryOptions();

    expect(options.staleTime).toBe(5 * 60_000);
    expect(options.gcTime).toBe(Infinity);
    expect(options.refetchInterval).toBeUndefined();
    expect(options.refetchOnWindowFocus).toBe(true);
  });
});
