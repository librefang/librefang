Skillhub search, browse, and detail lookups now degrade gracefully instead of failing with a cryptic "expected value at line 1 column 1" parse error.
`skillhub.tencent.com` redirects every API path to the `skillhub.cn` single-page app, which answers `200 OK` with an HTML shell instead of JSON — no verified replacement API endpoint exists, so a URL swap was not the fix.
`librefang-skills` now detects a markup response before handing it to `serde_json` and returns a distinct `SkillError::MarketplaceUnavailable` with an actionable message; the API layer maps it to `503 Service Unavailable` instead of `502` or a misleading `404`.
The check lives at the shared `ClawHubClient` parsing boundary too, so `get_skill` (used by both ClawHub and Skillhub) benefits without duplicating it.
The base URL is now configurable via `LIBREFANG_SKILLHUB_URL` so an operator can point at a working mirror without recompiling once one exists.
Local skill install and search on FangHub and ClawHub are unaffected (#7748) (@DaBlitzStein)
