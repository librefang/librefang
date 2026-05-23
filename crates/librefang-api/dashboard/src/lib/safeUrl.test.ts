import { describe, it, expect } from "vitest";
import { safeUrl } from "./safeUrl";

describe("safeUrl", () => {
  it.each([
    "javascript:alert(1)",
    "JavaScript:alert(1)",
    "  javascript:alert(1)  ",
    "data:text/html,<script>alert(1)</script>",
    "vbscript:msgbox(1)",
    "file:///etc/passwd",
    "JavaScript:void(0)",
  ])("rejects dangerous scheme %s", input => {
    expect(safeUrl(input)).toBeNull();
  });

  it.each(["http://example.com", "https://example.com/path?q=1#h", "mailto:user@example.com"])(
    "accepts safe scheme %s",
    input => {
      expect(safeUrl(input)).toBe(input);
    },
  );

  it("accepts protocol-relative URLs (synthetic https base)", () => {
    expect(safeUrl("//example.com/x")).toBe("//example.com/x");
  });

  it("rejects empty / whitespace / null / undefined", () => {
    expect(safeUrl("")).toBeNull();
    expect(safeUrl("   ")).toBeNull();
    expect(safeUrl(null)).toBeNull();
    expect(safeUrl(undefined)).toBeNull();
  });

  it("rejects malformed input that the URL parser cannot resolve", () => {
    expect(safeUrl("not a url")).toBeNull();
    // Relative paths have no scheme — we can't tell whether they
    // resolve to a safe origin without a base, so reject.
    expect(safeUrl("/path")).toBeNull();
  });

  it("rejects unicode-encoded variants of dangerous schemes", () => {
    // `javascript:` written with a leading-tab; some historical XSS
    // bypasses relied on browsers stripping control chars from the
    // scheme. The URL constructor canonicalises these to javascript:
    // so our scheme check still catches them.
    expect(safeUrl("\tjavascript:alert(1)")).toBeNull();
    expect(safeUrl("\u0000javascript:alert(1)")).toBeNull();
  });
});
