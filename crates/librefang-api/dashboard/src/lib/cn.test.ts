import { describe, expect, it } from "vitest";

import { cn } from "./cn";

describe("cn", () => {
  it("lets later Tailwind utilities override defaults", () => {
    expect(cn("mb-2.5 text-label", "mb-0")).toBe("text-label mb-0");
  });

  it("supports conditional class values", () => {
    expect(cn("flex", false, { rounded: true })).toBe("flex rounded");
  });

  it("keeps the custom label size alongside a text color", () => {
    expect(cn("text-label text-text-dim")).toBe("text-label text-text-dim");
    expect(cn("text-xs text-label text-text-dim")).toBe("text-label text-text-dim");
  });
});
