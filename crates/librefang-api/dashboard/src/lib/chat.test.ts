import { describe, expect, it } from "vitest";
import { asText, formatMeta, normalizeRole } from "./chat";

describe("chat utilities", () => {
  it("normalizes API message roles", () => {
    expect(normalizeRole("User")).toBe("user");
    expect(normalizeRole("System")).toBe("system");
    expect(normalizeRole("Assistant")).toBe("assistant");
  });

  it("converts unknown values to text", () => {
    expect(asText("hello")).toBe("hello");
    expect(asText({ ok: true })).toContain('"ok": true');
  });

  it("formats usage metadata", () => {
    expect(
      formatMeta({
        input_tokens: 12,
        output_tokens: 34,
        iterations: 2,
        cost_usd: 0.00123
      })
    ).toBe("12 in / 34 out | 2 iter | $0.0012");
  });
});
