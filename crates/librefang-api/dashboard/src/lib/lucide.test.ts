import { describe, expect, it } from "vitest";
import packageJson from "../../package.json";
import * as icons from "./lucide";

describe("curated Lucide entrypoint", () => {
  it("keeps the internal-path dependency pinned to an exact version", () => {
    expect(packageJson.dependencies["lucide-react"]).toMatch(/^\d+\.\d+\.\d+$/);
  });

  it("resolves every version-pinned icon mapping", () => {
    expect(Object.keys(icons).length).toBeGreaterThan(150);
    for (const [name, icon] of Object.entries(icons)) {
      expect(icon, name).toBeDefined();
    }
  });
});
