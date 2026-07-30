# Changelog fragments

Write your changelog entry as a **new file in this directory** instead of editing `CHANGELOG.md`.

Every PR that appends a bullet to the single `## [Unreleased]` section of `CHANGELOG.md` conflicts with every other PR that does the same, in a file where the conflict is pure noise — both sides are correct and the resolution is always "keep both".
Two PRs never touch the same fragment file, so the conflict becomes structurally impossible rather than merely rarer.

Editing `## [Unreleased]` directly still works and is still supported.
A fragment is the same bullet with a delay on it: `cargo xtask collect-fragments` folds fragments into `## [Unreleased]` and deletes the files it consumed, and the release flow runs that step before cutting the dated release section.

## Where your fragment ends up

In the GitHub release body, verbatim.

`cargo xtask release` folds the fragments in, then moves the whole `## [Unreleased]` body into the dated `## [VERSION]` section it cuts — subsections and order intact — leaving the `## [Unreleased]` heading behind and empty for the next cycle.
`.github/workflows/release.yml` slices that section out of `CHANGELOG.md` and publishes it as the release notes; `release-notify.yml` reuses the same slice for the announcement article and the social post.

The rest of the section is generated from PR metadata and fills only the gaps.
Every merged PR in the range gets a `- <PR title> (#N) (@author)` line **unless** its number appears in the trailing `(#N)` group of a curated bullet, in which case that bullet is the entry and no generated line is added.
So write prose that explains *why*, not a restatement of your PR title — the title is already covered for free.

## Sections

One directory per `### ` heading of `## [Unreleased]`:

| Directory | Heading |
| --- | --- |
| `added/` | `### Added` |
| `fixed/` | `### Fixed` |
| `changed/` | `### Changed` |
| `security/` | `### Security` |
| `documentation/` | `### Documentation` |

A fragment in any other directory is **rejected** by `scripts/check-changelog-attribution.py`, because assembly has no heading to render it under and would drop it silently.

## Format

One file, one bullet.
The file holds the bullet body **without** the leading `- `, wrapped one sentence per line with continuation lines indented two spaces, ending with `(#PR) (@your-github-login)`.

The file name is yours to choose, but lead it with the PR or issue number so fragments sort in a useful order — bullets are assembled in file-name order within each section.

## Worked example

`changelog.d/fixed/6623-wire-max-content-chars.md`:

```markdown
Honour `max_content_chars` on the streaming path, which read the compiled-in default and ignored the per-agent override entirely.
The value was resolved once at kernel boot and captured into the driver, so an `agent.toml` edit took effect only after a restart.
It is now resolved per turn from the manifest, falling back to the kernel config and then the compiled default (#6623) (@houko)
```

After `cargo xtask collect-fragments` that lands under `### Fixed` in `## [Unreleased]` as:

```markdown
- Honour `max_content_chars` on the streaming path, which read the compiled-in default and ignored the per-agent override entirely.
  The value was resolved once at kernel boot and captured into the driver, so an `agent.toml` edit took effect only after a restart.
  It is now resolved per turn from the manifest, falling back to the kernel config and then the compiled default (#6623) (@houko)
```

## Rules the tooling enforces

- The bullet must carry a `(@your-github-login)` attribution, exactly as an `[Unreleased]` bullet must (issue #3400).
  The attribution may sit on any line of the bullet, but not past a blank line — a blank line ends the bullet, so anything after it is a separate paragraph.
- The fragment must sit in one of the five section directories listed above.
- Prose is not hard-wrapped at any column; break only at sentence boundaries.

Not enforced, but it costs you if you skip it: end the bullet with its PR reference, `(#1234)` — or `(#1234, #1235)` when one entry covers two PRs.
That group is how the release flow knows which generated line your prose replaces.
Without it your PR keeps its generated line, so it appears twice in the release body, and `cargo xtask release` prints a warning naming the bullet.
That is the only cost, and it is yours alone: an unreferenced bullet does not stop anyone else's bullet from replacing its own generated line.
Only the **last** `(#N)` group on the bullet's **last non-empty line** counts, so a mid-bullet cross-reference to some other PR is never mistaken for yours.

Checked by the `pre-commit` hook and by the `CHANGELOG Attribution` CI jobs, in all of their modes:

```bash
python3 scripts/check-changelog-attribution.py --staged          # what this commit stages
python3 scripts/check-changelog-attribution.py --all-unreleased  # everything pending
```

The `.gitkeep` files keep the empty section directories tracked; leave them alone.
