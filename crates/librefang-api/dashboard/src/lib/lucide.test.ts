import { describe, expect, it } from "vitest";
import * as icons from "./lucide";

describe("curated Lucide entrypoint", () => {
  it("resolves every version-pinned icon mapping", () => {
    expect(Object.keys(icons).length).toBeGreaterThan(150);
    for (const [name, icon] of Object.entries(icons)) {
      expect(icon, name).toBeDefined();
    }
  });
});
