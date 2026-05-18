# `serde_yaml 0.9.34+deprecated` is archived (RUSTSEC-2024-0320)

**Severity:** Medium
**Category:** Dependencies and supply chain
**Labels:** `security`, `unmaintained`, `medium`

## Affected files
- `Cargo.toml:197` (`serde_yaml = "0.9"`)

## Description

`Cargo.lock` resolves to `0.9.34+deprecated`, which pulls in `unsafe-libyaml 0.2.11`.

- The upstream author **archived** the crate;
- `unsafe-libyaml` is heavily `unsafe` by design;
- Future CVEs will never be fixed;
- There is **no** corresponding ignore in `deny.toml` — the moment the advisory DB grows an entry, cargo-deny CI fails.

## Recommendation

Migrate to an actively maintained fork:

- `serde_yml` (actively maintained);
- `serde-yaml-bw`.

Start with `rg "serde_yaml" --type rust` to scope call sites — typically a 1–2 day effort.
