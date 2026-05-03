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
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const LOCALES_DIR = join(here, "..", "src", "locales");
const REFERENCE = "en.json";

function flatten(node, prefix = "") {
  if (node === null || typeof node !== "object" || Array.isArray(node)) {
    return [prefix];
  }
  const out = [];
  for (const [k, v] of Object.entries(node)) {
    out.push(...flatten(v, prefix ? `${prefix}.${k}` : k));
  }
  return out;
}

function loadFlat(file) {
  return new Set(flatten(JSON.parse(readFileSync(join(LOCALES_DIR, file), "utf8"))));
}

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
  process.exit(1);
}
console.log("\nAll locales in parity with en.json.");
