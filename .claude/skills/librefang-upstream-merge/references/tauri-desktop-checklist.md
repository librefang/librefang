# Tauri desktop merge checklist

When upstream changes the desktop crate, BossFang inherits the Rust
side (commands, IPC, lifecycle) but keeps its own branding overlays.
The `crates/librefang-desktop/` crate doesn't depend on
`librefang-storage` directly — the kernel it embeds owns the
SurrealDB layer — so storage conversion is a non-issue here.

## Files that ARE BossFang-overlay (always-take-ours)

| File | Field(s) we own | Notes |
|---|---|---|
| `tauri.conf.json` | `productName`, `identifier`, `bundle.shortDescription`, `bundle.longDescription` | Take upstream's `version`, new `bundle` options, new security config keys; merge by hand |
| `tauri.desktop.conf.json` | `identifier`, `plugins.updater.endpoints[0]`, `plugins.updater.pubkey` | Take upstream's other updater options (`windows.installMode`, etc.) |
| `tauri.ios.conf.json` | `identifier` | Take upstream's `bundle.iOS.*` and `app.windows` additions |
| `tauri.android.conf.json` | `identifier` | Take upstream's `bundle.android.*` and `app.security` additions |
| `icons/icon.ico` | binary BossFang artwork | Never overwrite from upstream |
| `icons/icon.png` | binary BossFang artwork | Never overwrite |
| `icons/32x32.png` | binary BossFang artwork | Never overwrite |
| `icons/128x128.png` | binary BossFang artwork | Never overwrite |
| `icons/128x128@2x.png` | binary BossFang artwork | Never overwrite |
| `frontend/connection.html` | title, ember palette, brand image | If upstream adds a new connection screen, take their layout + flow but flip palette tokens |
| `frontend/boss-libre.png` | BossFang logo | Replace upstream's `logo.png` references with `boss-libre.png` if introduced |

## Files that take-upstream

- Everything in `crates/librefang-desktop/src/*.rs` — Rust code (commands, server, connection logic, tray, updater glue, shortcuts)
- `crates/librefang-desktop/build.rs`
- `crates/librefang-desktop/capabilities/` — Tauri capability ACLs
- `crates/librefang-desktop/gen/` — auto-generated platform code (Tauri rewrites this)
- `crates/librefang-desktop/MOBILE.md` — mobile build instructions
- `crates/librefang-desktop/Cargo.toml` — except if upstream adds a SurrealDB dep (then it duplicates `librefang-storage`'s pin; investigate before accepting)

## Post-merge audit (per `scripts/audit-tauri-desktop.sh`)

The audit script checks four things after merge:

1. **productName** in `tauri.conf.json` is `"BossFang"`. Failure: upstream merged their `"LibreFang"` value back in. Fix: edit the file, restore `"BossFang"`.

2. **identifier**:
   - `tauri.conf.json`: `ai.bossfang.desktop`
   - `tauri.desktop.conf.json`: `ai.bossfang.desktop`
   - `tauri.ios.conf.json`: `ai.bossfang.app`
   - `tauri.android.conf.json`: `ai.bossfang.app`

3. **Updater endpoint** in `tauri.desktop.conf.json`:
   - Host must be `github.com/GQAdonis/librefang/...` (NOT `github.com/librefang/librefang`)

4. **Minisign pubkey** in `tauri.desktop.conf.json`:
   - Key ID must be `E329A6B2863F1707` (NOT upstream's `BC91908BD3F1520D`)
   - The key ID lives in the decoded comment line of the base64 pubkey blob. Decode with: `echo '<pubkey>' | base64 -d | head -1`
   - Expect: `untrusted comment: minisign public key E329A6B2863F1707`

## When upstream upgrades Tauri version

If upstream bumps the Tauri version (`tauri = { workspace = true, ... }`
in `Cargo.toml`), the schema URL inside the `*.conf.json` files may
change too (`https://schema.tauri.app/config/2` → `/3`). Take
upstream's schema URL but keep our values inside the schema.

Also check `crates/librefang-desktop/capabilities/` for new ACL files
upstream added — Tauri 2 capability files are usually backward-
compatible, but a major version bump may invalidate old paths.

## When upstream changes desktop's storage approach

The current state (verified): desktop has zero direct dependency on
`librefang-storage` or any DB crate. It calls `LibreFangKernel::boot(None)`
which internally selects the SurrealDB backend via the default-features
chain on `librefang-api` / `librefang-runtime`. Connection.rs persists
a single field (`server_url`) to `~/.librefang/desktop.toml` as plain
TOML, not via a DB.

If upstream introduces a direct DB dependency in the desktop crate
(e.g. they decide to cache local-only state in a SQLite file at
`~/.librefang/desktop.db`):

1. Don't take the SQLite dep — replace with SurrealDB embedded 3.0.5.
2. Add a new SurrealDB migration if applicable (`refs surrealdb-migrations.md`).
3. Adjust the connection logic to use SurrealDB's record IDs / queries.
4. Pin the same `surrealdb = "=3.0.5"` workspace dep — never let
   desktop's version diverge from `librefang-storage`'s pin.

## Building (NOT in this skill's scope)

The actual `cargo tauri build` (or `cargo build -p librefang-desktop --release`)
is forbidden in the worktree by the `forbid-main-worktree.sh` hook.
Surface the build command to the user for them to run locally:

```bash
# Workspace-cargo build (no Tauri CLI needed for a basic compile)
cargo build --release -p librefang-desktop

# Full Tauri bundle (needs tauri-cli)
cargo install tauri-cli --version "^2"
cd crates/librefang-desktop
cargo tauri build
```

The bundle lands in `target/release/bundle/`. Verify on macOS with:

```bash
APP="target/release/bundle/macos/BossFang.app"
plutil -p "$APP/Contents/Info.plist" | grep -E 'CFBundleIdentifier|CFBundleName'
# expect: CFBundleIdentifier = "ai.bossfang.desktop", CFBundleName = "BossFang"
```
