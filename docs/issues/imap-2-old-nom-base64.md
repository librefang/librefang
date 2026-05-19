# `imap 2.x` drags in old `nom 5.1.3` and `base64 0.13.1`

**Severity:** Medium
**Category:** Dependencies and supply chain
**Labels:** `security`, `unmaintained`, `medium`

## Affected files
- `Cargo.toml:268` (`imap = { version = "2", default-features = false }`)
- Transitively: `nom 5.1.3` (`Cargo.lock:3718`), `base64 0.13.1`, `imap-proto 0.10.2`

## Description

- `imap 2.x` has been unmaintained since 2021;
- `nom 5` is upstream-deprecated;
- `base64 0.13.1` is also unmaintained;
- These sit on the **IMAP parsing path** in the channels crate — a network-facing parser is the very last place to be using EOL libraries.

## Recommendation

Migrate to a modern implementation:

- `async-imap` (active, on `nom 7`);
- `imap-codec`.

During migration, add a temporary ignore in `deny.toml` plus a tracking-issue link.
