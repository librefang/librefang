# [Low] LLM driver / MCP Low roundup — schema check, `token_url` http, `max_tokens=0`, `Url::origin`, stderr drain, `SystemAndN(0)`, `FallbackChain` doc drift, capability catalog

**Severity:** Low · **Domain:** LLM driver & MCP
**Status:** Merges 7 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | MCP tool args are not checked against `input_schema`; malformed LLM tool-calls reach the MCP server and produce confusing errors | runtime-mcp call boundary |
| token_url http | `oauth.token_url` accepts `http://` — only the SSRF recheck saves us | OAuth provider config |
| max_tokens=0 rewrite | Anthropic driver rewrites client `max_tokens=0` to 8192 — silently changes semantics | drivers/anthropic.rs |
| Url::origin opaque | `Url::origin()` opaque-origin pitfall: when origin is opaque, comparison returns false but the code assumes it won't | URL comparison site |
| stdio stderr drain | MCP stdio stderr drain leak: when the child's stderr buffer fills up, it blocks; we never drain it continuously | runtime-mcp stdio transport |
| SystemAndN(0) | Anthropic `SystemAndN(0)` cache misuse — n=0 miscomputes the cache key | drivers/anthropic.rs cache section |
| FallbackChain doc drift | `FallbackChain`'s `Timeout` semantics in the docs do not match the code | fallback_chain.rs |
| capability catalog | Capability negotiation should be catalog-driven, so adding a feature does not require editing multiple sites | capability negotiation |

## Combined fix plan

1. (this) Add a JSON-schema validation layer in the runtime: malformed args are rejected with a structured error; alternatively, document the "trust boundary" explicitly.
2. (token_url http) `oauth.token_url` allows only `https://` (dev overrides go through an env flag).
3. (max_tokens=0 rewrite) Treat `max_tokens=0` as client intent "use the provider default" and document it; do not silently rewrite to 8192.
4. (Url::origin opaque) Handle opaque origins explicitly in URL comparison — fail closed on the non-comparable case.
5. (stdio stderr drain) Continuously and asynchronously drain stderr into a ring buffer / log; never let the child block.
6. (SystemAndN(0)) Make `SystemAndN(0)` panic in debug and skip the cache (normalize at the driver boundary alongside `max_tokens=0`).
7. (FallbackChain doc drift) Align docs with code or the reverse — decide whether `Timeout` is per-attempt or total, and pin it down in a test.
8. (capability catalog) Move capability negotiation from hard-coded paths to a catalog file / reflection; CI verifies the catalog matches the runtime.

## Tests

- One unit test per item; the MCP schema-validation integration test lives in `tests/mcp_schema_validation.rs`.
