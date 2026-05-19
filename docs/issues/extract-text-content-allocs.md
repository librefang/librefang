# [Low] `extract_text_content` allocates `Vec<String>` per save

**Severity:** Low · **Domain:** Performance · **Source:** `audit-05-performance.md`

**Location:** `crates/librefang-memory/src/session.rs:501-508`

**Problem:** Allocation churn in a hot save path. Each save call constructs a fresh `Vec<String>` then joins.

**Fix:** Stream into a single `String` with `write!`/`push_str`:
```rust
let mut out = String::with_capacity(estimated);
for block in &message.content { write!(out, "{}", block.text())?; }
```
