# Origin-repoint knobs

Four mechanisms keep BossFang installs talking to GQAdonis-owned URLs
by default. The plumbing is already landed (Phase-1 + Phase-2 PRs);
this doc lists where each lives so a merge audit can verify upstream
hasn't introduced a new hardcoded URL that bypasses them.

## 1. Registry source — `[registry] base_url`

**Config field**: `crates/librefang-types/src/config/types.rs`
`RegistryConfig.base_url`

**Default**: `"https://github.com/GQAdonis/librefang-registry"` (set
by `default_registry_base_url()` near the field definition)

**Plumbing**: `crates/librefang-runtime/src/registry_sync.rs`
`resolve_registry_urls(base_url: &str)` derives `(tarball_url,
git_clone_url, tarball_prefix)`. Empty string → falls back to the
historical upstream constants. Threaded through:

- `sync_registry()` (kernel boot path)
- `refresh_registry_checkout()` (catalog-only refresh from API)
- `catalog_sync::sync_catalog_to()` (the API endpoint behind `POST /api/catalog/update`)

Four external call sites flow the config field through:

- `crates/librefang-kernel/src/kernel/boot.rs` (boot)
- `crates/librefang-cli/src/bundled_agents.rs` (CLI init)
- `crates/librefang-api/src/server.rs` (background sync task)
- `crates/librefang-api/src/routes/providers.rs` (`POST /api/catalog/update`)

**Audit signal**: an upstream merge that adds a new `sync_registry`-style
call site without plumbing through `config.registry.base_url` is a
regression. Grep `git diff HEAD..upstream/main -- '*.rs'` for
`REGISTRY_TARBALL_URL` or `librefang/librefang-registry` references in
new code.

## 2. Dashboard release tarball

**File**: `crates/librefang-api/src/webchat.rs:378`

**Default URL** (hardcoded, not a config knob): `https://github.com/GQAdonis/librefang/releases/latest/download/dashboard-dist.tar.gz`

**Why no config knob**: the BossFang fork defaults to
`LIBREFANG_DASHBOARD_EMBEDDED_ONLY=true`, which short-circuits
`sync_dashboard()` entirely. The hardcoded URL is reached only when an
operator explicitly opts out of embedded mode. Plumbing a config knob
for a code path that's dead in the default flow was deemed
over-engineering during PR #1's planning.

**Audit signal**: an upstream merge that changes this URL must be
re-flipped to the GQAdonis form. Grep `webchat.rs` for
`librefang/librefang/releases`.

## 3. Skills marketplace org — `[skills.marketplace] github_org`

**Config field**: `crates/librefang-skills/src/marketplace.rs`
`MarketplaceConfig.github_org`

**Default**: `"GQAdonis"` (set in `MarketplaceConfig::default()` near
the field)

**Plumbing**: `MarketplaceClient::new(MarketplaceConfig)` passes the org
through to GitHub search API calls (`org:<github_org>` filter).

**Note**: third-party marketplaces (ClawHub, SkillHub Tencent) are
**not** repointed — they're independent service providers, not
LibreFang-owned. Leave those alone.

**Audit signal**: an upstream merge that adds a new
`MarketplaceConfig::default()` call site bypassing the user's config
is a regression. Grep `marketplace.rs` for `"librefang-skills"`
literal.

## 4. Tauri auto-updater endpoint

**File**: `crates/librefang-desktop/tauri.desktop.conf.json`

**Fields**:
- `identifier: "ai.bossfang.desktop"`
- `plugins.updater.pubkey` — BossFang minisign pubkey (key ID
  `E329A6B2863F1707`), base64-encoded
- `plugins.updater.endpoints[0]` — `https://github.com/GQAdonis/librefang/releases/latest/download/latest.json`

**No config knob** — Tauri reads this JSON at build time. Direct file
edit is the only override path.

**Companion**: GitHub repo secrets `TAURI_SIGNING_PRIVATE_KEY` (full
minisign secret-key file contents) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
(empty for a no-password keypair). Set via `gh secret set ... < ~/.tauri/bossfang.key`.

**Audit signal**: upstream version bumps touch this file (the `version`
field). Always-take-ours during conflict resolution; let upstream's
version-number changes through but keep our `identifier`, `pubkey`,
and `endpoints[0]`.

## Plugin signing trust root (DEFERRED)

The plugin-registry signing pubkey is **not yet rotated** to a BossFang
trust root. BossFang users still verify signed plugins against
upstream's `librefang.ai/.well-known/registry-pubkey`. Three override
mechanisms exist for operators who want to flip locally:

- `LIBREFANG_REGISTRY_PUBKEY` env var (base64 pubkey, takes priority)
- `~/.librefang/registry.pub` TOFU cache file (auto-populated on first fetch)
- `LIBREFANG_REGISTRY_PUBKEY_URL` env var (custom fetch endpoint, default `https://librefang.ai/.well-known/registry-pubkey`)

Tracked as a follow-up. When you eventually rotate, generate an
ed25519 keypair, host the pubkey at a stable URL (recommended path:
`https://github.com/GQAdonis/librefang/raw/main/.well-known/registry-pubkey`),
and set the env vars accordingly.

## What NOT to repoint

- **ClawHub** (`crates/librefang-skills/src/clawhub.rs`) — third-party Anthropic-adjacent marketplace, BossFang has no equivalent
- **SkillHub Tencent** (`crates/librefang-skills/src/skillhub.rs`) — third-party, regional
- **GitHub API base URL** (`https://api.github.com`) — that's GitHub's API, not a librefang surface
- **Provider-specific endpoints** (Anthropic, OpenAI, Gemini, etc.) — those are LLM providers, unrelated to fork origin
- **MCP server registry URLs** unless they're a librefang-owned domain
