# [High] Memory layer N+1 elimination — recall UPDATE + vector hydrate

**Severity:** High · **Domain:** Performance
**Status:** Merges 1 earlier issue into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | After recall, each fragment gets a per-row UPDATE with no transaction; runs on every tool-augmented agent turn | `memory/src/semantic.rs:414-421` |
| ANN hydrate | After ANN search, the K returned ids are hydrated one-by-one via `get_by_id` | `memory/src/semantic.rs:451-459` |

## Why merged

Both N+1s are in the same file (`semantic.rs`) on the same call chain — a single PR can eliminate both most economically.

## Combined fix plan

1. **Wrap recall UPDATEs in a transaction**:
   ```rust
   let tx = conn.transaction()?;
   let mut stmt = tx.prepare("UPDATE fragments SET last_used=?1, hit_count=hit_count+1 WHERE id=?2")?;
   for f in &recalled { stmt.execute(params![now, f.id])?; }
   tx.commit()?;
   ```
   Amortizes WAL fsync over a single transaction.

2. **Convert vector hydrate to a batched IN query**:
   ```rust
   let placeholders = (0..ids.len()).map(|_| "?").collect::<Vec<_>>().join(",");
   let sql = format!("SELECT id, ... FROM fragments WHERE id IN ({placeholders})");
   ```
   Single SELECT; for K=50, drops 50 round-trips to 1.

## Tests

- Unit test: recall of 100 fragments → write latency ≤ single WAL fsync × 2.
- Unit test: vector ANN with K=50 → SQL call count = 1 (no longer grows with K).
- Bench: end-to-end recall of 100 fragments improves ≥ 10× over baseline.
