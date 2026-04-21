/**
 * Format a serialized `TriggerPattern` (from `/api/triggers`) into a short
 * human-readable label for list rendering.
 *
 * Serde default-serializes Rust enums without a tag, so unit variants like
 * `Lifecycle` come over the wire as the string `"lifecycle"`, but struct
 * variants like `AgentSpawned { name_pattern }` come as an object shaped
 * `{ "agent_spawned": { "name_pattern": "..." } }`. Rendering that object
 * directly in JSX throws "Objects are not valid as a React child" and
 * blanks the page — see issue #2703.
 *
 * Returns `undefined` when the input is missing or shaped in a way we
 * don't recognize, so callers can fall back to a different label (e.g. the
 * trigger ID) instead of rendering junk.
 */
function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

export function formatTriggerPattern(pattern: unknown): string | undefined {
  if (pattern == null) return undefined;
  if (typeof pattern === "string") return pattern;
  if (typeof pattern !== "object") return undefined;
  if (!isRecord(pattern)) return undefined;

  const entries = Object.entries(pattern);
  if (entries.length === 0) return undefined;
  const [variant, payload] = entries[0];

  if (payload == null || typeof payload !== "object") {
    return variant;
  }
  if (!isRecord(payload)) return variant;

  const values: string[] = [];
  for (const v of Object.values(payload)) {
    if (typeof v === "string" && v.length > 0) {
      values.push(v);
    }
  }
  return values.length > 0 ? `${variant}: ${values.join(" ")}` : variant;
}
