# [Low] Dashboard Low roundup — data-layer baseline, `commsKeys lists()`, localStorage raw, modal focus

**Severity:** Low · **Domain:** Dashboard
**Status:** Merges 3 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | Verified clean: the data-layer rule is broadly upheld (baseline) | dashboard, overall |
| commsKeys lists() | `commsKeys` factory is missing the `lists()` parent key — list invalidation cannot batch | `dashboard/.../lib/queries/keys.ts` |
| localStorage raw | `TerminalTabs.tsx` uses raw `localStorage` directly, bypassing the storage helper | `dashboard/.../TerminalTabs.tsx` |
| modal focus | The Modal `panel` variant has indeterminate initial focus — keyboard users lose focus on open | Modal component |

## Combined fix plan

1. (this) Keep the baseline and treat it as a review checklist item; any new PR adding `fetch()` or inline query keys fails review.
2. (commsKeys lists()) Add the `all` / `lists()` / `list(filters)` / `details()` / `detail(id)` hierarchy to `commsKeys`, mirroring already-standardized factories such as `agentKeys`.
3. (localStorage raw) Move `TerminalTabs.tsx` to the unified `lib/storage.ts` API; delete the direct `localStorage` call.
4. (modal focus) The Modal `panel` variant defines `autoFocus` behaviour explicitly — the first interactive element, or the close button.
