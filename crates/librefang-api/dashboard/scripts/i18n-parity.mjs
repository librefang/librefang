#!/usr/bin/env node
// Standalone CLI and shared comparison used by locale-parity.test.ts.
// Use this when you want a quick pre-commit check without spinning up
// vitest. Both entry points use compareKeys; the vitest suite gates CI (part of
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

// i18next chooses cardinal suffixes from the locale's CLDR categories rather
// than requiring the English one/other pair in every language.
const PLURAL_SUFFIX_RE = /_(zero|one|two|few|many|other)$/;

export function compareKeys(reference, locale, tag) {
  const pluralBase = (key) => key.replace(PLURAL_SUFFIX_RE, "");
  const nonPlural = (keys) => new Set([...keys].filter((key) => !PLURAL_SUFFIX_RE.test(key)));
  const refNonPlural = nonPlural(reference);
  const localeNonPlural = nonPlural(locale);
  const missing = [...refNonPlural].filter((key) => !localeNonPlural.has(key)).sort();
  const extra = [...localeNonPlural].filter((key) => !refNonPlural.has(key)).sort();
  const bases = new Set([...reference].filter((key) => PLURAL_SUFFIX_RE.test(key)).map(pluralBase));
  const categories = new Intl.PluralRules(tag, { type: "cardinal" }).resolvedOptions().pluralCategories;
  const missingPlural = [];
  for (const base of bases) {
    for (const category of categories) {
      const key = `${base}_${category}`;
      if (!locale.has(key)) missingPlural.push(key);
    }
  }
  // Preserve the CI policy: unused plural suffixes are tolerated.
  return { missing, extra, missingPlural: missingPlural.sort() };
}

export function runParity(localesDir = LOCALES_DIR) {
  const reference = loadFlat(REFERENCE, localesDir);
  const others = readdirSync(localesDir).filter(
    (f) => f.endsWith(".json") && f !== REFERENCE,
  );

  let drift = false;
  for (const file of others) {
    const locale = loadFlat(file, localesDir);
    const result = compareKeys(reference, locale, file.slice(0, -".json".length));
    const missing = [...result.missing, ...result.missingPlural].sort();
    const extra = result.extra;
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
