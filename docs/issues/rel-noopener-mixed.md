# [High] Dashboard XSS hardening — `rel=noopener noreferrer` + `javascript:` URL guard

**Severity:** High · **Domain:** Dashboard
**Status:** Merges 1 earlier issue into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | Multiple `target="_blank"` sites set only `rel="noreferrer"` or omit `rel` entirely — pre-16.4 Safari and WebViews do not imply `noopener`, allowing reverse tabnabbing | `dashboard/.../MediaPage.tsx:348,469,618,723`, `PluginsPage.tsx:81,362`, `ChatPage.tsx:1261,1278`, `WorkflowStepImageGallery.tsx:27` |
| javascript: URL XSS | MCP catalog `get_url` accepts `javascript:` scheme — XSS via a malicious catalog URL | `dashboard/.../MCPCatalogPage.tsx`, the `get_url` field |

## Why merged

Both are dashboard XSS-class issues; unified ESLint + URL-scheme validator covers both in one go.

## Combined fix plan

1. **(this) Standardize on `rel="noopener noreferrer"` everywhere**: replace every existing `target="_blank"` site. Add ESLint:
   ```json
   "react/jsx-no-target-blank": ["error", { "allowReferrer": false, "enforceDynamicLinks": "always" }]
   ```
2. **(javascript: URL XSS) URL scheme validator**: introduce a `safeUrl(input: string): string | null` helper that rejects `javascript:` / `data:` / `vbscript:` / `file:` and accepts only `http:` / `https:` / `mailto:`. All catalog / external-link inputs must pass through it.
3. **Tighten CSP** (related): remove `style-src 'unsafe-inline'` (see the "registry-content-abs-path-leak" cluster).

## Tests

- ESLint in CI fails any PR that adds a `target="_blank"` without `rel="noopener noreferrer"`.
- Unit test: `safeUrl("javascript:alert(1)")` returns `null`; `safeUrl("https://x")` passes.
- E2E: inject a `javascript:` URL into the MCP catalog → renders as non-clickable text without firing JS.
