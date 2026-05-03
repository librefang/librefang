import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

// Per issue #3557: with i18next `fallbackLng: "en"`, any key present in
// `en.json` but missing from a non-English locale file silently falls back
// to English at runtime, leaving Chinese (or future) users with mid-page
// English text. This test asserts that every locale file has the exact
// same flattened key set as `en.json` — no missing keys, no dead keys.

const LOCALES_DIR = join(__dirname, "..", "..", "locales");
const REFERENCE = "en.json";

type JsonValue = string | number | boolean | null | JsonValue[] | { [k: string]: JsonValue };

function flatten(node: JsonValue, prefix = ""): string[] {
  if (node === null || typeof node !== "object" || Array.isArray(node)) {
    return [prefix];
  }
  const out: string[] = [];
  for (const [k, v] of Object.entries(node)) {
    const next = prefix ? `${prefix}.${k}` : k;
    out.push(...flatten(v, next));
  }
  return out;
}

function loadFlat(file: string): Set<string> {
  const text = readFileSync(join(LOCALES_DIR, file), "utf8");
  return new Set(flatten(JSON.parse(text) as JsonValue));
}

describe("locale parity", () => {
  const reference = loadFlat(REFERENCE);
  const others = readdirSync(LOCALES_DIR).filter(
    (f) => f.endsWith(".json") && f !== REFERENCE,
  );

  it.each(others)("%s has the same key set as en.json", (file) => {
    const locale = loadFlat(file);
    const missing = [...reference].filter((k) => !locale.has(k)).sort();
    const extra = [...locale].filter((k) => !reference.has(k)).sort();

    expect(
      { missing, extra },
      `Locale ${file} drifted from en.json.\n` +
        `Missing keys (will fall back to English at runtime): ${JSON.stringify(missing, null, 2)}\n` +
        `Extra keys (dead — no English source): ${JSON.stringify(extra, null, 2)}`,
    ).toEqual({ missing: [], extra: [] });
  });
});
