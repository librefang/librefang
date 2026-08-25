import { describe, expect, it, vi } from "vitest";
import { formatHours, formatKvValue, formatRelativeMs } from "./formatters";

describe("formatRelativeMs", () => {
  const never = () => "never";
  const justNow = () => "localized just now";
  const rtf = new Intl.RelativeTimeFormat("en", { numeric: "auto", style: "narrow" });

  it("uses localized callbacks for sentinel and near-current timestamps", () => {
    const neverSpy = vi.fn(never);
    const justNowSpy = vi.fn(justNow);

    expect(formatRelativeMs(undefined, 1_000, "en", neverSpy, justNowSpy)).toBe("never");
    expect(formatRelativeMs(1_001, 1_000, "en", neverSpy, justNowSpy)).toBe(
      "localized just now",
    );
    expect(neverSpy).toHaveBeenCalledOnce();
    expect(justNowSpy).toHaveBeenCalledOnce();
  });

  it("rolls rounded minute values into hours instead of emitting 60 minutes", () => {
    const delta = 59.6 * 60_000;

    expect(formatRelativeMs(1_000 + delta, 1_000, "en", never, justNow)).toBe(
      rtf.format(1, "hour"),
    );
    expect(formatRelativeMs(1_000, 1_000 + delta, "en", never, justNow)).toBe(
      rtf.format(-1, "hour"),
    );
  });

  it("rolls rounded hour values into days", () => {
    const delta = 23.96 * 3_600_000;

    expect(formatRelativeMs(1_000 + delta, 1_000, "en", never, justNow)).toBe(
      rtf.format(1, "day"),
    );
  });
});

describe("formatHours", () => {
  const units = { minute: "m", hour: "h", day: "d", week: "w" };

  it("treats tiny floating-point residuals as whole units", () => {
    expect(formatHours(2 + 5e-10, units)).toBe("2h");
    expect(formatHours(48 + 5e-10, units)).toBe("2d");
    expect(formatHours(168 + 5e-10, units)).toBe("1w");
  });

  it("retains meaningful fractional units", () => {
    expect(formatHours(1.25, units)).toBe("1.3h");
    expect(formatHours(36, units)).toBe("1.5d");
    expect(formatHours(252, units)).toBe("1.5w");
  });
});

describe("formatKvValue", () => {
  it("serializes values without applying presentation truncation", () => {
    const value = "x".repeat(500);

    expect(formatKvValue(value)).toBe(value);
    expect(formatKvValue({ nested: true })).toBe('{"nested":true}');
  });
});
