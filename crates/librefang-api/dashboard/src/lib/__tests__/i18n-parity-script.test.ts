import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const modulePath = "../../../scripts/i18n-parity.mjs";

interface ParityScript {
  flatten: (node: unknown) => string[];
  loadFlat: (file: string, localesDir?: string) => Set<string>;
}

const parity = await import(/* @vite-ignore */ modulePath) as ParityScript;

describe("i18n parity script", () => {
  it("walks arrays by index instead of collapsing their shape", () => {
    expect(parity.flatten({ messages: ["first", { label: "second" }] })).toEqual([
      "messages[0]",
      "messages[1].label",
    ]);
    expect(parity.flatten({ messages: [] })).toEqual(["messages[]"]);
    expect(parity.flatten({ messages: ["first"] })).not.toEqual(
      parity.flatten({ messages: { "0": "first" } }),
    );
    expect(parity.flatten({ messages: [] })).not.toEqual(
      parity.flatten({ messages: "first" }),
    );
  });

  it("rejects malformed locale roots", () => {
    expect(() => parity.flatten([])).toThrow("Locale root must be a JSON object");
    expect(() => parity.flatten("translation")).toThrow("Locale root must be a JSON object");
    expect(() => parity.flatten(null)).toThrow("Locale root must be a JSON object");
  });

  it("adds file context to read and JSON parse failures", () => {
    const dir = mkdtempSync(join(tmpdir(), "librefang-i18n-parity-"));
    writeFileSync(join(dir, "broken.json"), "{not-json", "utf8");
    expect(() => parity.loadFlat("broken.json", dir)).toThrow("Failed to load locale broken.json");
    expect(() => parity.loadFlat("missing.json", dir)).toThrow("Failed to load locale missing.json");
  });
});
