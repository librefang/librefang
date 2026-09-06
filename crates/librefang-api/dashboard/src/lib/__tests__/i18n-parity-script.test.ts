import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";

const modulePath = "../../../scripts/i18n-parity.mjs";

interface ParityScript {
  runParity: (localesDir?: string) => number;
  flatten: (node: unknown) => string[];
  compareKeys: (reference: Set<string>, locale: Set<string>, tag: string) => {
    missing: string[]; extra: string[]; missingPlural: string[];
  };
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
    try {
      writeFileSync(join(dir, "broken.json"), "{not-json", "utf8");
      expect(() => parity.loadFlat("broken.json", dir)).toThrow("Failed to load locale broken.json");
      expect(() => parity.loadFlat("missing.json", dir)).toThrow("Failed to load locale missing.json");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});


describe("plural-aware parity", () => {
  const reference = new Set(["title", "nested.count_one", "nested.count_other"]);

  it.each<[string, string[]]>([
    ["ko", ["other"]],
    ["zh", ["other"]],
    ["pl", ["one", "few", "many", "other"]],
    ["uk", ["one", "few", "many", "other"]],
    ["ar", ["zero", "one", "two", "few", "many", "other"]],
  ])("accepts the required cardinal forms for %s", (tag, categories) => {
    const locale = new Set(["title", ...categories.map((c) => `nested.count_${c}`)]);
    expect(parity.compareKeys(reference, locale, tag)).toEqual({
      missing: [], extra: [], missingPlural: [],
    });
    for (const category of categories) {
      const incomplete = new Set(locale);
      incomplete.delete(`nested.count_${category}`);
      expect(parity.compareKeys(reference, incomplete, tag).missingPlural)
        .toContain(`nested.count_${category}`);
    }
  });

  it("retains ordinary key drift and tolerates unused plural forms", () => {
    expect(parity.compareKeys(reference, new Set([
      "typo", "nested.count_other", "nested.count_one",
    ]), "ko")).toEqual({ missing: ["title"], extra: ["typo"], missingPlural: [] });
  });

  it("reports every form when a plural family is entirely missing", () => {
    expect(parity.compareKeys(reference, new Set(["title"]), "en").missingPlural)
      .toEqual(["nested.count_one", "nested.count_other"]);
  });
});


it("returns a failing CLI status for a missing required form", () => {
  const dir = mkdtempSync(join(tmpdir(), "librefang-i18n-parity-"));
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  const error = vi.spyOn(console, "error").mockImplementation(() => {});
  try {
    writeFileSync(join(dir, "en.json"), JSON.stringify({ count_one: "one", count_other: "many" }));
    writeFileSync(join(dir, "ko.json"), JSON.stringify({ count_other: "many" }));
    expect(parity.runParity(dir)).toBe(0);
    writeFileSync(join(dir, "ko.json"), JSON.stringify({ count_one: "unused" }));
    expect(parity.runParity(dir)).toBe(1);
    expect(error).toHaveBeenCalledWith("  missing (1):", ["count_other"]);
  } finally {
    log.mockRestore();
    error.mockRestore();
    rmSync(dir, { recursive: true, force: true });
  }
});
