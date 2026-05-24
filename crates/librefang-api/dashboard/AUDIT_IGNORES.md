# Dashboard pnpm audit ignores

This file enumerates each entry under `pnpm.auditConfig.ignoreGhsas` in
[`package.json`](./package.json), with the rationale and unlock
condition for each ignore. Add a new section here whenever you add a
GHSA to that list; remove the section when the ignore is dropped.

The list is parsed by `pnpm audit` at every dashboard CI run
(`xtask deps --audit --web`); ignores are not a free pass — they are a
deliberate trade-off documented in writing so a future maintainer (or
auditor) can re-evaluate without spelunking the git history. Refs #5667.

## Currently ignored

### `GHSA-rmmr-r34h-pfm5` — `@tanstack/history` (critical, "malware")

- **Why ignored.** The advisory was filed to cover the supply-chain
  hijack of `@tanstack/history` (the malicious `1.161.9` /
  `1.161.12` publishes), but its affected-range expression is
  `>= 0`, so it also flags the legitimate `1.161.6` we resolve to.
  Our lockfile (`pnpm-lock.yaml`) is pinned to the clean `1.161.6`
  via `@tanstack/react-router`, and the dashboard never imports
  `@tanstack/history` directly. Re-pinning to a hotfix tag isn't
  available because `react-router` controls the transitive
  resolution.
- **Risk if wrong.** None at the pinned version: the malicious
  versions are never installed (verified by `grep -A1
  '@tanstack/history@' pnpm-lock.yaml`).
- **Unlock condition.** Drop this ignore when *any* of the following
  is true:
  1. The advisory's affected range is narrowed upstream to exclude
     `1.161.6` (re-check via `pnpm audit --json | jq
     '.advisories["GHSA-rmmr-r34h-pfm5"]'`).
  2. `@tanstack/react-router` ships a release that resolves
     `@tanstack/history` to a version the advisory does not flag,
     and we bump to it.
  3. The dashboard stops depending on `@tanstack/react-router`
     entirely (the only path that pulls `@tanstack/history` in).
- **Owner / last review.** Originally landed in #4944
  (`fix(ci): unblock Security audit job`). Re-audit at the next
  dependency-hygiene roundup (refs #5667) or whenever
  `@tanstack/react-router` is bumped, whichever comes first.

## How to add a new ignore

1. Verify the advisory genuinely does not apply to the resolved
   version (grep `pnpm-lock.yaml`, read the advisory's affected
   range, confirm we don't call the affected API path).
2. Add the GHSA id to `pnpm.auditConfig.ignoreGhsas` in
   `package.json`.
3. Add a section here with **Why**, **Risk**, **Unlock condition**,
   and **Owner / last review**.
4. Reference both files in the PR body so reviewers see the
   trade-off explicitly.
