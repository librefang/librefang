import { describe, expect, it } from "vitest";
import { formatBytes, formatCompact, formatCost } from "./format";

const oneDecimal = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
});

describe("numeric formatters", () => {
  it("compacts positive and negative values with locale decimals", () => {
    expect(formatCompact(1_500)).toBe(`${oneDecimal.format(1.5)}K`);
    expect(formatCompact(-1_500)).toBe(`${oneDecimal.format(-1.5)}K`);
  });

  it("promotes values that round across a compact tier", () => {
    expect(formatCompact(999_999)).toBe(`${oneDecimal.format(1)}M`);
    expect(formatCompact(-999_950_000)).toBe(`${oneDecimal.format(-1)}B`);
  });

  it.each([Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY])(
    "uses a safe fallback for non-finite value %s",
    (value) => {
      expect(formatCompact(value)).toBe("—");
      expect(formatCost(value)).toBe("—");
      expect(formatBytes(value)).toBe("—");
    },
  );

  it("formats negative costs with the sign before the currency marker", () => {
    expect(formatCost(-0.005)).toBe("-$0.0050");
    expect(formatCost(-1.5)).toBe("-$1.50");
  });

  it("clamps negative byte counts to zero", () => {
    expect(formatBytes(-5)).toBe("0 B");
  });
});
