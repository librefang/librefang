# Release versioning policy

Operational reference for tagging, pre-releases, and how publish jobs pick
their distribution tags. Sister doc to `docs/architecture/`; lives at the
`docs/` root because the release workflow tooling reads from one source of
truth and `docs/` is what `cargo xtask release` reviewers cite when a PR
asks "why this format?".

Closes #3310 — unifies pre-release tag format across Cargo, npm, PyPI, and
Homebrew so dist-tag automation and SemVer parsers (`node-semver`, `cargo`'s
`semver`, `packaging` for PEP 440) agree.

## Version format

| Channel | Pattern | Example |
|---|---|---|
| Stable | `vYYYY.M.D` | `v2026.5.4` |
| Beta | `vYYYY.M.D-beta.N` | `v2026.5.4-beta.7` |
| Release candidate | `vYYYY.M.D-rc.N` | `v2026.5.4-rc.1` |
| LTS patch | `vYYYY.M.PATCH-lts` | `v2026.3.0-lts` |

Rules:

- **No zero-padding** on month or day. `2026.5.4`, not `2026.05.04`.
- **Dot before the pre-release counter**: `-beta.7`, not `-beta7`. The dot
  is the separator the SemVer 2.0.0 spec calls a "dot-separated identifier"
  inside a pre-release suffix; without it, `node-semver` parses the entire
  `beta7` as a single alphanumeric identifier and downstream npm dist-tag
  selection becomes ambiguous.
- **Counter resets per channel, not per day.** `next_beta` is global within
  a calendar day from the generator's perspective (`xtask release`
  inspects `git tag -l 'v<base>-beta*'` and picks `max + 1`), so two
  beta cuts on the same day are `-beta.1` then `-beta.2`, but the counter
  does not roll back to 1 the next day.

## Why `-beta.N` (with dot)

- **SemVer 2.0.0 §9**: pre-release identifiers are dot-separated. `1.0.0-beta`
  is valid (single identifier `beta`), and so is `1.0.0-beta.1` (two
  identifiers, `beta` and `1`). `1.0.0-beta1` is *also* valid SemVer but
  treated as one identifier `beta1` — sort order is then lexicographic on
  the full string, so `beta10 < beta2`. The dot form sorts numerically on
  the second identifier (`beta.10 > beta.2`), which is what users expect.
- **`node-semver` / npm**: `npm publish --tag` automation chains scripts
  that compare against existing dist-tagged versions; it relies on
  `semver.prerelease()` returning the second component as a number for
  channel ordering. With `beta1`, that returns `["beta1"]` (string), and
  `--tag next` picks the wrong baseline.
- **PEP 440 (`packaging` / `pip`)**: `2026.5.4b7` is the canonical Python
  pre-release form; the PyPI publish step converts `-beta.N` → `bN`.
- **Cargo / `semver` crate**: parses both, but `cargo publish` against a
  prerelease only succeeds when the prerelease identifier is recognized as
  pre-release (i.e. dash-prefixed). Both forms work; we standardize on the
  dotted form for cross-ecosystem consistency.

## Migration / backward compatibility

Historical tags use the old `vYYYY.M.D-betaN` form (and even older
`vYYYY.M.DD-betaN` with zero-padded day). Those tags are **not rewritten**
— rewriting tags breaks every downstream artifact (Homebrew formula
versions, GitHub Release URLs, PyPI release filenames, the
`@librefang/cli` npm dist-tag history) and would invalidate every checksum
already on disk in the wild.

Going forward:

- The generator (`xtask src/release.rs`) only emits the canonical
  `-beta.N` / `-rc.N` form.
- The parser accepts both forms so re-syncing from a Cargo.toml that
  was last bumped under the old generator still computes
  `next_beta = max + 1` correctly. The first new bump after this PR
  reads `2026.5.2-beta8`, sees `8`, and emits `2026.5.X-beta.9`.
- The publish workflows accept both forms in their `grep -qE` /
  `sed` pipelines so old tags retro-pushed for a bug-fix re-publish
  still route to `--tag next` and produce a valid PEP 440 string.

## CI dist-tag behavior

Hardcoded in `.github/workflows/release.yml`:

| Tag matches | npm dist-tag | PyPI version | Homebrew channels |
|---|---|---|---|
| `-(beta\|rc)\.?N` | `next` | `bN` / `rcN` | `beta`, `rc` (or `rc` only for `-rc`) |
| `-lts` | `lts` | (skipped) | (skipped) |
| stable | (default → `latest`) | (canonical) | `stable`, `beta`, `rc` |

The regex is `'-(beta|rc)\.?[0-9]'` — the `\.?` is what made the legacy
`-betaN` continue to work after the format change.

## Generator + parser invariants (locked by tests)

`xtask/src/release.rs::tests`:

- `generator_emits_canonical_beta_with_dot` — output is `-beta.N`.
- `generator_emits_canonical_rc_with_dot` — output is `-rc.N`.
- `generator_does_not_zero_pad` — single-digit month/day stay unpadded.
- `parser_accepts_canonical_beta_dot_form` — new tags parse.
- `parser_accepts_legacy_beta_no_dot` — historical `-beta8` still parses.
- `parser_accepts_legacy_zero_padded_day` — `2026.03.21-beta1` parses.
- `migration_path_parse_then_regenerate_uses_dot_form` — read legacy,
  bump, regenerate → new format.
- `calver_re_accepts_both_forms` — channel-pick validator accepts both.

`xtask/src/sync_versions.rs::tests` mirrors the same coverage for
`validate_calver`, the PEP 440 conversion, and the Tauri patch regex.

## Operator quick reference

```bash
# Cut a beta off main (interactive)
cargo xtask release

# Non-interactive
cargo xtask release --channel beta --no-confirm
# → emits v2026.5.4-beta.N where N = (latest matching tag's N) + 1

# Stable release
cargo xtask release --channel stable

# LTS patch (must be on release/<series> branch)
cargo xtask release --lts-patch
```
