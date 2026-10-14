import type { QueryClient } from "@tanstack/react-query";
import type { AgentManifestSnapshot } from "../api";
import { agentQueries } from "./queries/agents";

/**
 * Re-read the manifest ETag after a write the manifest form did not make
 * (#8424).
 *
 * The grant panels and their modals write through their own endpoints, so the
 * token the form was seeded with no longer describes the server's manifest.
 * Reading it back keeps the next form save from failing with 409 on a write
 * the user did make.
 *
 * The `staleTime: 0` override is the whole point: `createDashboardQueryClient`
 * defaults every query to a 30 s `staleTime`, and `fetchQuery` inside that
 * window returns the cached snapshot without touching the server. That made
 * this refresh a no-op — the form kept its pre-panel ETag and the next save
 * 409'd. Pinning the fetch stale forces the network read regardless of the
 * cache's freshness.
 */
export async function fetchManifestVersion(
  qc: QueryClient,
  agentId: string,
): Promise<string | null> {
  return (await fetchManifestSnapshot(qc, agentId)).version;
}

/**
 * The whole fresh manifest read — the TOML and the ETag that describes it — forced past the query client's `staleTime` for the same reason as `fetchManifestVersion`.
 *
 * A caller that moves the editor onto a new token must also adopt the content that token describes, which is why this exists next to the token-only read: echoing a fresh token with the form's stale copy is exactly the lost update the token was introduced to prevent (#8516).
 */
export async function fetchManifestSnapshot(
  qc: QueryClient,
  agentId: string,
): Promise<AgentManifestSnapshot> {
  return qc.fetchQuery({
    ...agentQueries.manifest(agentId),
    staleTime: 0,
  });
}

type PlainObject = Record<string, unknown>;

function isPlainObject(value: unknown): value is PlainObject {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const proto = Object.getPrototypeOf(value);
  return proto === Object.prototype || proto === null;
}

/**
 * Structural equality that ignores `_uid`.
 *
 * Row collections (`metadata`, `workspaces`, …) carry a `_uid` the parser mints for React keys on every parse, so two parses of the same TOML never agree on it; comparing it would make every row list look edited by everyone.
 */
function sameManifestValue(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((item, i) => sameManifestValue(item, b[i]));
  }
  if (isPlainObject(a) && isPlainObject(b)) {
    const keysA = Object.keys(a).filter((k) => k !== "_uid" && a[k] !== undefined);
    const keysB = Object.keys(b).filter((k) => k !== "_uid" && b[k] !== undefined);
    if (keysA.length !== keysB.length) return false;
    return keysA.every((k) => Object.prototype.hasOwnProperty.call(b, k) && sameManifestValue(a[k], b[k]));
  }
  if (a instanceof Date && b instanceof Date) {
    // TOML date-times parse to `TomlDate`, a `Date` subclass whose `toISOString` keeps the local and date-only forms apart.
    return a.toISOString() === b.toISOString();
  }
  return false;
}

function rebaseValue(base: unknown, mine: unknown, theirs: unknown): unknown {
  if (sameManifestValue(mine, base)) return theirs;
  if (sameManifestValue(theirs, base)) return mine;
  if (isPlainObject(base) && isPlainObject(mine) && isPlainObject(theirs)) {
    const out: PlainObject = {};
    for (const key of new Set([...Object.keys(base), ...Object.keys(mine), ...Object.keys(theirs)])) {
      const merged = rebaseValue(base[key], mine[key], theirs[key]);
      if (merged !== undefined) out[key] = merged;
    }
    return out;
  }
  // Both sides changed the same leaf (or a list, which is one value): the edit in this form wins, because adopting it is what the operator is about to save.
  return mine;
}

/**
 * Three-way merge of the manifest editor's state onto a fresh read (#8516).
 *
 * `base` is the parse the form was seeded from (or last rebased onto), `mine` is the form as the operator has edited it, and `theirs` is the parse of the manifest the server holds now.
 * A field the operator did not touch takes the server's value, a field only the operator changed keeps the edit, and a field both changed keeps the edit.
 * Tables are merged key by key, so an edit to one `[compaction]` knob does not pin the rest of the table to the seed; lists are a single value.
 *
 * This is what makes adopting a new ETag safe: a token that moves without the form adopting the content it describes lets the next save write the stale copy back over the intervening change, under a token the server accepts.
 */
export function rebaseManifestEdits<T>(base: T, mine: T, theirs: T): T {
  return rebaseValue(base, mine, theirs) as T;
}
