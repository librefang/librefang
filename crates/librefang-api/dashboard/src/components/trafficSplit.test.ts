import { describe, expect, it } from "vitest";
import { buildEvenTrafficSplit } from "./trafficSplit";

describe("buildEvenTrafficSplit", () => {
  it("covers all 100 traffic buckets", () => {
    expect(buildEvenTrafficSplit(3)).toEqual([34, 33, 33]);
    expect(buildEvenTrafficSplit(6)).toEqual([17, 17, 17, 17, 16, 16]);
  });

  it("keeps distribution fair when 100 divides evenly", () => {
    expect(buildEvenTrafficSplit(4)).toEqual([25, 25, 25, 25]);
  });

  it("returns no split for empty input", () => {
    expect(buildEvenTrafficSplit(0)).toEqual([]);
  });
});
