# Three-layer rebrand principle

Every change in the BossFang fork classifies into one of three layers.
This is the load-bearing rule for keeping upstream merges cheap.

## Layer Internal — DO NOT TOUCH (~532+ Rust files)

Renaming any of these guarantees a conflict on every future upstream
merge. Take upstream's version even when they refactor an internal
symbol — our overlays don't depend on these names.

**Examples**:

- Cargo crate names: `librefang-types`, `librefang-kernel`, `librefang-runtime`, `librefang-api`, `librefang-storage`, `librefang-llm-drivers`, `librefang-skills`, `librefang-extensions`, `librefang-channels`, `librefang-desktop`, `librefang-uar-spec`, `librefang-memory`, …
- Module paths: `use librefang_runtime::tool_runner`, `use librefang_kernel::scheduler`
- Struct / enum / trait names: `LibreFangKernel`, `LibreFangError`, `LibreFangConfig`, `KernelHandle`, `AgentManifest`
- Function names: `librefang_home()`, `librefang_dirs()`, anything starting with `librefang_*`
- Python module names exported by the SDK: `librefang_sdk`, `librefang_client`
- Cargo workspace member entries (the paths under `[workspace] members = [...]`)
- Test names and test fixture identifiers
- Internal binary `[[bin]]` `path` (the source file path); ONLY the `name` is overridable

## Layer Boundary — ADDITIVE ALIASES ONLY

User-controllable settings that already have a `LIBREFANG_*` name. Add
a `BOSSFANG_*` alias as the new primary; keep the legacy form working
forever (or until the user deliberately drops it).

**Aliased today** (post Phase-2 PRs):

| Primary | Fallback | Resolution helper | File |
|---|---|---|---|
| `BOSSFANG_HOME` | `LIBREFANG_HOME` | `librefang_home()` | `crates/librefang-kernel/src/config.rs:431` |
| `BOSSFANG_VAULT_KEY` | `LIBREFANG_VAULT_KEY` | `vault_key_from_env()` / `vault_key_env()` | `crates/librefang-extensions/src/vault.rs`, `crates/librefang-cli/src/doctor.rs:148` |

**Default values stay LibreFang-flavoured** even when the variable
name is BossFang. Notably the on-disk home directory defaults to
`~/.librefang/`, NOT `~/.bossfang/`. Renaming the default path would
orphan every existing user's config / vault / registry cache. Power
users on a fresh install can `export BOSSFANG_HOME=~/.bossfang` and
pay the migration cost themselves.

**Deliberate non-goal**: alias every `LIBREFANG_*` env var the
codebase reads. There are 70+ of them; merge cost outweighs benefit.
Most aren't user-typed (operational plumbing). Adding new aliases is
a one-line change in the right helper — easy to do later if pain
emerges. Don't pre-emptively flood the boundary layer.

## Layer Surface — FULL RENAME OK

Everything a user reads, types at the shell, or downloads. Always-
take-ours during conflict resolution; flip new upstream additions to
BossFang during the audit pass.

**Surface inventory** (Phase-2 PR-D landed):

- **Binary names** (`crates/librefang-cli/Cargo.toml`): `[[bin]]` `bossfang` (primary) + `librefang` (legacy alias), both from `src/main.rs`
- **CLI help text** (`crates/librefang-cli/src/main.rs`): clap `#[command(name = "bossfang", ...)]`, AFTER_HELP examples, Quick Start, every subcommand's `long_about`
- **Distribution packages**:
  - npm: `@bossfang/sdk` (`sdk/javascript/package.json`)
  - PyPI: `bossfang-sdk` (`sdk/python/setup.py`, Python module names `librefang_sdk` / `librefang_client` STAY for import compat)
  - Cargo: `bossfang-sdk` (`sdk/rust/Cargo.toml`)
- **Workspace metadata** (`Cargo.toml`): `repository = "https://github.com/GQAdonis/librefang"`, `homepage` same
- **Tauri**: `productName: "BossFang"`, `identifier: "ai.bossfang.{desktop,app}"`, BossFang descriptions in all four `tauri.*.conf.json`
- **Dashboard chrome**: window title "BossFang Dashboard", `manifest.json` name/short_name, `index.html` title and meta tags
- **Ember palette tokens** in `crates/librefang-api/dashboard/src/index.css` `:root` / `:root.dark` blocks
- **`boss-libre.png` logo** referenced from `App.tsx` sidebar + mobile header, plus `/boss-libre.png` route via `webchat.rs`
- **User-agent strings** in 5 sites: `BossFang/0.1` (skillhub 4x, clawhub 1x), `bossfang-skills/0.1` (marketplace), `bossfang-plugin-{updater,search}/1.0` (plugins routes), `BossFang-Webhook/1.0` (webhooks). Plus `SkillMeta { author: "BossFang" }` in `openclaw_compat`.
- **Content-Type vendor prefix**: primary `application/vnd.bossfang.v1+json`; parser also accepts legacy `application/vnd.librefang.*` (back-compat). Cf. `crates/librefang-api/src/versioning.rs`.
- **Release artifact names** in `.github/workflows/release{,-cli,-desktop}.yml`: `bossfang-<target>.tar.gz`/`.zip`, install script URL `github.com/GQAdonis/librefang/raw/main/install.sh`
- **README.md, CONTRIBUTING.md**: product name (`LibreFang` → `BossFang`), URLs (`librefang.ai` → `github.com/GQAdonis/librefang`), shell-command examples (`librefang init` → `bossfang init`)

**What's intentionally still LibreFang-named in user-visible places**:

- Historical attribution in `README.md` ("BossFang is a community fork of `RightNow-AI/openfang`")
- Internal crate-name mentions in the README "crate map" section (the crate IS `librefang-cli`, that's not rebrandable without exploding Layer Internal)

## Decision rule when in doubt

Ask: "if I rename this, how many merge conflicts does the next upstream pull cause?"

- Zero or one (a config / metadata file) → Surface, rename freely
- A handful (a small refactor surface) → Boundary, alias additively
- Dozens (a hot edit zone, an identifier referenced in many files) → Internal, leave alone

The number isn't the only signal — also weigh how much the change benefits
end-user clarity. A user reading "LibreFang" in CLI help confuses them;
a developer reading `librefang_runtime::` doesn't, because it's clearly
a stable internal name.
