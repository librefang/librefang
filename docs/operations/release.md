# Release pipeline

LibreFang's release pipeline lives in `.github/workflows/`. As of #3304
1/N it is in a transitional state: the monolithic `release.yml`
remains the canonical entrypoint, and a set of `workflow_dispatch`-only
"split" workflows have been added alongside it as scaffolding for a
later cutover.

## Current state (post #3304 1/N)

- **Canonical entrypoint:** `release.yml` (~2,500 lines, ~30 jobs).
  Triggered by `push: tags: v*`, the `release-bump` `workflow_dispatch`
  flow, and the `pull_request: closed` auto-tag flow. Everything that
  ships to npm, PyPI, crates.io, GHCR, the Homebrew tap, Fly, Render,
  Play Internal Testing, and TestFlight goes through this file.
- **Manual single-target reruns:** five new files added in this PR.
  They are `workflow_dispatch`-only — they never fire automatically
  on tag push, so they cannot accidentally double-publish.

| File | Purpose | Inputs | Mirrors monolithic jobs |
| ---- | ------- | ------ | ----------------------- |
| `release-tag.yml`          | Manual tag verify + push                    | `version`                                                         | `tag_on_merge` (PAT-pushed tag), `create_release` precondition |
| `release-cli.yml`          | CLI binaries, signed manifest, optional PyPI | `tag`, `include_pypi`                                             | `build_dashboard`, `cli_*`, `sign_release_artifacts`, `cli_pypi` |
| `release-sdks.yml`         | JS / Python / Rust / Go SDK publishes       | `tag`, `include_javascript`, `include_cli_npm`, `include_python`, `include_rust`, `include_go` | `sdk_javascript`, `sdk_cli_npm`, `sdk_python`, `sdk_rust`, `sdk_go` |
| `release-desktop.yml`      | Tauri desktop bundle (5 platforms) + cask sync | `tag`, `sync_cask`                                                | `desktop`, `sync_homebrew_cask` |
| `release-npm-binaries.yml` | CLI npm binary packages (per-arch)          | `tag`                                                             | `cli_npm` |

When you trigger any split workflow, it operates on an already-existing
GitHub Release tag. It checks out the source tree at that tag,
rebuilds the artifact, and re-uploads with `gh release upload --clobber`,
which is the same idempotent path the monolithic jobs use on rerun.

## When to use each path

- **Normal release:** push a tag (or merge a `chore/bump-version-…` PR).
  `release.yml` runs end-to-end. **Do nothing else.**
- **One target failed and you want to redo it without rerunning the
  whole 30-job pipeline:** trigger the matching split workflow from the
  Actions UI. Provide the existing tag as `tag` (and any other input
  the workflow declares).
- **You messed up a tag:** delete the tag from origin via Releases UI,
  then trigger `release-tag` to re-create it on the corrected commit
  (or just push again with `git push origin <tag>`).

## Authentication deltas in the split files

The split workflows are wired for **OIDC-based publishing** wherever
possible, instead of the long-lived PATs the monolithic file uses:

- **npm** (`release-npm-binaries`, `release-sdks` JavaScript jobs):
  uses `permissions: id-token: write` + `npm publish --provenance`.
  The maintainer must configure the npm trusted-publisher relationship
  for each package on https://www.npmjs.com/package/<pkg>/access *before*
  any cutover that flips traffic from `release.yml` (NPM_TOKEN) to a
  split workflow. Until then, only the monolithic NPM_TOKEN path
  publishes real releases.
- **PyPI** (`release-cli` CLI wheels, `release-sdks` Python SDK):
  already OIDC in both monolithic and split.
- **crates.io** (`release-sdks` Rust SDK): still uses
  `CARGO_REGISTRY_TOKEN`. crates.io has no OIDC trusted-publisher path
  yet; this stays a PAT until upstream supports it.

`release.yml` itself is not modified by this PR — its existing NPM_TOKEN
path stays in place and is the actual publish path until cutover.

## GitHub environments referenced

Each split job sets `environment: <name>`. None of these environments
exist yet — GitHub creates them on first reference but with no
protection rules, which means anyone with `actions: write` can run the
workflow. The maintainer must configure each via
https://github.com/librefang/librefang/settings/environments before
relying on these workflows for real releases. Recommended settings:

| Environment       | Required reviewers          | Wait timer | Notes |
| ----------------- | --------------------------- | ---------- | ----- |
| `release-tag`     | 1 release maintainer        | 0 min      | Tag pushes are recoverable; reviewer is the gate |
| `release-cli`     | 1 release maintainer        | 0 min      | Re-uploading existing CLI tarballs is low-risk |
| `release-desktop` | 1 release maintainer        | 0 min      | Same reasoning; cask sync writes to the tap repo |
| `release-npm`     | 2 release maintainers       | 5 min      | Publish is irrevocable on npm |
| `release-pypi`    | 2 release maintainers       | 5 min      | Yank exists on PyPI but is not "delete" |
| `release-crates`  | 2 release maintainers       | 5 min      | crates.io publish is irrevocable beyond yank |

## Maintainer follow-up checklist (after this PR merges)

- [ ] Configure each environment listed above with required reviewers
      and wait timer via the repo Settings → Environments page.
- [ ] Configure npm trusted-publisher for `@librefang/sdk`,
      `@librefang/cli`, and the per-arch binary packages emitted by
      `cargo xtask publish-npm-binaries`. Workflow filename is the
      split file; environment name is `release-npm`.
- [ ] Confirm `cargo xtask publish-npm-binaries` propagates
      `--provenance` to its underlying `npm publish` calls. If it does
      not, add the flag in `xtask/src/publish_npm_binaries.rs` (or
      whichever module owns it) before any cutover PR — otherwise the
      OIDC-equipped split workflow still publishes without provenance
      attestations.
- [ ] Smoke-test each split workflow on a real tag that already
      shipped. The split jobs use `gh release upload --clobber`, so
      re-uploading identical artifacts is a no-op as far as
      consumers are concerned.

## Migration plan

1. **Phase 1 (this PR):** scaffold the split workflows alongside
   `release.yml`. No traffic moves. Configure environments + npm
   trusted publishers. **Done when** maintainers have manually run
   each split workflow at least once on a recent tag and confirmed it
   re-uploads the right artifacts.
2. **Phase 2 (follow-up PR):** convert each split file to also accept
   `on: workflow_call:` so `release.yml` can call them as reusable
   workflows. Replace the inlined `cli_*` / `desktop` / `sdk_*` /
   `cli_npm` / `cli_pypi` blocks in `release.yml` with `uses:
   ./.github/workflows/release-<area>.yml` references. Mobile splits
   (`mobile_android`, `mobile_ios`) join in this phase too; they were
   intentionally left in `release.yml` for Phase 1.
3. **Phase 3 (follow-up PR):** flip npm SDK/CLI publishes from
   NPM_TOKEN to the OIDC path by removing `NODE_AUTH_TOKEN` from the
   reusable workflow callers. Requires Phase 2 + npm trusted-publisher
   configuration to be live.

Refs #3304.
