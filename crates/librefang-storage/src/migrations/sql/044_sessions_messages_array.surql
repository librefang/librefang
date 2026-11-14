-- Migration v44: type `messages` on `sessions` and `canonical_sessions` as a list of message objects.
-- SQLite stores `messages` as an opaque msgpack BLOB, so there is no SQLite equivalent of this step.
--
-- `005_sessions.surql` declared both columns `option<object> FLEXIBLE`, but a session history is a JSON array of `Message` objects.
-- `SurrealSessionBackend::save_session` / `append_canonical` and `migrate::sqlite_to_surreal` all write that array, and SurrealDB 3.x rejects it with `Expected none | object but found [...]`.
-- 005 has been applied on existing databases, so it cannot be edited (the runner would report checksum drift); this migration redefines the fields instead.
--
-- `array<object>` keeps each element an object, and FLEXIBLE keeps every nested key (content blocks, tool inputs, provider metadata) that SCHEMAFULL would otherwise strip.
-- No data conversion is needed: every array write was rejected under 005, so no row can hold a message list today, and DEFINE FIELD does not re-validate stored rows.

DEFINE FIELD OVERWRITE messages ON sessions           TYPE option<array<object>> FLEXIBLE;
DEFINE FIELD OVERWRITE messages ON canonical_sessions TYPE option<array<object>> FLEXIBLE;
