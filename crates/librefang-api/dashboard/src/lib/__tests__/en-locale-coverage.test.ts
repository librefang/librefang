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

type DefaultedKey = UsedKey & {
  defaultValue: string;
};

type DynamicKeyPattern = {
  pattern: RegExp;
  source: string;
  path: string;
  line: number;
};

type HardcodedText = {
  text: string;
  kind: string;
  path: string;
  line: number;
};

type LiteralCandidate = {
  text: string;
  kind: string;
  path: string;
  line: number;
};

type I18nUsage = {
  keys: UsedKey[];
  looseKeys: UsedKey[];
  dynamicPatterns: DynamicKeyPattern[];
  defaultedKeys: DefaultedKey[];
  hardcodedTexts: HardcodedText[];
  literalCandidates: LiteralCandidate[];
};

let usageCache: I18nUsage | null = null;

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
      !/\.test\.(ts|tsx)$/.test(entry.name) &&
      entry.name !== "setupTests.ts"
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

function bindingNameText(name: ts.BindingName): string | null {
  return ts.isIdentifier(name) ? name.text : null;
}

function isTranslationCall(node: ts.CallExpression): boolean {
  const callee = node.expression;
  return (
    (ts.isIdentifier(callee) && callee.text === "t") ||
    (ts.isPropertyAccessExpression(callee) && callee.name.text === "t")
  );
}

function hasAncestor(node: ts.Node, predicate: (ancestor: ts.Node) => boolean): boolean {
  let current = node.parent;
  while (current) {
    if (predicate(current)) return true;
    current = current.parent;
  }
  return false;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function templatePattern(node: ts.TemplateExpression): RegExp {
  const parts = [escapeRegExp(node.head.text)];
  for (const span of node.templateSpans) {
    parts.push(".+", escapeRegExp(span.literal.text));
  }
  return new RegExp(`^${parts.join("")}$`);
}

function isUserFacingPropertyName(name: string): boolean {
  return /^(ariaLabel|badge|cancelLabel|confirmLabel|description|detail|empty|emptyBody|emptyTitle|errorText|help|helpText|hint|label|message|placeholder|subtitle|successText|title|tooltip)$/.test(name);
}

function isUserFacingVariableName(name: string): boolean {
  return /^(description|detail|empty|errorText|helpText|message|subtitle|title)$/.test(name);
}

function collectLiteralTexts(node: ts.Node, out: string[] = []): string[] {
  const text = stringLiteralText(node);
  if (text !== null) {
    out.push(text);
    return out;
  }
  if (ts.isTemplateExpression(node)) {
    out.push(node.head.text);
    for (const span of node.templateSpans) {
      out.push(span.literal.text);
    }
    return out;
  }
  ts.forEachChild(node, (child) => collectLiteralTexts(child, out));
  return out;
}

function collectI18nUsage(): I18nUsage {
  if (usageCache) return usageCache;

  const keys: UsedKey[] = [];
  const looseKeys: UsedKey[] = [];
  const dynamicPatterns: DynamicKeyPattern[] = [];
  const defaultedKeys: DefaultedKey[] = [];
  const hardcodedTexts: HardcodedText[] = [];
  const literalCandidates: LiteralCandidate[] = [];

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

    function addLooseKey(key: string, node: ts.Node) {
      const { line } = source.getLineAndCharacterOfPosition(node.getStart());
      looseKeys.push({
        key,
        path: relative(SRC_DIR, file),
        line: line + 1,
      });
    }

    function addDefaultedKey(key: string, node: ts.Node, defaultValue: string) {
      const { line } = source.getLineAndCharacterOfPosition(node.getStart());
      defaultedKeys.push({
        key,
        path: relative(SRC_DIR, file),
        line: line + 1,
        defaultValue,
      });
    }

    function literalDefault(node: ts.Node): string | null {
      const direct = stringLiteralText(node);
      if (direct !== null) return direct;
      if (ts.isObjectLiteralExpression(node)) {
        for (const property of node.properties) {
          if (!ts.isPropertyAssignment(property)) continue;
          const name = propertyNameText(property.name);
          if (name !== "defaultValue") continue;
          return stringLiteralText(property.initializer);
        }
      }
      return null;
    }

    function addDynamicPattern(node: ts.TemplateExpression) {
      const { line } = source.getLineAndCharacterOfPosition(node.getStart());
      dynamicPatterns.push({
        pattern: templatePattern(node),
        source: node.getText(source),
        path: relative(SRC_DIR, file),
        line: line + 1,
      });
    }

    function addHardcodedText(text: string, kind: string, node: ts.Node) {
      const { line } = source.getLineAndCharacterOfPosition(node.getStart());
      hardcodedTexts.push({
        text,
        kind,
        path: relative(SRC_DIR, file),
        line: line + 1,
      });
    }

    function addLiteralCandidate(text: string, kind: string, node: ts.Node) {
      const { line } = source.getLineAndCharacterOfPosition(node.getStart());
      literalCandidates.push({
        text,
        kind,
        path: relative(SRC_DIR, file),
        line: line + 1,
      });
    }

    function literalCandidateKind(node: ts.Node): string | null {
      const parent = node.parent;

      if (!parent) return "literal";
      if (ts.isImportDeclaration(parent) || ts.isExportDeclaration(parent)) {
        return null;
      }
      if (ts.isLiteralTypeNode(parent)) return null;
      if (ts.isPropertyAssignment(parent) && parent.name === node) return null;
      if (ts.isPropertyAccessExpression(parent)) return null;
      if (ts.isElementAccessExpression(parent)) return null;
      if (hasAncestor(node, (ancestor) => ts.isCallExpression(ancestor) && isTranslationCall(ancestor))) {
        return null;
      }
      if (
        hasAncestor(
          node,
          (ancestor) =>
            ts.isCallExpression(ancestor) &&
            ts.isPropertyAccessExpression(ancestor.expression) &&
            ts.isIdentifier(ancestor.expression.expression) &&
            ancestor.expression.expression.text === "console",
        )
      ) {
        return null;
      }

      if (ts.isJsxAttribute(parent) && parent.initializer === node) {
        if (!ts.isIdentifier(parent.name)) return null;
        const attrName = parent.name.text;
        if (
          /^(className|id|href|to|target|rel|type|value|name|role|data-|width|height|viewBox|d|stroke|fill|xmlns)$/.test(
            attrName,
          )
        ) {
          return null;
        }
        return `jsx-attr:${attrName}`;
      }

      if (ts.isPropertyAssignment(parent) && parent.initializer === node) {
        const propertyName = propertyNameText(parent.name);
        if (!propertyName) return "ts-prop";
        if (propertyName === "defaultValue") return null;
        return isUserFacingPropertyName(propertyName)
          ? `ts-prop:${propertyName}`
          : null;
      }

      if (ts.isVariableDeclaration(parent) && parent.initializer === node) {
        const name = bindingNameText(parent.name);
        return name && isUserFacingVariableName(name) ? `ts-var:${name}` : null;
      }

      if (hasAncestor(node, ts.isJsxExpression)) {
        return "jsx-expression";
      }

      return null;
    }

    function visit(node: ts.Node) {
      const literalKey = stringLiteralText(node);
      if (literalKey && isLikelyLocaleKey(literalKey)) {
        addLooseKey(literalKey, node);
      }

      if (literalKey) {
        const kind = literalCandidateKind(node);
        if (kind) addLiteralCandidate(literalKey, kind, node);
      }

      if (ts.isCallExpression(node) && isTranslationCall(node)) {
        const firstArg = node.arguments[0];
        if (firstArg) {
          const key = stringLiteralText(firstArg);
          if (key) addKey(key, firstArg);
          if (ts.isTemplateExpression(firstArg)) addDynamicPattern(firstArg);
          const secondArg = node.arguments[1];
          const defaultValue = secondArg ? literalDefault(secondArg) : null;
          if (key && defaultValue !== null) {
            addDefaultedKey(key, firstArg, defaultValue);
          }
        }
      }

      if (ts.isPropertyAssignment(node)) {
        const propertyName = propertyNameText(node.name);
        const key = stringLiteralText(node.initializer);
        if (propertyName?.endsWith("Key") && key && isLikelyLocaleKey(key)) {
          addKey(key, node.initializer);
        }
        if (
          propertyName?.endsWith("Key") &&
          ts.isTemplateExpression(node.initializer)
        ) {
          addDynamicPattern(node.initializer);
        }
        if (
          propertyName &&
          propertyName !== "defaultValue" &&
          isUserFacingPropertyName(propertyName)
        ) {
          for (const text of collectLiteralTexts(node.initializer)) {
            addHardcodedText(text, `ts-prop:${propertyName}`, node.initializer);
          }
        }
      }

      if (ts.isVariableDeclaration(node)) {
        const name = bindingNameText(node.name);
        if (name && node.initializer && isUserFacingVariableName(name)) {
          for (const text of collectLiteralTexts(node.initializer)) {
            addHardcodedText(text, `ts-var:${name}`, node.initializer);
          }
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

      if (ts.isJsxText(node)) {
        addHardcodedText(node.getText(source), "jsx-text", node);
      }

      if (
        ts.isJsxAttribute(node) &&
        ts.isIdentifier(node.name) &&
        ["aria-label", "title", "placeholder", "alt"].includes(node.name.text) &&
        node.initializer &&
        ts.isStringLiteral(node.initializer)
      ) {
        addHardcodedText(
          node.initializer.text,
          `jsx-attr:${node.name.text}`,
          node.initializer,
        );
      }

      ts.forEachChild(node, visit);
    }

    visit(source);
  }

  usageCache = {
    keys,
    looseKeys,
    dynamicPatterns,
    defaultedKeys,
    hardcodedTexts,
    literalCandidates,
  };
  return usageCache;
}

function collectUsedKeys(): UsedKey[] {
  return collectI18nUsage().keys;
}

function localeFiles(): string[] {
  return readdirSync(LOCALES_DIR)
    .filter((file) => file.endsWith(".json"))
    .map((file) => join(LOCALES_DIR, file));
}

function keyMatchesDynamicUsage(
  key: string,
  dynamicPatterns: DynamicKeyPattern[],
): boolean {
  return dynamicPatterns.some(({ pattern }) => pattern.test(key));
}

function normalizeJsxText(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function isTechnicalLiteral(text: string): boolean {
  if (text === "") return true;
  if (text === "×") return true;
  if (!/\p{L}/u.test(text)) return true;
  if (/^&[a-z]+;$/.test(text)) return true;
  if (/^(⌘|Ctrl\+|ESC|↑↓|↵)/.test(text)) return true;
  if (/^(Cmd\/Ctrl|Shift\+Cmd\/Ctrl|Space \+ Drag)/.test(text)) return true;
  if (/^librefang\b/.test(text)) return true;
  if (/^(GET|POST|PUT|PATCH|DELETE) \//.test(text)) return true;
  if (/^[A-Z][A-Za-z0-9]+(?:[A-Z][A-Za-z0-9]+)+$/.test(text)) return true;
  if (/^(in \/|out tokens|\/ M)$/.test(text)) return true;
  if (/^[·•›→—-]\s*[\w./:$?&=%#{}()[\]@+-]+$/.test(text)) return true;
  if (/^[A-Z0-9_./:$?&=%#{}()[\]@+-]+$/.test(text)) return true;
  if (/^[a-z0-9_./:$?&=%#{}()[\]@+-]+$/.test(text)) return true;
  if (/^https?:\/\//.test(text)) return true;
  if (/^\/[\w./-]+$/.test(text)) return true;
  if (/^[\w.-]+@[\w.-]+$/.test(text)) return true;
  if (/^[\w./:-]+(\s*,\s*[\w./:-]+)+$/.test(text)) return true;
  return false;
}

function isStylingLiteral(text: string): boolean {
  if (/^(linear-gradient|#[0-9a-fA-F]{3,8}\b|[0-9.]+px solid\b)/.test(text)) {
    return true;
  }
  if (
    /(?:^|\s)(?:absolute|relative|fixed|sticky|block|inline|inline-flex|flex|grid|hidden|contents|sr-only|pointer-events-none|cursor-|select-|resize-|overflow-|z-|inset-|top-|right-|bottom-|left-|m[trblxy]?-|p[trblxy]?-|h-|w-|min-|max-|rounded|border|bg-|from-|via-|to-|text-|font-|tracking-|leading-|uppercase|lowercase|capitalize|items-|justify-|content-|self-|gap-|space-|shadow|ring-|outline|opacity-|transition|duration-|ease-|delay-|transform|translate-|scale-|rotate-|animate-|motion-|hover:|focus:|active:|disabled:|group-hover:|sm:|md:|lg:|xl:|2xl:|3xl:|4xl:|\[[^\]]+\])/.test(
      text,
    )
  ) {
    return true;
  }
  return false;
}

function isAllowedHardcodedText(text: string, kind: string): boolean {
  if (isTechnicalLiteral(text)) return true;
  if (["English", "Українська", "中文", "简体中文"].includes(text)) return true;
  if (["Telegram", "Slack", "Discord", "Signal"].includes(text)) return true;
  if (["CLI N/A"].includes(text)) return true;
  if (kind === "jsx-attr:placeholder" && /^e\.g\. [\w./:"{}* -]+$/.test(text)) {
    return true;
  }
  if (
    kind === "jsx-attr:placeholder" &&
    /^(gpt-|GPT-|OPENAI_|UTC$|none$|main$|daily-|my-|platform_id$|tool_name$|\$\.|Bearer )/.test(text)
  ) {
    return true;
  }
  return false;
}

function isAllowedLiteralCandidate(text: string, kind: string): boolean {
  if (isAllowedHardcodedText(text, kind)) return true;
  if (isStylingLiteral(text)) return true;
  if (isLikelyLocaleKey(text)) return true;
  if (/^\.\/pages\/[A-Za-z]+/.test(text)) return true;
  if (/^[a-z][A-Za-z0-9]*(?:\.[a-z][A-Za-z0-9]*)+$/.test(text)) return true;
  if (/^[A-Za-z][A-Za-z0-9_]*(?:\[\])?$/.test(text)) return true;
  if (/^[A-Za-z][A-Za-z0-9_]*(?:\s+[A-Za-z][A-Za-z0-9_]*)+$/.test(text) && kind === "ts-prop:keyword") return true;
  if (/^\\n$/.test(text)) return true;
  if (/^".*"$/.test(text)) return true;
  if (/^[A-Z][a-z]+(?: [A-Z][a-z]+)*$/.test(text)) return true;
  if (/^(OpenAI|Anthropic|Gemini|Groq|Ollama|Mistral|DeepSeek|Qwen|Llama|Claude|GPT|MCP|OAuth|PKCE|SSE|HTTP|JSON|YAML|TOML|CSV|SQLite|GitHub|GitLab|Bitbucket|Notion|Linear|WhatsApp|WeChat|Mailgun|IMAP|SMTP)$/.test(text)) return true;
  if (/^(text|password|number|checkbox|radio|button|submit|reset|email|url|search|tel|date|time|datetime-local|month|week|color|file|range|hidden)$/.test(text)) return true;
  if (/^(left|right|top|bottom|center|start|end|horizontal|vertical|none|auto|manual|custom|default|primary|secondary|success|warning|danger|info|small|medium|large)$/.test(text)) return true;
  if (/^(true|false|null|undefined|NaN|Infinity)$/.test(text)) return true;
  if (/^[A-Za-z_][A-Za-z0-9_]*\([^)]*\)$/.test(text)) return true;
  return false;
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

  it("copies every literal default fallback into src/locales/en.json", () => {
    const enKeys = new Set(
      flatten(JSON.parse(readFileSync(EN_LOCALE, "utf8")) as JsonValue),
    );
    const pluralBases = new Set(
      [...enKeys].map(pluralBase).filter((b): b is string => b !== null),
    );

    const missingByKey = new Map<string, string[]>();
    for (const { key, path, line } of collectI18nUsage().defaultedKeys) {
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
      "Dashboard t(...) calls provide literal fallback text that is not copied into src/locales/en.json.",
    ).toEqual([]);
  });

  it("does not use conflicting literal fallbacks for the same i18n key", () => {
    const defaultsByKey = new Map<string, Map<string, string[]>>();
    for (const { key, defaultValue, path, line } of collectI18nUsage()
      .defaultedKeys) {
      const locationsByDefault = defaultsByKey.get(key) ?? new Map();
      const locations = locationsByDefault.get(defaultValue) ?? [];
      locations.push(`${path}:${line}`);
      locationsByDefault.set(defaultValue, locations);
      defaultsByKey.set(key, locationsByDefault);
    }

    const conflicts = [...defaultsByKey.entries()]
      .filter(([, locationsByDefault]) => locationsByDefault.size > 1)
      .map(([key, locationsByDefault]) => {
        const variants = [...locationsByDefault.entries()]
          .map(([defaultValue, locations]) => {
            return `${JSON.stringify(defaultValue)} at ${locations.join(", ")}`;
          })
          .join("; ");
        return `${key}: ${variants}`;
      })
      .sort();

    expect(
      conflicts,
      "A single i18n key should not carry multiple literal fallback strings.",
    ).toEqual([]);
  });

  it("does not hardcode user-facing JSX text", () => {
    const hardcoded = collectI18nUsage().hardcodedTexts
      .map(({ text, kind, path, line }) => ({
        text: normalizeJsxText(text),
        kind,
        path,
        line,
      }))
      .filter(({ text, kind }) => !isAllowedHardcodedText(text, kind))
      .map(({ text, kind, path, line }) => {
        return `${path}:${line} ${kind} ${JSON.stringify(text)}`;
      })
      .sort();

    expect(
      hardcoded,
      "Dashboard JSX contains hardcoded user-facing text. Route it through t(...).",
    ).toEqual([]);
  });

  it("does not leave unaudited user-facing string literals", () => {
    const candidates = collectI18nUsage().literalCandidates
      .map(({ text, kind, path, line }) => ({
        text: normalizeJsxText(text),
        kind,
        path,
        line,
      }))
      .filter(({ text, kind }) => !isAllowedLiteralCandidate(text, kind))
      .map(({ text, kind, path, line }) => {
        return `${path}:${line} ${kind} ${JSON.stringify(text)}`;
      })
      .sort();

    expect(
      candidates,
      "Dashboard source contains string literals that look user-facing but are not routed through i18n. Either translate them or add a narrow technical allowlist.",
    ).toEqual([]);
  });

  it("does not carry dead keys in locale files", () => {
    const { keys, looseKeys, dynamicPatterns } = collectI18nUsage();
    const usedKeys = new Set(
      [...keys, ...looseKeys].map(({ key }) => key),
    );

    const deadKeys = localeFiles()
      .flatMap((localeFile) => {
        const localeName = relative(LOCALES_DIR, localeFile);
        return flatten(
          JSON.parse(readFileSync(localeFile, "utf8")) as JsonValue,
        )
          .filter((key) => {
            const base = pluralBase(key);
            return (
              !usedKeys.has(key) &&
              (base === null || !usedKeys.has(base)) &&
              !keyMatchesDynamicUsage(key, dynamicPatterns)
            );
          })
          .map((key) => `${localeName}: ${key}`);
      })
      .sort();

    expect(
      deadKeys,
      "Locale files define i18n keys that are not referenced by dashboard source.",
    ).toEqual([]);
  });
});
