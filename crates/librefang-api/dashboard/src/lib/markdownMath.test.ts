import { describe, expect, it } from "vitest";
import { containsMathDelimiters } from "./markdownMath";

// Issue #3381: `containsMathDelimiters` gates the lazy KaTeX import in
// chat rendering. False negatives drop math from the rendered output;
// false positives merely fetch ~280KB we didn't need. The detector
// must mirror the delimiter set `remark-math` recognises by default:
// `$...$`, `$$...$$`, `\(...\)`, and `\[...\]`.
describe("containsMathDelimiters", () => {
  it("returns false for empty / nullish input", () => {
    expect(containsMathDelimiters("")).toBe(false);
    expect(containsMathDelimiters(null)).toBe(false);
    expect(containsMathDelimiters(undefined)).toBe(false);
  });

  it("returns false for plain prose without delimiters", () => {
    expect(containsMathDelimiters("hello world")).toBe(false);
    expect(containsMathDelimiters("a price tag of 5 dollars")).toBe(false);
  });

  it("detects inline `$...$` math", () => {
    expect(containsMathDelimiters("the answer is $x = 42$ today")).toBe(true);
  });

  it("detects display `$$...$$` math", () => {
    expect(containsMathDelimiters("derivation:\n$$\nE = mc^2\n$$\n")).toBe(true);
  });

  it("detects `\\(...\\)` inline form", () => {
    expect(containsMathDelimiters("we know \\(a^2 + b^2 = c^2\\) holds")).toBe(true);
  });

  it("detects `\\[...\\]` display form", () => {
    expect(containsMathDelimiters("\\[ \\int_0^1 x\\,dx = 1/2 \\]")).toBe(true);
  });

  it("does not match a single stray `$`", () => {
    // A lone dollar sign in prose ("$5") is not math — `remark-math`
    // also requires a closing delimiter on the same paragraph.
    expect(containsMathDelimiters("the cost is $5 per gallon")).toBe(false);
  });
});
