import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { basename, join } from "node:path";

// Per issue #3557: with i18next `fallbackLng: "en"`, any key present in
// `en.json` but missing from a non-English locale file silently falls back
// to English at runtime, leaving Chinese (or future) users with mid-page
// English text. This test asserts that every locale file carries the same
// non-plural key set as `en.json` — no missing keys, no dead keys.
//
// Plural keys are the exception: i18next selects a per-language CLDR plural
// category suffix (`_one`, `_few`, `_many`, …), and the required set of
// categories differs by language (English has one/other; Ukrainian has
// one/few/many/other; Chinese has only other). A flat key-set equality check
// would wrongly flag a locale's language-correct extra forms (e.g. Ukrainian
// `_few`/`_many`) as drift, or pass a locale that is missing forms its grammar
// requires. So plural families are validated against `Intl.PluralRules`
// instead of against the reference key set.

const LOCALES_DIR = join(__dirname, "..", "..", "locales");
const REFERENCE = "en.json";

// i18next / CLDR cardinal plural suffixes. A flattened key ending in one of
// these (after a `_`) is a member of a plural family.
const PLURAL_SUFFIXES = ["zero", "one", "two", "few", "many", "other"] as const;
const PLURAL_SUFFIX_RE = new RegExp(`_(${PLURAL_SUFFIXES.join("|")})$`);

type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [k: string]: JsonValue };

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

function loadFlat(file: string): string[] {
  const text = readFileSync(join(LOCALES_DIR, file), "utf8");
  return flatten(JSON.parse(text) as JsonValue);
}

// The base of a plural key (everything before the `_<suffix>`), or null if the
// key is not a plural family member.
function pluralBase(key: string): string | null {
  return PLURAL_SUFFIX_RE.test(key) ? key.replace(PLURAL_SUFFIX_RE, "") : null;
}

function pluralSuffix(key: string): string {
  return key.slice(key.lastIndexOf("_") + 1);
}

// `uk.json` -> `uk`, the BCP-47 tag i18next derives the locale from.
function localeTag(file: string): string {
  return basename(file, ".json");
}

// Per issue #8162: `JSON.parse` is last-wins on a repeated key, so a locale can
// carry the same key twice inside one object and every check built on the parsed
// tree — including the parity assertions above — sees only the survivor.
// The first copy is dead text that no amount of translating will ever reach.
// It gets in without a merge conflict: two branches add the key at different
// positions in the same object and git merges both cleanly.
//
// Detection has to read the raw text, not the parsed tree. This walks the source
// tracking one key set per object; a string literal is a key when it follows `{`
// or `,` inside an object.
function findDuplicateKeys(text: string): { path: string; key: string }[] {
  type Frame = { keys: Set<string> | null; name: string; lastKey: string };
  const dups: { path: string; key: string }[] = [];
  const stack: Frame[] = [];
  const pathOf = () =>
    stack
      .map((f) => f.name)
      .filter(Boolean)
      .join(".") || "(root)";
  let prevSig = "";

  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '"') {
      let j = i + 1;
      while (j < text.length && text[j] !== '"') j += text[j] === "\\" ? 2 : 1;
      const top = stack[stack.length - 1];
      if (top?.keys && (prevSig === "{" || prevSig === ",")) {
        const key = JSON.parse(text.slice(i, j + 1)) as string;
        if (top.keys.has(key)) dups.push({ path: pathOf(), key });
        else top.keys.add(key);
        top.lastKey = key;
      }
      i = j;
      prevSig = '"';
      continue;
    }
    if (c === "{" || c === "[") {
      const parent = stack[stack.length - 1];
      stack.push({
        keys: c === "{" ? new Set() : null,
        name: parent?.keys ? parent.lastKey : "",
        lastKey: "",
      });
    } else if (c === "}" || c === "]") {
      stack.pop();
    }
    if (!/\s/.test(c)) prevSig = c;
  }
  return dups;
}

describe("locale parity", () => {
  const refKeys = loadFlat(REFERENCE);
  const refNonPlural = new Set(refKeys.filter((k) => pluralBase(k) === null));
  const refPluralBases = new Set(
    refKeys
      .map(pluralBase)
      .filter((b): b is string => b !== null),
  );

  const others = readdirSync(LOCALES_DIR).filter(
    (f) => f.endsWith(".json") && f !== REFERENCE,
  );

  it.each([REFERENCE, ...others])("%s declares every key once", (file) => {
    const dups = findDuplicateKeys(readFileSync(join(LOCALES_DIR, file), "utf8"));
    expect(
      dups,
      `Locale ${file} repeats a key inside one object. JSON.parse keeps the last ` +
        `copy, so the earlier one is unreachable dead text. Delete the copy whose ` +
        `value is not the one the UI shows:\n${JSON.stringify(dups, null, 2)}`,
    ).toEqual([]);
  });

  it("the duplicate scanner reads raw text, not the parsed tree", () => {
    // Same key twice in one object is the bug; the same key in sibling objects,
    // in a nested object, or as a string value is not.
    expect(findDuplicateKeys('{"a": 1, "b": {"c": 2}, "a": 3}')).toEqual([
      { path: "(root)", key: "a" },
    ]);
    expect(findDuplicateKeys('{"x": {"a": 1, "a": 2}}')).toEqual([
      { path: "x", key: "a" },
    ]);
    expect(findDuplicateKeys('{"x": {"a": 1}, "y": {"a": 2}}')).toEqual([]);
    expect(findDuplicateKeys('{"a": ["a", "a"], "b": "a"}')).toEqual([]);
    // A quote or a brace inside a value must not shift the key/value phase.
    expect(findDuplicateKeys('{"a": "say \\" {", "a": 1}')).toEqual([
      { path: "(root)", key: "a" },
    ]);
  });

  it.each(others)("%s has the same non-plural key set as en.json", (file) => {
    const nonPlural = new Set(
      loadFlat(file).filter((k) => pluralBase(k) === null),
    );
    const missing = [...refNonPlural].filter((k) => !nonPlural.has(k)).sort();
    const extra = [...nonPlural].filter((k) => !refNonPlural.has(k)).sort();

    expect(
      { missing, extra },
      `Locale ${file} drifted from en.json (non-plural keys).\n` +
        `Missing keys (will fall back to English at runtime): ${JSON.stringify(missing, null, 2)}\n` +
        `Extra keys (dead — no English source): ${JSON.stringify(extra, null, 2)}`,
    ).toEqual({ missing: [], extra: [] });
  });

  it.each(others)(
    "%s provides every CLDR plural form its language requires",
    (file) => {
      const tag = localeTag(file);
      const requiredCategories = new Intl.PluralRules(tag, {
        type: "cardinal",
      }).resolvedOptions().pluralCategories;

      // Suffixes present in this locale, grouped by plural base.
      const presentByBase = new Map<string, Set<string>>();
      for (const key of loadFlat(file)) {
        const base = pluralBase(key);
        if (base === null) continue;
        if (!presentByBase.has(base)) presentByBase.set(base, new Set());
        presentByBase.get(base)!.add(pluralSuffix(key));
      }

      // For every plural family that exists in the reference, the locale must
      // supply each category its own grammar selects. Extra suffixes a language
      // never selects (e.g. a dead `_one` in Chinese) are tolerated — they are
      // benign and removing them is out of scope here.
      const missing: string[] = [];
      for (const base of refPluralBases) {
        const have = presentByBase.get(base) ?? new Set<string>();
        for (const category of requiredCategories) {
          if (!have.has(category)) missing.push(`${base}_${category}`);
        }
      }
      missing.sort();

      expect(
        missing,
        `Locale ${file} (${tag}) is missing CLDR-required plural forms ` +
          `[${requiredCategories.join(", ")}]:\n${JSON.stringify(missing, null, 2)}`,
      ).toEqual([]);
    },
  );
});
