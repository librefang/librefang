# SPEC-MT-003: Database Migration

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `ADR-MT-004-DATA-MEMORY-ISOLATION.md`

---

## Purpose

Define the database migration rules required to make tenant ownership explicit in
the Qwntik multi-tenant runtime.

This spec does **not** preserve `"system sees all"` database semantics as a target
behavior.

---

## Migration Rules

1. Every tenant-owned table must gain a concrete `account_id`
2. New writes must persist a non-null tenant `account_id`
3. Query paths must filter by `account_id` where the data model is tenant-owned
4. Transitional legacy rows may exist during migration, but the end state is explicit ownership

---

## Tables in Scope

Tenant-sensitive tables include at least:

- sessions
- events
- kv_store
- task_queue
- memories
- entities
- relations
- usage_events
- canonical_sessions
- paired_devices
- audit_entries
- prompt_versions
- prompt_experiments
- approval_audit

Tables that are purely migration metadata or structurally derived can be handled separately.

---

## Acceptance Criteria

### AC-1: Tenant-owned rows carry `account_id`

- Given a tenant-owned table after migration
- Then rows persist a concrete `account_id`

### AC-2: New writes require account ownership

- Given a post-migration write path
- When creating tenant-owned data
- Then the write includes a concrete tenant `account_id`

### AC-3: Query paths scope by `account_id`

- Given tenant-owned data from accounts A and B
- When querying as account A
- Then only account A's visible rows are returned

### AC-4: No target reliance on `"system"` query bypass

- Given the intended end state
- Then store/query APIs do not depend on `"system"` meaning “see all data”

---

## Implementation Notes

- During transition, some migrations may use temporary defaults to backfill existing rows
- Those defaults are migration mechanics, not long-term permission semantics
- Follow-up cleanup should remove any runtime dependency on legacy `"system"` behavior
