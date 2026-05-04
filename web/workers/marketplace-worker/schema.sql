-- FangHub Marketplace D1 Schema

CREATE TABLE IF NOT EXISTS users (
  id          TEXT PRIMARY KEY,          -- github:{github_id}
  github_id   INTEGER NOT NULL UNIQUE,
  handle      TEXT NOT NULL,
  display_name TEXT NOT NULL,
  avatar_url  TEXT,
  created_at  INTEGER NOT NULL           -- unix timestamp
);

CREATE TABLE IF NOT EXISTS packages (
  id              TEXT PRIMARY KEY,      -- slug
  name            TEXT NOT NULL,
  kind            TEXT NOT NULL CHECK (kind IN ('skill', 'hand', 'extension', 'mcp')),
  description     TEXT NOT NULL DEFAULT '',
  author_id       TEXT NOT NULL REFERENCES users(id),
  github_repo     TEXT,                  -- owner/repo
  homepage        TEXT,
  tags            TEXT NOT NULL DEFAULT '[]',  -- JSON array
  total_downloads INTEGER NOT NULL DEFAULT 0,
  weekly_downloads INTEGER NOT NULL DEFAULT 0,
  stars           INTEGER NOT NULL DEFAULT 0,
  is_verified     INTEGER NOT NULL DEFAULT 0,
  is_featured     INTEGER NOT NULL DEFAULT 0,
  latest_version  TEXT,
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS package_versions (
  id          TEXT PRIMARY KEY,          -- {slug}@{version}
  package_id  TEXT NOT NULL REFERENCES packages(id),
  version     TEXT NOT NULL,
  changelog   TEXT NOT NULL DEFAULT '',
  bundle_url  TEXT NOT NULL,
  bundle_sha256 TEXT NOT NULL,
  bundle_sig  TEXT,                      -- Ed25519 signature over `id|bundle_url|bundle_sha256`, base64; NULL when REGISTRY_PRIVATE_KEY unset
  downloads   INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL,
  UNIQUE(package_id, version)
);

-- Migration for existing databases (D1 ignores duplicate ALTER):
-- ALTER TABLE package_versions ADD COLUMN bundle_sig TEXT;

CREATE TABLE IF NOT EXISTS stars (
  user_id    TEXT NOT NULL REFERENCES users(id),
  package_id TEXT NOT NULL REFERENCES packages(id),
  created_at INTEGER NOT NULL,
  PRIMARY KEY (user_id, package_id)
);

-- Batched download events flushed by cron
CREATE TABLE IF NOT EXISTS download_counts_pending (
  package_id TEXT NOT NULL,
  version_id TEXT NOT NULL,
  count      INTEGER NOT NULL DEFAULT 0,
  week       TEXT NOT NULL,             -- YYYY-WW for weekly rollup
  PRIMARY KEY (package_id, version_id, week)
);

-- Registry item click counts
CREATE TABLE IF NOT EXISTS registry_clicks (
  category   TEXT NOT NULL,
  item_id    TEXT NOT NULL,
  count      INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (category, item_id)
);

-- GitHub repo daily stats history (replaces KV stats_history blob)
CREATE TABLE IF NOT EXISTS github_stats_history (
  date    TEXT NOT NULL PRIMARY KEY,  -- YYYY-MM-DD
  stars   INTEGER NOT NULL DEFAULT 0,
  forks   INTEGER NOT NULL DEFAULT 0,
  issues  INTEGER NOT NULL DEFAULT 0,
  prs     INTEGER NOT NULL DEFAULT 0
);

-- Key-value store for singleton config/state (replaces remaining KV keys)
-- Used for: registry_data, registry_data_time, stats_migration_done
CREATE TABLE IF NOT EXISTS kv_store (
  key        TEXT NOT NULL PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);

-- UI error reports (replaces KV ui_errors:* shards)
CREATE TABLE IF NOT EXISTS ui_errors (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  at         TEXT NOT NULL,
  message    TEXT NOT NULL,
  stack      TEXT,
  pathname   TEXT,
  lang       TEXT,
  ua         TEXT
);

-- Page visit counts (replaces KV visit counter)
-- Special rows: '__total__' for all-time total, '__migrated__' as migration flag
CREATE TABLE IF NOT EXISTS visit_counts (
  date   TEXT NOT NULL PRIMARY KEY,  -- YYYY-MM-DD, '__total__', or '__migrated__'
  count  INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_packages_kind ON packages(kind);
CREATE INDEX IF NOT EXISTS idx_packages_downloads ON packages(total_downloads DESC);
CREATE INDEX IF NOT EXISTS idx_packages_updated ON packages(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_versions_package ON package_versions(package_id);
CREATE INDEX IF NOT EXISTS idx_clicks_category ON registry_clicks(category, count DESC);
CREATE INDEX IF NOT EXISTS idx_stats_history_date ON github_stats_history(date DESC);
CREATE INDEX IF NOT EXISTS idx_ui_errors_at ON ui_errors(at DESC);
