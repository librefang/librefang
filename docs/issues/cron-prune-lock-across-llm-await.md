# `_prune_guard` is held across LLM `try_summarize_trim().await`

**Severity:** Medium
**Category:** Concurrency, races, deadlocks
**Labels:** `concurrency`, `latency`, `medium`

## Affected files
- `crates/librefang-kernel/src/kernel/cron_tick.rs:280-286` (lock acquisition)
- `crates/librefang-kernel/src/kernel/cron_tick.rs:395-402` (LLM await)
- Related: `messaging.rs:874`'s `session_id_override` path uses `session_msg_locks` the same way

## Description

The cron prune lock (`session_msg_locks[for_channel(agent,"cron")]`) is acquired at line 286 and held **across** `try_summarize_trim().await` (line 395).

Consequences: under provider congestion the await can block for tens of seconds, during which:

- Every concurrent cron fire for the same agent (persistent-cron-session shares this key) blocks;
- Automation-cron ticks that derive `SenderContext{channel:"cron"}` jam on the same key;
- The comment at `:272-279` claims the lock will be released before `send_message_full`, **but doesn't acknowledge that the LLM await still holds it**.

## Recommendation

1. Inside the lock, only do slice computation + clone;
2. Release the lock;
3. Run `try_summarize_trim` outside the lock (lock-free);
4. Re-acquire briefly to write the trimmed result back, with a generation check / CAS to guard against concurrent overwrite.
