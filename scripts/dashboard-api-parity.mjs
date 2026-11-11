#!/usr/bin/env node
// Inventory of HTTP routes (openapi.json) against the dashboard source.
//
// The dashboard reaches the API through `api.ts` and a few page helpers, mostly
// with template strings such as `/api/agents/${id}`. Matching whole paths would
// report every parameterised route as missing, so this groups routes by their
// first static segments and looks for that prefix in `dashboard/src`. A route
// marked "covered" means some dashboard code names that group somewhere; it
// does not prove the screen exists.
//
// Usage:
//   node scripts/dashboard-api-parity.mjs           # markdown table
//   node scripts/dashboard-api-parity.mjs --gaps    # uncovered routes only
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const root = new URL("..", import.meta.url).pathname;
const spec = JSON.parse(readFileSync(join(root, "openapi.json"), "utf8"));
const srcRoot = join(root, "crates/librefang-api/dashboard/src");

function walk(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) walk(path, out);
    else if (/\.(ts|tsx)$/.test(entry)) out.push(readFileSync(path, "utf8"));
  }
  return out;
}
const sources = walk(srcRoot).join("\n");

// Longest static prefix of a path: stop at the first `{param}`.
function staticPrefix(path) {
  const parts = path.split("/").filter(Boolean);
  const out = [];
  for (const part of parts) {
    if (part.startsWith("{")) break;
    out.push(part);
  }
  return "/" + out.slice(0, 3).join("/");
}
const groupOf = (prefix) => "/" + prefix.split("/").filter(Boolean).slice(0, 2).join("/");

const groups = new Map();
for (const path of Object.keys(spec.paths).sort()) {
  const prefix = staticPrefix(path);
  const key = prefix === "/" ? path : groupOf(prefix);
  if (!groups.has(key)) groups.set(key, { total: 0, covered: 0, gaps: [] });
  const group = groups.get(key);
  group.total += 1;
  if (prefix !== "/" && sources.includes(prefix)) group.covered += 1;
  else group.gaps.push(path);
}

const rows = [...groups.entries()].sort((a, b) => a[0].localeCompare(b[0]));
const routes = Object.keys(spec.paths).length;
const showGaps = process.argv.includes("--gaps");

if (showGaps) {
  for (const [group, g] of rows) {
    for (const path of g.gaps) console.log(`${group}\t${path}`);
  }
  process.exit(0);
}

console.log("| Group | Routes | Seen in dashboard/src | Unreferenced |");
console.log("|---|---:|---:|---:|");
for (const [group, g] of rows) {
  console.log(`| \`${group}\` | ${g.total} | ${g.covered} | ${g.total - g.covered} |`);
}
const referenced = rows.reduce((n, [, g]) => n + g.covered, 0);
const withGaps = rows.filter(([, g]) => g.gaps.length > 0).length;
console.log(
  `\n${referenced}/${routes} routes are referenced by the dashboard source ` +
    `across ${rows.length} groups (${withGaps} groups with gaps).`,
);
