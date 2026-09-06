import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";

// The CLI and CI share the comparison so valid CLDR forms cannot become
// false-positive drift in one entry point while passing the other (#8167).
const modulePath = "../../../scripts/i18n-parity.mjs";
interface ParityScript {
  loadFlat: (file: string, localesDir?: string) => Set<string>;
  compareKeys: (reference: Set<string>, locale: Set<string>, tag: string) => {
    missing: string[]; extra: string[]; missingPlural: string[];
  };
}
const parity = await import(/* @vite-ignore */ modulePath) as ParityScript;
const LOCALES_DIR = join(__dirname, "..", "..", "locales");
const REFERENCE = "en.json";

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
  const reference = parity.loadFlat(REFERENCE, LOCALES_DIR);
  const others = readdirSync(LOCALES_DIR).filter(
    (file) => file.endsWith(".json") && file !== REFERENCE,
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
    const { missing, extra } = parity.compareKeys(
      reference, parity.loadFlat(file, LOCALES_DIR), basename(file, ".json"),
    );
    expect({ missing, extra }, `Locale ${file} drifted from en.json`).toEqual({
      missing: [], extra: [],
    });
  });

  it.each(others)("%s provides every CLDR plural form its language requires", (file) => {
    const { missingPlural } = parity.compareKeys(
      reference, parity.loadFlat(file, LOCALES_DIR), basename(file, ".json"),
    );
    expect(missingPlural, `Locale ${file} is missing required plural forms`).toEqual([]);
  });
});
