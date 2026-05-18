# [Medium] Dashboard data-layer roundup — i18n escapeValue, storage naming divergence, mutation invalidation marker

**Severity:** Medium · **Domain:** Dashboard
**Status:** Merges 2 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | i18n `escapeValue: false` + `dangerouslySetInnerHTML` — any translator-supplied string containing HTML becomes an XSS landmine | `dashboard/.../lib/i18n.ts:19`; consumers `MobilePairingPage.tsx`, `ConnectWizardPage.tsx` |
| storage naming | localStorage carries both `lf_creds` and `librefang-api-key` — divergent key names | `dashboard/.../lib/storage.ts` |
| no-invalidate marker | Some mutations deliberately do not invalidate queries but carry no marker, conflicting with the CLAUDE.md data-layer "must invalidate" rule | `dashboard/.../lib/mutations/*.ts` |

## Why merged

All three are dashboard data-layer / security hygiene items that touch the same file group.

## Combined fix plan

1. **(this) Pick one**:
   - `escapeValue: true` + use the `<Trans>` component for any HTML-bearing strings; or
   - Keep `escapeValue: false` but add an ESLint rule forbidding `dangerouslySetInnerHTML` outside a single audited helper (`lib/i18n-html.ts`).
2. **(storage naming) Unify on `librefang-api-key`**: migrate `lf_creds` → `librefang-api-key`; after reading the old key, immediately rewrite to the new key and delete the old. Document the storage schema.
3. **(no-invalidate marker) Explicit marker**: when a mutation hook deliberately skips invalidation, require an inline `// no-invalidate: <reason>` comment, enforced via ESLint — any hook missing invalidation without the marker fails.

## Tests

- (this) ESLint rule in CI fails any PR that adds `dangerouslySetInnerHTML` outside the i18n-html helper.
- (storage naming) After migration, the `lf_creds` key is read or written by no code.
- (no-invalidate marker) ESLint example error: mutation hook without invalidation and without a `no-invalidate` comment.
