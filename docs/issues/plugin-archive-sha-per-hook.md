# Plugin install: integrity SHA-256 is per-hook, not per-archive

**Severity:** Medium
**Category:** Command injection, SSRF, sandbox
**Labels:** `security`, `supply-chain`, `plugin`, `medium`

## Affected files
- `crates/librefang-runtime/src/plugin_manager/install.rs:191-211`, `:308`
- `crates/librefang-runtime/src/plugin_manager.rs:39-52` (`manifest_missing_integrity_hooks`)
- Registry signature path: `registry.rs:297-318`

## Description

The integrity table is keyed by `hooks/<script>`, and the comment at `install.rs:211-213` admits: "the official registry does not publish per-plugin Ed25519 archive signatures."

Consequences:

- **Non-hook** files inside a plugin directory (helper modules imported by hooks, `requirements.txt`, datasets) are outside the SHA-256 coverage;
- A compromised registry can replace a helper `.py` that `ingest.py` imports, with no validation;
- The signed `index.json` is a **metadata** trust root, but does not cover the archive bytes between fetch and decompression.

## Recommendation

Add a per-archive SHA-256 field to the signed `index.json`; `install_from_registry` hashes the archive before extraction. The registry signature path (`registry.rs:297-318`) already exists; the marginal cost is minimal.
