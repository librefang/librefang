# BossFang ember palette tokens

Canonical source: `docs/branding/branding-guide.html`. This is the
short reference the merge skill uses to identify a brand-token
regression in a diff or apply a manual fix when `enforce-branding.py`
can't catch something (typically: a new SVG fang glyph, a CSS variable
upstream renamed, a hardcoded hex value in inline JSX style props).

## Brand anchors

| Token | Light mode | Dark mode |
|---|---|---|
| Primary brand color (`--brand-color`) | `#E04E28` (Muted Ember) | `#FF6A3D` (Bright Ember) |
| Brand muted (`--brand-muted`) | `#F2D6CF` (Ember blush) | `rgba(255,106,61,0.15)` |
| Main background (`--bg-main`) | `#F7F7F8` | `#0B0F14` (Deep Charcoal) |
| Surface (`--bg-surface`) | `#FFFFFF` | `rgba(15,23,42,0.92)` |
| Surface hover (`--bg-surface-hover`) | `#F1F5F9` | `rgba(30,41,59,0.75)` |

## Logo asset

- File: `crates/librefang-api/dashboard/public/boss-libre.png` (Vite-bundled at build)
- Source-of-truth: `docs/branding/boss-libre.png` (4.2 MB)
- API route: `/boss-libre.png` served by `crates/librefang-api/src/webchat.rs::boss_libre_png()`
- Referenced in: `crates/librefang-api/dashboard/src/App.tsx` (sidebar + mobile header), `crates/librefang-desktop/frontend/connection.html`

## Typography (informational)

The branding guide also specifies fonts; these come in via the
dashboard's `index.html` Google Fonts link and don't typically conflict
in merges:

- Space Grotesk — display headings
- Inter — UI text
- Roboto — body text
- JetBrains Mono — code blocks

## Upstream sky-blue tokens forbidden in the BossFang source tree

`scripts/enforce-branding.py --check` exits non-zero if any of these
appear under the scanned directories
(`crates/librefang-api/dashboard/src/`, `crates/librefang-api/static/`,
`crates/librefang-desktop/frontend/`, `crates/librefang-desktop/src/`):

- `#0284c7`, `#38bdf8`, `#0ea5e9` (sky-blue hex)
- `#020617` (slate-950 dark background)
- `rgba(2,132,199,...)`, `rgba(56,189,248,...)`, `rgba(14,165,233,...)` (sky-blue rgba variants)
- `rgba(2,6,23,...)` (slate-950 rgba)
- `linear-gradient(135deg,#a78bfa,#7c3aed)` (upstream's purple avatar gradient)

## What the enforcement script does NOT fix

`enforce-branding.py` is token-substitution only. It does not catch:

- New TSX components rendering an SVG fang glyph inside a sky-blue gradient box — manually replace with `<img src="/boss-libre.png" alt="BossFang" ...>` per `references/conflict-resolution.md`
- The `card-glow` / `glow-text` CSS helpers if upstream rewrites them with hardcoded sky-blue `rgba(...)` outside the script's pattern list — inspect `index.css` after merge
- Product-name string flips ("LibreFang" → "BossFang") in newly-added JSX — these need manual review since the script avoids touching identifier-style strings (function names, CSS class names) that legitimately contain "librefang"
- New hardcoded `github.com/librefang` / `librefang.ai` URLs in upstream Rust source — covered by `scripts/scan-hardcoded-urls.sh` (Phase 3b), not the brand script
