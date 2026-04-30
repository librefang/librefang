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
  downloads   INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL,
  UNIQUE(package_id, version)
);

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

-- Registry item click counts (replaces KV shard pattern)
CREATE TABLE IF NOT EXISTS registry_clicks (
  category   TEXT NOT NULL,
  item_id    TEXT NOT NULL,
  count      INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (category, item_id)
);

CREATE INDEX IF NOT EXISTS idx_packages_kind ON packages(kind);
CREATE INDEX IF NOT EXISTS idx_packages_downloads ON packages(total_downloads DESC);
CREATE INDEX IF NOT EXISTS idx_packages_updated ON packages(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_versions_package ON package_versions(package_id);
CREATE INDEX IF NOT EXISTS idx_clicks_category ON registry_clicks(category, count DESC);
