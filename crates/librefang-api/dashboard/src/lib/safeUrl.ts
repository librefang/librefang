// safeUrl — URL-scheme validator for any href that originates outside
// the dashboard's own source (MCP catalog entries, agent outputs,
// plugin metadata, channel attachments, …).
//
// Background (audit: rel-noopener-mixed). The dashboard renders
// arbitrary external links from server-controlled catalogues. A
// malicious catalogue that returned `get_url: "javascript:alert(1)"`
// — or `vbscript:`, `data:text/html,…`, `file:` — would be wired
// straight into a `<a href={...}>` and fire JS / exfiltrate state in
// the dashboard's origin when clicked.
//
// safeUrl returns the original string only if the scheme matches one
// of `http:` / `https:` / `mailto:`, the schemes that have no
// in-context JS execution semantics in modern browsers. Everything
// else (including parse failures and protocol-relative URLs that the
// URL parser can't resolve without a base) returns null.
//
// Protocol-relative URLs (`//example.com/x`) get a synthetic base so
// they round-trip through the URL parser; if the result is `http`
// or `https` they're accepted.
//
// Callers should treat a `null` return as "do not render the link";
// the canonical pattern is `const safe = safeUrl(input); return safe
// ? <a href={safe} … /> : <span>{input}</span>;`.

const SAFE_SCHEMES = new Set(["http:", "https:", "mailto:"]);

export function safeUrl(input: string | undefined | null): string | null {
  if (!input || typeof input !== "string") {
    return null;
  }
  const trimmed = input.trim();
  if (trimmed === "") {
    return null;
  }
  // Try parsing with no base first — handles absolute schemes
  // (http:, https:, javascript:, …) and tells us which one we got.
  // Protocol-relative URLs (`//host/path`) fall through to the
  // second branch.
  try {
    const u = new URL(trimmed);
    return SAFE_SCHEMES.has(u.protocol) ? trimmed : null;
  } catch {
    // Fallthrough: not an absolute URL.
  }
  if (trimmed.startsWith("//")) {
    try {
      const u = new URL(`https:${trimmed}`);
      if (u.protocol === "https:") {
        return trimmed;
      }
    } catch {
      // ignore
    }
  }
  return null;
}
