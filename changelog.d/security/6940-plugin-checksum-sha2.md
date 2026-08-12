Replace the hand-rolled SHA-256 implementation in the plugin integrity path with the workspace's existing vetted `sha2` crate.
The hand-rolled version was never audited and carried a stale comment suggesting a future swap to `sha2` that never happened, leaving plugin checksum verification resting on unreviewed cryptographic code.
The public `sha256_hex` API and its lowercase 64-character hex digest format are unchanged, so no caller or stored checksum is affected (#6940) (@houko)
