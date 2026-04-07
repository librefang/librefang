# MIGRATION-GUIDE: Single-Tenant → Multi-Tenant

**Status:** Draft
**Date:** 2026-04-06
**Author:** Engineering
**Related:** MASTER-PLAN.md, ADR-MT-001 (Account Model), SPEC-MT-003 (Database Migration), PLAN-MT-001 (Implementation Plan)
**Audience:** LibreFang operators upgrading existing single-tenant installations

---

## Overview

This guide covers upgrading an existing single-tenant LibreFang installation to
multi-tenant mode. The migration is **non-destructive, backward-compatible, and
requires zero downtime**.

All existing data is preserved — it migrates to the implicit `system` account.
Existing CLI, desktop, and API clients continue working without changes.

---

## Prerequisites

| Requirement | Minimum | Notes |
|-------------|---------|-------|
| LibreFang version | ≥ TBD (with multi-tenant support) | Check: `librefang --version` |
| SQLite version | ≥ 3.35.0 | Required for `ALTER TABLE DROP COLUMN` in rollback |
| Disk space | ~50 MB free | For new indexes on existing tables |
| Downtime | None required | Migration is online (ALTER + CREATE INDEX) |

---

## Step-by-Step Migration

### Step 1: Update LibreFang

```bash
# Pull latest release with multi-tenant support
librefang update
# — or from source —
git pull && cargo build --release
```

### Step 2: Enable Multi-Tenant Mode

Add the `[multi_tenant]` section to your config:

```toml
# ~/.librefang/config.toml

[multi_tenant]
enabled = true
hmac_secret = ""  # Generate below
default_account = "system"
accounts_dir = "accounts"
```

Generate the HMAC secret:

```bash
# Generate a 256-bit secret
openssl rand -hex 32
# Example output: a1b2c3d4e5f6...  (64 hex chars)
```

Paste the generated value into `hmac_secret`:

```toml
[multi_tenant]
enabled = true
hmac_secret = "a1b2c3d4e5f6...your-64-char-hex-secret-here"
```

> **Security:** This secret must match your upstream SaaS layer (e.g., Qwntik's
> `OPENFANG_ACCOUNT_HMAC_SECRET` env var). Both sides use it to sign and verify
> `X-Account-Id` headers.

### Step 3: Restart LibreFang

```bash
librefang restart
```

### Step 4: Verify Migration

```bash
# Check that multi-tenant is active
curl -s http://localhost:3000/api/health | jq '.multi_tenant'
# Expected: { "enabled": true, "default_account": "system" }

# Verify existing agents are accessible (now under 'system' account)
curl -s http://localhost:3000/api/agents | jq '.[] | .name'
# Expected: same agents as before

# Verify database migration applied
sqlite3 ~/.librefang/data/librefang.db \
  "SELECT sql FROM sqlite_master WHERE name='memories'" | grep account_id
# Expected: account_id TEXT NOT NULL DEFAULT 'system'
```

---

## What Happens Automatically on Restart

| Step | Action | Reversible |
|------|--------|------------|
| 1 | Database migration runs — adds `account_id TEXT NOT NULL DEFAULT 'system'` to `memories`, `sessions`, `kv_store`, `proactive_memories` | Yes (see Rollback) |
| 2 | Indexes created: `idx_memories_account_id`, `idx_sessions_account_id`, etc. | Yes |
| 3 | `accounts/system/` directory created | Yes |
| 4 | Existing agent manifests moved to `accounts/system/agents/` | Yes |
| 5 | Migration version recorded in `migrations` table | Yes |

All operations are idempotent — safe to run multiple times.

---

## What Doesn't Change

| Component | Behavior | Why |
|-----------|----------|-----|
| **CLI commands** | Work identically | CLI always uses `system` account |
| **Desktop app** | Works identically | No `X-Account-Id` header = `system` |
| **Existing API clients** | Work identically | Bearer-only auth → `system` account |
| **Channel bridges** | Default to `system` | Channels without `account_id` binding default |
| **Skill installations** | Remain global | Skills are shared; accounts get allowlists |
| **Config format** | Additive only | New `[multi_tenant]` section; all existing fields unchanged |

---

## Creating New Accounts

Accounts are provisioned by the upstream SaaS layer (e.g., Qwntik). LibreFang
does not manage account lifecycle. When a request arrives with a new `X-Account-Id`:

1. LibreFang validates the HMAC signature
2. Creates `accounts/{account_id}/` directory lazily
3. Loads default `AccountConfig` (or account-specific `config.toml` if present)
4. All resources created in that request are tagged with the account_id

No pre-provisioning required.

### Account-Specific Config (Optional)

```toml
# ~/.librefang/accounts/acc_abc123/config.toml

default_model = "claude-3-sonnet"
skill_allowlist = ["web_search", "code_exec", "file_manager"]
rate_limit_per_minute = 60
system_prompt_prefix = "You are a helpful assistant for Acme Corp."
```

---

## For Qwntik Integration

Qwntik's `@kit/openfang` package sends `X-Account-Id` + `X-Account-Sig`
headers on all requests via `getAccountOptions()`. Once multi-tenant is enabled
on LibreFang, Qwntik's existing account isolation flows through end-to-end.

### Required Qwntik Environment

```env
# .env.local (Qwntik app)
OPENFANG_ACCOUNT_HMAC_SECRET=same-secret-as-librefang-config
```

### Request Flow

```
Qwntik Server Action
  → getAccountOptions(accountId)
  → Compute HMAC: sha256(secret, accountId)
  → HTTP Request to LibreFang:
      X-Account-Id: acc_abc123
      X-Account-Sig: hmac-sha256-hex
  → LibreFang middleware validates HMAC
  → Routes to account-scoped handlers
  → Response (no account_id leaked in body)
```

---

## Supabase + RuVector Setup (Optional)

For production multi-tenant deployments with vector search:

### Step 1: Start Supabase with RuVector Extension

```bash
cd docker/
docker compose up -d db
# Starts PostgreSQL 17 with ruvector extension
```

### Step 2: Run Migration SQL

```bash
# Apply RLS policies for account isolation
docker compose exec db psql -U postgres -f /docker-entrypoint-initdb.d/ruvector_setup.sql
```

### Step 3: Configure LibreFang

```toml
# ~/.librefang/config.toml

[memory]
vector_backend = "http"
vector_store_url = "http://localhost:54321/rest/v1/rpc"
vector_store_api_key = "your-supabase-anon-key"
```

### Step 4: Verify

```bash
# Test semantic search round-trip
curl -X POST http://localhost:3000/api/memory \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "X-Account-Id: acc_test" \
  -H "X-Account-Sig: ..." \
  -d '{"content": "test memory", "agent_id": "assistant"}'

curl -X POST http://localhost:3000/api/memory/search \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "X-Account-Id: acc_test" \
  -H "X-Account-Sig: ..." \
  -d '{"query": "test", "limit": 5}'
```

RLS ensures Account A's memories are invisible to Account B — enforced at the
PostgreSQL level, independent of LibreFang application code.

---

## Rollback

To disable multi-tenant and revert to single-tenant:

### Step 1: Disable in Config

```toml
[multi_tenant]
enabled = false
```

### Step 2: Restart

```bash
librefang restart
```

All data remains in the `system` account. The `account_id` columns stay in the
database but are ignored when multi-tenant is disabled.

### Full Database Rollback (Optional, Destructive)

> **Warning:** Only run this if you want to remove the `account_id` columns entirely.
> Requires SQLite ≥ 3.35.0.

```sql
-- Rollback: 001_add_account_isolation
DROP INDEX IF EXISTS idx_memories_account_id;
DROP INDEX IF EXISTS idx_memories_account_agent;
DROP INDEX IF EXISTS idx_sessions_account_id;
DROP INDEX IF EXISTS idx_sessions_account_agent;
DROP INDEX IF EXISTS idx_kv_account_id;
DROP INDEX IF EXISTS idx_proactive_account_id;

ALTER TABLE memories DROP COLUMN account_id;
ALTER TABLE sessions DROP COLUMN account_id;
ALTER TABLE kv_store DROP COLUMN account_id;
ALTER TABLE proactive_memories DROP COLUMN account_id;

DELETE FROM migrations WHERE name = '001_add_account_isolation';
```

---

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `401 Invalid signature` | HMAC secret mismatch between LibreFang and Qwntik | Ensure both use identical `hmac_secret` value |
| `401 Invalid signature` (intermittent) | HMAC secret rotated on one side but not the other | Rotate secret on both LibreFang and Qwntik simultaneously |
| Agents missing after migration | Manifests not moved to `accounts/system/agents/` | Restart LibreFang — migration is idempotent |
| `404` on valid agent | Request sent with wrong `X-Account-Id` | Verify account header matches agent's account |
| Database locked during migration | Concurrent write during ALTER TABLE | Stop LibreFang, run migration manually, restart |
| `account_id column already exists` | Migration already applied | Safe to ignore — migration is idempotent |
| Supabase connection refused | Docker not running or wrong port | Check `docker compose ps`; default port is 54321 |

---

## Security Checklist

- [ ] `hmac_secret` is at least 32 bytes (64 hex chars)
- [ ] `hmac_secret` is not committed to version control
- [ ] `hmac_secret` matches between LibreFang and upstream SaaS
- [ ] Config file permissions: `chmod 600 ~/.librefang/config.toml`
- [ ] API responses do not include `account_id` (prevents enumeration)
- [ ] Cross-account access returns `404` not `403` (prevents enumeration)
- [ ] Supabase RLS policies are active (if using vector backend)

---

## Timeline

The multi-tenant migration touches 4 phases of the implementation plan:

| Phase | What Changes | Operator Action |
|-------|-------------|----------------|
| Phase 0 (RuVector) | Docker image available | Optional: `docker compose up` |
| Phase 1 (Foundation) | `[multi_tenant]` config, middleware | Set `enabled = true` + `hmac_secret` |
| Phase 2 (Resource Isolation) | Agents/channels/skills scoped | None — transparent |
| Phase 3 (Data Isolation) | DB migration, memory scoped | None — runs on restart |
| Phase 4 (Hardening) | Security audit, this guide | Read this guide |

---

## Cross-References

| Document | Relationship |
|----------|-------------|
| MASTER-PLAN | Overall architecture and timeline |
| ADR-MT-001 | Account model decisions |
| ADR-MT-002 | Authentication & HMAC signature design |
| ADR-MT-003 | Resource isolation strategy |
| ADR-MT-004 | Data & memory isolation |
| SPEC-MT-003 | Database migration script details |
| PLAN-MT-001 | Implementation task breakdown |
| ADR-RV-001 | RuVector extension (optional Supabase backend) |
| SPEC-RV-002 | Supabase vector store configuration |
