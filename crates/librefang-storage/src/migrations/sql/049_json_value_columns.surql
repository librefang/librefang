-- Migration v49: let the free-form JSON columns hold the JSON the runtime actually writes.
-- SQLite stores these as opaque BLOB / TEXT, so there is no SQLite equivalent of this step.
--
-- `006_kv_store.surql` typed `kv_store.value` as `option<object> FLEXIBLE`, but `StructuredStore::set` / `SurrealKvBackend::structured_set` store any JSON value: strings, numbers, booleans and arrays as well as objects.
-- `010_prompt_management.surql` typed `prompt_versions.tools` and `variables` as `option<object> FLEXIBLE`, but both are `Vec<String>` (`PromptVersion::tools` / `variables`), stored as JSON arrays of strings (`SurrealPromptStore::create_version` writes `[]`, SQLite stores `["..."]`).
-- SurrealDB 3.2 and 3.3 both reject those writes (`Expected none | object but found ...`), so no stored row can hold a non-object value and no data conversion is needed; DEFINE FIELD does not re-validate stored rows.
-- 006 and 010 are applied on existing databases and cannot be edited (checksum drift), so this migration redefines the fields.
--
-- `any` accepts every JSON value, including nested objects of any shape, in a SCHEMAFULL table.

DEFINE FIELD OVERWRITE value     ON kv_store        TYPE any;
DEFINE FIELD OVERWRITE tools     ON prompt_versions TYPE option<array<string>>;
DEFINE FIELD OVERWRITE variables ON prompt_versions TYPE option<array<string>>;
