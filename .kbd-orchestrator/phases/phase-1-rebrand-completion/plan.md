# Phase 1 — Rebrand completion

## Goal

Complete the user-facing BossFang rebrand while preserving cheap upstream
merges. Source plan: `docs/architecture/bossfang-rebrand-completion.md`
(landing in [GQAdonis/librefang#9](https://github.com/GQAdonis/librefang/pull/9)).

## Source artifact

The roadmap doc enumerates 7 milestones (M1-M7) bucketed by priority
and merge-cost forecast. This phase tracks execution of all 7.

## Change inventory (5 changes mapped from 7 milestones)

| Change ID | Milestones | Priority | Status |
|---|---|---|---|
| `m1-m2-cli-banner-install-paths-config-templates` | M1 + M2 | HIGH | IN_PROGRESS |
| `m3-install-script` | M3 | HIGH | PENDING |
| `m4-docs-mdx-sweep` | M4 | HIGH | PENDING |
| `m5-m7-ua-strings-env-aliases` | M5 + M7 | MED/LOW | PENDING |
| `m6-dashboard-localstorage-shim` | M6 | MED | PENDING |

## Sequencing rationale

M1+M2 are bundled because both are pure Surface flips in `librefang-cli`
(plus one Boundary aliased install path migration in M1). M5+M7 share
`plugin_manager.rs` neighbourhood. M3, M4, M6 each get standalone PRs
because of scope (install script security review, doc churn dominates
review, dashboard state-touching needs explicit shim review).

## Phase goals (✅ when ALL changes DONE)

- [ ] M1 — CLI banner + 15 desktop install path strings (with `LibreFang.app` → `BossFang.app` migration shim)
- [ ] M2 — Init wizard / config template comment flips
- [ ] M3 — install.sh user-facing echoes + `BOSSFANG_INSTALL_DIR` alias
- [ ] M4 — 120 docs/src/app/**/*.mdx prose flips via extended `enforce-branding.py`
- [ ] M5 — Outbound UA strings: `User-Agent: LibreFang/1.0` → `BossFang/1.0`, `LibreFangAgent/0.1` → `BossFangAgent/0.1`
- [ ] M6 — Dashboard `localStorage.getItem("librefang-api-key")` → `bossfang-api-key` with shim
- [ ] M7 — 4 `BOSSFANG_*` env-var aliases (additive): `BOSSFANG_INSTALL_DIR`, `BOSSFANG_REGISTRY_PUBKEY`, `BOSSFANG_REGISTRY_PUBKEY_URL`, `BOSSFANG_DASHBOARD_EMBEDDED_ONLY`

## Forecasted per-merge cost increment after phase completion

~10 minutes per upstream merge (dominated by M4 MDX prose re-run, bounded by
`enforce-branding.py` automation). Without M4 automation, M4 alone would
add 30+ minutes.

## Out-of-scope (explicitly)

See `.kbd-orchestrator/constraints.md` for the full default-migration /
domain-ownership / Layer Internal preservation lists.

Notably **not** in this phase:

- Renaming default `~/.librefang/` → `~/.bossfang/`
- Hosting a BossFang plugin trust root (deferred until a BossFang registry exists)
- Aliasing the ~60 deep plumbing `LIBREFANG_*` env vars (devs-only)
- Touching any of the 532+ files containing Layer Internal symbols
