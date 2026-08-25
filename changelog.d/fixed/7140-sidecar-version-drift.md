A sidecar adapter running an SDK older than the daemon is no longer invisible.
The protocol version rode on every `ready` frame, was documented as `1`, and was pinned at `1` in the shared conformance corpus — but no adapter ever set it, so every real frame carried `null`, and the daemon logged whatever arrived without comparing it to anything.
Both SDKs now declare the version by default, the daemon compares it against `SIDECAR_PROTOCOL_VERSION` and warns on skew or absence, `--describe` reports the adapter's `librefang-sdk` version through `GET /api/channels` and the configure drawer, and a stale pip install that shadows the daemon's bundled SDK says so at `WARN` instead of at `debug`.
A drift guard pins the protocol version across the daemon constant, both SDKs, the corpus fixture, and the architecture doc, and the corpus finally covers the `Command` content frame that a slash command travels in — the one frozen-core shape that had no fixture on either side.
(#7848) (@houko)
