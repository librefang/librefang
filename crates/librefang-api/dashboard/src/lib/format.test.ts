import { describe, expect, it } from "vitest";
import { NUMBER_LOCALE, formatBytes, formatCompact, formatCost, formatNumber } from "./format";

// Built against the same pinned locale the formatters use.
// This used to pass `undefined`, which made both sides follow the environment: the assertion and the implementation drifted together, so the test agreed with itself no matter what and asserted nothing about the output an operator sees (#8156).
const oneDecimal = new Intl.NumberFormat(NUMBER_LOCALE, {
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
});

describe("numeric formatters", () => {
  // Literal expectations, not `Intl`-derived ones.
  // Everything else in this file builds its expected value from the same API the implementation uses, which catches a wrong tier or a dropped sign but cannot catch the whole output shifting with the environment — the failure #8156 was about.
  // These are the assertions that fail if the pinned locale is removed.
  it("groups thousands the same way regardless of the environment locale", () => {
    expect(formatNumber(2_000)).toBe("2,000");
    expect(formatNumber(1_234_567)).toBe("1,234,567");
    expect(formatCompact(1_500)).toBe("1.5K");
    expect(formatCompact(2_000_000)).toBe("2.0M");
  });

  it("treats a nullish count as zero", () => {
    expect(formatNumber(null)).toBe("0");
    expect(formatNumber(undefined)).toBe("0");
    expect(formatNumber(0)).toBe("0");
  });

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
