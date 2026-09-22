import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import {
  MAX_BASE_NAME_CHARS,
  MAX_FILENAME_CHARS,
  isValidBaseName,
  isValidDocumentName,
} from "./knowledgeNames";

// src/lib -> src -> dashboard -> librefang-api -> crates -> repo root
const REPO_ROOT = join(__dirname, "..", "..", "..", "..", "..");
const TEXT_RS = join(REPO_ROOT, "crates", "librefang-types", "src", "text.rs");
const KNOWLEDGE_RS = join(REPO_ROOT, "crates", "librefang-api", "src", "routes", "knowledge.rs");

/** The `\u{XXXX}` escapes inside one named Rust const, in source order. */
function rustCharConst(source: string, name: string): string[] {
  const start = source.indexOf(`${name}: &[char] = &[`);
  expect(start, `${name} not found — the Rust const was renamed or moved`).toBeGreaterThan(-1);
  const end = source.indexOf("];", start);
  expect(end, `${name} is not terminated`).toBeGreaterThan(start);
  const block = source.slice(start, end);
  return [...block.matchAll(/\\u\{([0-9A-Fa-f]+)\}/g)].map(([, hex]) =>
    String.fromCodePoint(parseInt(hex, 16)),
  );
}

function rustUsize(source: string, name: string): number {
  const match = new RegExp(`const ${name}: usize = (\\d+);`).exec(source);
  expect(match, `${name} not found in knowledge.rs`).not.toBeNull();
  return Number(match![1]);
}

describe("knowledge name rule", () => {
  it("accepts the names an internationalised product has to accept", () => {
    // The whole point of the change away from `[A-Za-z0-9._-]`: these are
    // ordinary things to call a document, and the old rule refused all of them.
    for (const ok of [
      "handbook",
      "team-notes",
      "v2.1_specs",
      "A9",
      "Manual de operaciones",
      "Informe Q3 (final).pdf",
      "運用マニュアル.md",
      "Руководство",
      "안내서.txt",
      "Employee Handbook 2026.pdf",
    ]) {
      expect(isValidDocumentName(ok), ok).toBe(true);
    }
  });

  it("refuses the classes that are dangerous or are not stored faithfully", () => {
    for (const bad of [
      "", // empty
      ".", // current directory
      "..", // traversal
      ".hidden", // dotfile
      "a/b", // posix separator
      "a\\b", // windows separator
      "trailing.", // windows strips it
      " leading", // windows strips it
      "trailing ", // windows strips it
      "nul\u0000byte", // control character, written as an escape: a raw one
      // makes this file binary to git and unreviewable in a diff
      "line\nbreak", // control
    ]) {
      expect(isValidDocumentName(bad), JSON.stringify(bad)).toBe(false);
    }
  });

  it("counts code points, not UTF-16 units, so a multi-byte script is not penalised", () => {
    // `"🙂".length` is 2, so a `.length` check would refuse this at half the
    // documented limit and a name in an astral script could never reach 64.
    const astral = "🙂".repeat(MAX_BASE_NAME_CHARS);
    expect([...astral]).toHaveLength(MAX_BASE_NAME_CHARS);
    expect(astral.length).toBe(MAX_BASE_NAME_CHARS * 2);
    expect(isValidBaseName(astral)).toBe(true);
    expect(isValidBaseName(astral + "🙂")).toBe(false);
  });

  it("gives a document more room than a base name", () => {
    expect(isValidBaseName("a".repeat(MAX_BASE_NAME_CHARS))).toBe(true);
    expect(isValidBaseName("a".repeat(MAX_BASE_NAME_CHARS + 1))).toBe(false);
    expect(isValidDocumentName("a".repeat(MAX_FILENAME_CHARS))).toBe(true);
    expect(isValidDocumentName("a".repeat(MAX_FILENAME_CHARS + 1))).toBe(false);
    // A filename the base-name limit would have refused.
    expect(isValidDocumentName("a".repeat(MAX_BASE_NAME_CHARS + 1))).toBe(true);
  });

  // Drift guards. The mirror is only worth having while it still says what the
  // server says, and the failure mode of a stale copy is silent: it accepts a
  // name the server refuses, and the operator meets a 400 the UI promised
  // could not happen.
  describe("stays in step with the Rust source", () => {
    it("refuses every code point in INVISIBLE_FORMAT_CHARS", () => {
      const invisible = rustCharConst(readFileSync(TEXT_RS, "utf8"), "INVISIBLE_FORMAT_CHARS");
      // Guard the guard: an empty parse would make every assertion below vacuous.
      expect(invisible.length).toBeGreaterThan(40);
      for (const char of invisible) {
        const point = char.codePointAt(0)!.toString(16).toUpperCase();
        // A name carrying U+202E renders as `invoiceexe.pdf` while being
        // stored as `invoice`+U+202E+`fdp.exe`. The name the operator reads is
        // not the name stored, or the one replayed to a model. Named rather
        // than embedded: a literal one would reverse this comment too.
        expect(isValidDocumentName(`invoice${char}fdp.exe`), `U+${point}`).toBe(false);
      }
    });

    it("refuses every code point in the tag block the server denies", () => {
      // Read the bounds off the Rust range literal rather than hardcoding them,
      // so widening it on either side fails here instead of drifting.
      const source = readFileSync(KNOWLEDGE_RS, "utf8");
      const match = /'\\u\{([0-9A-Fa-f]+)\}'\.\.='\\u\{([0-9A-Fa-f]+)\}'/.exec(source);
      expect(match, "the tag-block range literal was not found in knowledge.rs").not.toBeNull();
      const [lo, hi] = [parseInt(match![1], 16), parseInt(match![2], 16)];
      expect(hi).toBeGreaterThan(lo);

      for (let cp = lo; cp <= hi; cp++) {
        // U+E0069 U+E0067 U+E006E is a tag-encoded "ign": invisible to the
        // operator, ordinary ASCII to the model reading `file_list`.
        expect(
          isValidDocumentName(`handbook${String.fromCodePoint(cp)}.md`),
          `U+${cp.toString(16).toUpperCase()}`,
        ).toBe(false);
      }
      // Just past the block is an ordinary (unassigned) code point that the
      // server accepts, so refusing it here would be the stricter-than-server
      // bug in miniature.
      expect(isValidDocumentName(`handbook${String.fromCodePoint(hi + 1)}.md`)).toBe(true);
    });

    // `is_safe_name` also requires exactly one `Component::Normal`, which on
    // Windows refuses the drive-relative `C:evil.md`. That reading is
    // platform-dependent and the browser cannot know the daemon's OS, so the
    // mirror stays quiet and the server answers. Pinned so that "completing the
    // mirror" later is a deliberate decision rather than a tidy-up.
    it("leaves the platform-dependent path-component check to the server", () => {
      expect(isValidDocumentName("C:evil.md")).toBe(true);
    });

    it("refuses every Windows reserved stem, with or without an extension", () => {
      const source = readFileSync(KNOWLEDGE_RS, "utf8");
      const start = source.indexOf("const WINDOWS_RESERVED_STEMS");
      expect(start, "WINDOWS_RESERVED_STEMS not found in knowledge.rs").toBeGreaterThan(-1);
      const block = source.slice(start, source.indexOf("];", start));
      const stems = [...block.matchAll(/"([a-z0-9]+)"/g)].map(([, s]) => s);
      expect(stems.length).toBeGreaterThan(20);
      for (const stem of stems) {
        expect(isValidDocumentName(stem), stem).toBe(false);
        expect(isValidDocumentName(`${stem}.txt`), `${stem}.txt`).toBe(false);
        expect(isValidDocumentName(stem.toUpperCase()), stem.toUpperCase()).toBe(false);
        // Only the stem is reserved — the same letters inside a longer name are fine.
        expect(isValidDocumentName(`${stem}tract.md`), `${stem}tract.md`).toBe(true);
      }
    });

    it("uses the same two length limits as the server", () => {
      const source = readFileSync(KNOWLEDGE_RS, "utf8");
      expect(MAX_BASE_NAME_CHARS).toBe(rustUsize(source, "MAX_BASE_NAME_CHARS"));
      expect(MAX_FILENAME_CHARS).toBe(rustUsize(source, "MAX_FILENAME_CHARS"));
    });
  });
});
