import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

// Per AGENTS.md: all dashboard API access must go through hooks in
// src/lib/queries/ and src/lib/mutations/. Pages and components must not
// call `fetch(` directly — that path bypasses TanStack Query, error
// translation, and authentication header injection.
//
// Documented exceptions (file downloads, SSE, raw probes) must be
// annotated on the line above the fetch with:
//
//     // lint-disable-next-line dashboard/no-inline-fetch -- <reason>
//
// This test scans the source tree at test time so the rule is enforced
// without adding ESLint as a build dependency.

const ROOT = join(__dirname, "..", "..");
const SCAN_DIRS = ["pages", "components"];
const ALLOWED_FILE_SUFFIXES = [".tsx", ".ts"];
const SKIP_FILE_SUFFIXES = [".test.tsx", ".test.ts"];

const FETCH_PATTERN = /(?<![a-zA-Z0-9_.])fetch\s*\(/;
const REFETCH_PATTERN = /\.refetch\s*\(/;
const DISABLE_PATTERN = /lint-disable-next-line\s+dashboard\/no-inline-fetch/;

function walk(dir: string, out: string[]): void {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    const st = statSync(full);
    if (st.isDirectory()) {
      walk(full, out);
      continue;
    }
    if (!ALLOWED_FILE_SUFFIXES.some((s) => entry.endsWith(s))) continue;
    if (SKIP_FILE_SUFFIXES.some((s) => entry.endsWith(s))) continue;
    out.push(full);
  }
}

function collectFiles(): string[] {
  const files: string[] = [];
  for (const sub of SCAN_DIRS) {
    walk(join(ROOT, sub), files);
  }
  return files;
}

function stripStringsAndComments(line: string): string {
  // Cheap blank-out of double/single/backtick strings and `// ...` tails.
  // Good enough to keep `"fetch("` literals and trailing `// fetch(...)`
  // comments from triggering the rule. Multi-line block comments that
  // wrap a fetch are rare; if they ever appear, add a disable annotation.
  let out = line.replace(/(["'`])(?:\\.|(?!\1).)*\1/g, '""');
  const slash = out.indexOf("//");
  if (slash >= 0) out = out.slice(0, slash);
  return out;
}

describe("no-inline-fetch", () => {
  it("pages and components route API access through hooks", () => {
    const violations: string[] = [];

    for (const file of collectFiles()) {
      const lines = readFileSync(file, "utf8").split("\n");
      for (let i = 0; i < lines.length; i++) {
        const code = stripStringsAndComments(lines[i]);
        if (!FETCH_PATTERN.test(code)) continue;
        if (REFETCH_PATTERN.test(code)) continue;
        const prev = i > 0 ? lines[i - 1] : "";
        if (DISABLE_PATTERN.test(prev)) continue;
        violations.push(`${relative(ROOT, file)}:${i + 1}: ${lines[i].trim()}`);
      }
    }

    expect(
      violations,
      `Inline fetch() calls in pages/components must go through src/lib/queries or src/lib/mutations.\nAdd "// lint-disable-next-line dashboard/no-inline-fetch -- <reason>" above genuine exceptions (file downloads, SSE, raw probes).\n\nViolations:\n${violations.join("\n")}`,
    ).toEqual([]);
  });
});
