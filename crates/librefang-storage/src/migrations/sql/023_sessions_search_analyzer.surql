-- Migration v23: Define unicode-normalizing search analyzer for sessions (#3548).
--
-- Mirrors SQLite migration v33, which rebuilt the sessions_fts virtual table
-- with an explicit 'unicode61' tokenizer so tokenization is documented, stable,
-- and matches query-time normalization.
--
-- SurrealDB does not use FTS5 virtual tables; instead it provides native
-- SEARCH indexes backed by configurable ANALYZER definitions. This migration
-- declares the canonical analyzer used by session label/content search so
-- that future SEARCH index definitions have a consistent, unicode-normalizing
-- tokenization strategy rather than relying on implicit defaults.
--
-- The analyzer:
--   - TOKENIZERS blank,class  → splits on whitespace and Unicode character-
--                               class boundaries (equivalent to unicode61)
--   - FILTERS lowercase,ascii → case-folds and ASCII-normalizes before
--                               indexing, matching query-side normalization
--
-- DEFINE FIELD ensures messages_generation and last_repaired_generation —
-- added to the Session struct in upstream v2026.5.4 — are formally declared
-- on the SCHEMAFULL sessions table so they are preserved on round-trips even
-- though the current surreal_session.rs backend initialises them from defaults
-- rather than persisting them explicitly.

DEFINE ANALYZER IF NOT EXISTS sessions_unicode
    TOKENIZERS blank,class
    FILTERS lowercase,ascii;

DEFINE FIELD IF NOT EXISTS messages_generation    ON sessions TYPE int VALUE $value OR 0;
DEFINE FIELD IF NOT EXISTS last_repaired_generation ON sessions TYPE option<int>;
