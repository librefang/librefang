//! Custom serialization/deserialization helper functions.

// `OneOrMany<T>` and `deserialize_string_or_int_vec` removed
// — both were `serde(default)` shapes used by the in-process
// `[channels.<vendor>]` config blocks (single-table vs.
// array-of-tables; CSV-or-int Vec coercion). With every channel
// migrated to a sidecar `ChannelsConfig` no longer carries any
// `OneOrMany<T>` field and the helpers had zero production
// callers. Restore from git history (last on
// `feat/channels-google-chat-sidecar` ≈ 22eb9297) if a future
// in-process channel needs the multi-instance shape back.
