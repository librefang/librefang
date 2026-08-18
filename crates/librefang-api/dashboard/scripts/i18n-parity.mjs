#!/usr/bin/env node
// Standalone CLI mirror of src/lib/__tests__/locale-parity.test.ts.
// Use this when you want a quick pre-commit check without spinning up
// vitest. The vitest version is what gates CI (runs as part of
// `pnpm test` in dashboard-build.yml).
//
// Usage:
//   node scripts/i18n-parity.mjs
// Exit code: 0 on parity, 1 on drift.

import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const LOCALES_DIR = join(here, "..", "src", "locales");
const REFERENCE = "en.json";

function flattenValue(node, prefix) {
  if (Array.isArray(node)) {
    if (node.length === 0) return [`${prefix}[]`];
    return node.flatMap((value, index) => flattenValue(value, `${prefix}[${index}]`));
  }
  if (node === null || typeof node !== "object") {
    return [prefix];
  }
  const out = [];
  for (const [key, value] of Object.entries(node)) {
    out.push(...flattenValue(value, prefix ? `${prefix}.${key}` : key));
  }
  return out;
}

export function flatten(node) {
  if (node === null || typeof node !== "object" || Array.isArray(node)) {
    throw new TypeError("Locale root must be a JSON object");
  }
  return flattenValue(node, "");
}

export function loadFlat(file, localesDir = LOCALES_DIR) {
  try {
    const text = readFileSync(join(localesDir, file), "utf8");
    return new Set(flatten(JSON.parse(text)));
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new Error(`Failed to load locale ${file}: ${detail}`, { cause: error });
  }
}

export function runParity() {
  const reference = loadFlat(REFERENCE);
  const others = readdirSync(LOCALES_DIR).filter(
    (f) => f.endsWith(".json") && f !== REFERENCE,
  );

  let drift = false;
  for (const file of others) {
    const locale = loadFlat(file);
    const missing = [...reference].filter((k) => !locale.has(k)).sort();
    const extra = [...locale].filter((k) => !reference.has(k)).sort();
    if (missing.length === 0 && extra.length === 0) {
      console.log(`OK   ${file} (${locale.size} keys, parity with ${REFERENCE})`);
      continue;
    }
    drift = true;
    console.error(`FAIL ${file}`);
    if (missing.length) console.error(`  missing (${missing.length}):`, missing);
    if (extra.length) console.error(`  extra (${extra.length}):`, extra);
  }

  if (drift) {
    console.error(
      "\nLocale drift detected. Add the missing translations to the affected locale, " +
        "and remove any extra (dead) keys. See issue #3557 for context.",
    );
    return 1;
  }
  console.log("\nAll locales in parity with en.json.");
  return 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    process.exitCode = runParity();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
