The dashboard lockfile moves `dompurify` — a transitive dependency of `mermaid` — from 3.4.15 to 3.4.16 through a scoped pnpm override, closing GHSA-p98j-92pf-mc4p.
The advisory is low severity, but the Security lane (`pnpm audit --prod`) fails on any run that resolves 3.4.13–3.4.15, and the patched 3.4.16 release exists.
The override lives in `crates/librefang-api/dashboard/pnpm-workspace.yaml` because pnpm 12 no longer reads settings from `package.json` — it warns and ignores the `pnpm` field there.
Scope is the dashboard package alone; no other lockfile in the workspace moves. (#8424) (@DaBlitzStein)
