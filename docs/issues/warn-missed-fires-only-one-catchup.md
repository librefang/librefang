# [Medium] Cron subsystem hardening — missed-fire policy, persist write amplification, registration-time validation

**Severity:** Medium
**Category:** Scheduling · DoS · Validation
**Status:** Merges 2 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Severity | Description |
|--------|----------|-------------|
| this | Medium | `warn_missed_fires` logs N but fires only 1 catch-up; the log and behaviour disagree; for `session_mode="new"` jobs, `missed_count-1` fires are silently dropped |
| persist amplification | Low | `persist()` rewrites the whole `cron_jobs.json` on every API mutation; an attacker spamming edits burns IOPS |
| late validation | Low | Cron expressions are validated only at fire time; illegal expressions sit silently in storage and warn once per tick |

## Affected files

- `crates/librefang-kernel/src/cron.rs:445-481` (`warn_missed_fires`)
- `crates/librefang-kernel/src/cron.rs:561-565` (conflict with `log_missed_fires_since`)
- `crates/librefang-kernel/src/cron.rs:131-163` (`persist()`)
- `crates/librefang-kernel/src/cron.rs:786-802` (field-count normalization)
- Callers: cron-registration path in `crates/librefang-api/src/routes/workflows.rs`

## Why merged

All three are cron-subsystem runtime / registration hygiene items concentrated in `kernel/cron.rs`.

## Combined fix plan

1. **Pick an explicit missed-fire policy (this)** — choose one:
   - **Genuine N-replay**: iterate `missed_count` fires, each using `SessionId::for_cron_run(agent, "<job_id>:<scheduled>")` for per-fire isolation;
   - **Coalesce to 1**: rewrite the log as "missed N fires, coalesced into 1 catch-up at <now>" and document the semantics.

   Either way, log and behaviour must agree.
2. **Persistence debounce (persist amplification)**: `persist()` becomes a 500 ms window with a dirty bit + ≤ 1 Hz background flush. The skill workshop already uses this pattern; lift it into a shared helper.
3. **Registration-time validation (late validation)**: extract `validate_cron_expr(&str) -> Result<(), String>` covering field-count normalization + `cron::Schedule::from_str`. Every route that accepts a cron string (`routes/workflows.rs`, etc.) calls it on the create path, returning 400 with the parse error.

## Tests

- (this) Daemon offline for 5 scheduling cycles; on restart: policy 1 → 5 fires; policy 2 → 1 fire + log line "coalesced 5 fires."
- (persist amplification) 100 edits in 1 second → at most 2 actual disk writes.
- (late validation) `POST /api/cron` body `cron = "garbage"` → 400 with parse error; nothing reaches storage.
