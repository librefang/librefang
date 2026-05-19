# [Critical] Audit CSV/JSON export downloads silently 401 since #3620

**Severity:** Critical
**Domain:** Dashboard

## Location

`crates/librefang-api/dashboard/src/pages/AuditPage.tsx:110`

```typescript
safeStorageGet("librefang-api-key")
```

## Problem

`safeStorageGet("librefang-api-key")` only checks `localStorage`. The current credential layer (`api.ts:1014-1023 getStoredApiKey()`) prefers `sessionStorage` and **wipes `localStorage` on save** (`api.ts:3341-3342`). So:

1. User logs in → token stored in `sessionStorage`, `localStorage` wiped.
2. User clicks "Export audit log".
3. `AuditPage` reads `localStorage` → empty → sends `Authorization: Bearer ` (empty) → 401.
4. ApiError surfaces to the user with no actionable hint.

The export has been broken for every user since the storage migration in #3620.

## Fix

Use the canonical accessor:

```typescript
import { getStoredApiKey } from '../lib/api';
const token = getStoredApiKey();
```

Audit other call sites in `AuditPage.tsx` and the rest of `pages/` for the same anti-pattern — any inline `safeStorageGet("librefang-api-key")` is suspect.

## Tests

- E2E (Playwright): log in, navigate to Audit page, click Export → file downloads with non-empty body.
- Lint rule / grep gate against `safeStorageGet("librefang-api-key")` in `pages/` to catch future regressions.
