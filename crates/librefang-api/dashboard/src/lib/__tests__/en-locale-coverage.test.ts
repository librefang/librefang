import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join, relative } from "node:path";
import ts from "typescript";

const SRC_DIR = join(__dirname, "..", "..");
const LOCALES_DIR = join(SRC_DIR, "locales");
const EN_LOCALE = join(LOCALES_DIR, "en.json");
const PLURAL_SUFFIX_RE = /_(zero|one|two|few|many|other)$/;

type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [k: string]: JsonValue };

type UsedKey = {
  key: string;
  path: string;
  line: number;
};

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

function pluralBase(key: string): string | null {
  return PLURAL_SUFFIX_RE.test(key) ? key.replace(PLURAL_SUFFIX_RE, "") : null;
}

function sourceFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== "__tests__") out.push(...sourceFiles(path));
      continue;
    }
    if (
      entry.isFile() &&
      /\.(ts|tsx)$/.test(entry.name) &&
      !/\.test\.(ts|tsx)$/.test(entry.name)
    ) {
      out.push(path);
    }
  }
  return out;
}

function stringLiteralText(node: ts.Node): string | null {
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) {
    return node.text;
  }
  return null;
}

function isLikelyLocaleKey(value: string): boolean {
  return /^[a-z][a-z0-9_]*(\.[a-z0-9_]+)+$/.test(value);
}

function propertyNameText(name: ts.PropertyName): string | null {
  if (ts.isIdentifier(name) || ts.isStringLiteral(name)) return name.text;
  return null;
}

function isTranslationCall(node: ts.CallExpression): boolean {
  const callee = node.expression;
  return (
    (ts.isIdentifier(callee) && callee.text === "t") ||
    (ts.isPropertyAccessExpression(callee) && callee.name.text === "t")
  );
}

function collectUsedKeys(): UsedKey[] {
  const keys: UsedKey[] = [];

  for (const file of sourceFiles(SRC_DIR)) {
    const text = readFileSync(file, "utf8");
    const source = ts.createSourceFile(
      file,
      text,
      ts.ScriptTarget.Latest,
      true,
      file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
    );

    function addKey(key: string, node: ts.Node) {
      const { line } = source.getLineAndCharacterOfPosition(node.getStart());
      keys.push({
        key,
        path: relative(SRC_DIR, file),
        line: line + 1,
      });
    }

    function visit(node: ts.Node) {
      if (ts.isCallExpression(node) && isTranslationCall(node)) {
        const firstArg = node.arguments[0];
        if (firstArg) {
          const key = stringLiteralText(firstArg);
          if (key) addKey(key, firstArg);
        }
      }

      if (ts.isPropertyAssignment(node)) {
        const propertyName = propertyNameText(node.name);
        const key = stringLiteralText(node.initializer);
        if (propertyName?.endsWith("Key") && key && isLikelyLocaleKey(key)) {
          addKey(key, node.initializer);
        }
      }

      if (
        ts.isJsxAttribute(node) &&
        ts.isIdentifier(node.name) &&
        node.name.text === "i18nKey" &&
        node.initializer &&
        ts.isStringLiteral(node.initializer)
      ) {
        addKey(node.initializer.text, node.initializer);
      }

      ts.forEachChild(node, visit);
    }

    visit(source);
  }

  return keys;
}

describe("Dashboard locale coverage", () => {
  it("defines every literal i18n key used by dashboard source", () => {
    const enKeys = new Set(
      flatten(JSON.parse(readFileSync(EN_LOCALE, "utf8")) as JsonValue),
    );
    const pluralBases = new Set(
      [...enKeys].map(pluralBase).filter((b): b is string => b !== null),
    );

    const missingByKey = new Map<string, string[]>();
    for (const { key, path, line } of collectUsedKeys()) {
      if (enKeys.has(key) || pluralBases.has(key)) continue;
      const locations = missingByKey.get(key) ?? [];
      locations.push(`${path}:${line}`);
      missingByKey.set(key, locations);
    }

    const missing = [...missingByKey.entries()]
      .map(([key, locations]) => `${key} (${locations.join(", ")})`)
      .sort();

    expect(
      missing,
      "Dashboard source references i18n keys that are missing from src/locales/en.json.",
    ).toEqual([]);
  });
});
