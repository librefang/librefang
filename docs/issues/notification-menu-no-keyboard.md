# [Medium] Dashboard a11y & UX roundup — NotificationCenter / UsersPage / WS reconnect / Media data-URL

**Severity:** Medium · **Domain:** Dashboard
**Status:** Merges 3 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | NotificationCenter declares `role="menu"` but has no keyboard navigation / focus management | `dashboard/.../NotificationCenter.tsx:108` |
| UsersPage ARIA | UsersPage mixes `<details>` and `role="menu"` — ARIA contradiction | `dashboard/.../UsersPage.tsx` |
| WS retry | When WebSocket reconnect gives up, there is no retry button — users have no explicit recovery action | `dashboard/.../ws hook` |
| Media data-URL | MediaPage does not MIME-check data-URLs — a user-uploaded `data:text/html,...` becomes an XSS sink at render time | `dashboard/.../MediaPage.tsx` |

## Why merged

All four are dashboard frontend a11y / UX hygiene items; fixing them in the same review pass is sensible.

## Combined fix plan

1. **(this) Use Radix DropdownMenu or react-aria useMenu**: gets ↑↓ Esc Tab focus flow for free. Or hand-roll: open → focus first item, Esc → close + focus bell.
2. **(UsersPage ARIA) Pick one menu semantic**: choose `<details>` or `role="menu"`; do not mix. For a disclosure pattern → use `<details>` + `<summary>` and drop `role="menu"`.
3. **(WS retry) Explicit retry button**: after reconnect gives up, the UI shows "Connection lost — [Retry]"; clicking triggers a new WS attempt.
4. **(Media data-URL) data-URL MIME allowlist**: accept only `data:image/(png|jpe?g|gif|webp);base64,...`; reject `text/html` / `application/javascript` etc. Validate both before frontend render and before backend storage.

## Tests

- (this / UsersPage ARIA) E2E keyboard walk through NotificationCenter / UsersPage menus: ↑↓ Esc behaviour conforms to the ARIA APG. axe / pa11y reports no critical issues.
- (WS retry) Mock WS server refusing connection → UI shows the Retry button.
- (Media data-URL) Uploading `data:text/html,<script>` → backend 400; frontend declines to render.
