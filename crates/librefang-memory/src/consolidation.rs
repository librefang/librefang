//! Memory consolidation and decay logic.
//!
//! Reduces confidence of old, unaccessed memories and merges
//! duplicate/similar memories.

use chrono::Utc;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{text_similarity, ConsolidationReport};
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Memory consolidation engine.
#[derive(Clone)]
pub struct ConsolidationEngine {
    conn: Arc<Mutex<Connection>>,
    /// Decay rate: how much to reduce confidence per consolidation cycle.
    decay_rate: f32,
}

impl ConsolidationEngine {
    /// Create a new consolidation engine.
    pub fn new(conn: Arc<Mutex<Connection>>, decay_rate: f32) -> Self {
        Self { conn, decay_rate }
    }

    /// Run a consolidation cycle: decay old memories.
    pub fn consolidate(&self) -> LibreFangResult<ConsolidationReport> {
        let start = std::time::Instant::now();
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        // Decay confidence of memories not accessed in the last 7 days
        let cutoff = (Utc::now() - chrono::Duration::days(7)).to_rfc3339();
        let decay_factor = 1.0 - self.decay_rate as f64;

        let decayed = conn
            .execute(
                "UPDATE memories SET confidence = MAX(0.1, confidence * ?1)
                 WHERE deleted = 0 AND accessed_at < ?2 AND confidence > 0.1",
                rusqlite::params![decay_factor, cutoff],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        // Phase 2: merge highly similar memories (>90% text similarity).
        // Load active memories per-agent to prevent cross-tenant merges: memories
        // that belong to different agents must never be compared or merged, even
        // when the global consolidation sweep runs across the shared database.
        // Cap at 100 merges per consolidation run to avoid O(n²) blowup on
        // large memory stores.
        const MAX_MERGES_PER_RUN: u64 = 100;
        let mut memories_merged: u64 = 0;

        // Collect the distinct agent_ids that have active memories so we can
        // process each tenant in isolation.
        let agent_ids: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT DISTINCT agent_id FROM memories WHERE deleted = 0")
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            rows.filter_map(|r| r.ok()).collect()
        };

        // One outer transaction wraps every merge in this run so we pay a
        // single fsync at the end instead of M (one per pair). Pre-fix
        // each pair opened its own `unchecked_transaction`; with WAL that
        // costs one fsync per commit, and `MAX_MERGES_PER_RUN = 100`
        // means up to 100 fsyncs back-to-back. Per-pair atomicity (loser
        // soft-delete + keeper update applied together) is preserved
        // automatically — both writes for a pair land in the same outer
        // tx, so a mid-pair failure rolls the whole batch back via the
        // `?` propagation below. Consolidate is idempotent and the next
        // run will pick up where this one left off, which makes
        // all-or-nothing safe here.
        let outer_tx = conn
            .unchecked_transaction()
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        'agents: for agent_id in &agent_ids {
            // Pull every column needed to merge state correctly. Pre-fix
            // we only loaded id/content/confidence and dropped the loser
            // entirely — losing metadata, access_count, and embedding
            // (#3537). Now we union metadata, sum access_count, and
            // confidence-weight embeddings before soft-deleting.
            let mut stmt = outer_tx
                .prepare(
                    "SELECT id, content, confidence, metadata, access_count, embedding \
                     FROM memories \
                     WHERE deleted = 0 AND agent_id = ?1 \
                     ORDER BY confidence DESC",
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;

            #[allow(clippy::type_complexity)]
            let mut rows: Vec<(String, String, f64, String, i64, Option<Vec<u8>>)> = stmt
                .query_map(rusqlite::params![agent_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, f64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<Vec<u8>>>(5)?,
                    ))
                })
                .map_err(|e| LibreFangError::Memory(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect();

            // Track which IDs have been absorbed into another memory.
            let mut absorbed: std::collections::HashSet<String> = std::collections::HashSet::new();
            // Per-keeper accumulated weight. Initialized lazily to the
            // keeper's original confidence; each loser merged into a
            // keeper grows its accumulated weight by the loser's
            // confidence. This makes the running embedding a true
            // confidence-weighted average over all losers absorbed by
            // a keeper, not a chain of pairwise blends biased toward
            // whichever loser arrived last.
            let mut accum_weights: HashMap<String, f32> = HashMap::new();

            for i in 0..rows.len() {
                if absorbed.contains(&rows[i].0) {
                    continue;
                }
                for j in (i + 1)..rows.len() {
                    if absorbed.contains(&rows[j].0) {
                        continue;
                    }
                    let sim = text_similarity(&rows[i].1.to_lowercase(), &rows[j].1.to_lowercase());
                    if sim > 0.9 {
                        // Keep rows[i] (sorted by confidence DESC). Merge:
                        //   - access_count: keeper + loser (sum)
                        //   - metadata: union, keeper wins on key conflict
                        //   - embedding: running confidence-weighted average
                        //                across all losers absorbed so far
                        //   - confidence: max(keeper, loser)
                        // Per-pair atomicity comes from the outer tx —
                        // both writes land in the same batch, and any
                        // mid-pair `?` aborts the whole consolidate run.
                        let keeper_id = rows[i].0.clone();
                        let loser_id = rows[j].0.clone();
                        let merged_access = rows[i].4.saturating_add(rows[j].4);
                        let merged_metadata = merge_metadata_json(&rows[i].3, &rows[j].3);
                        let keeper_w = *accum_weights
                            .entry(keeper_id.clone())
                            .or_insert(rows[i].2 as f32);
                        let loser_w = rows[j].2 as f32;
                        let merged_embedding = merge_embeddings_weighted(
                            rows[i].5.as_deref(),
                            keeper_w,
                            rows[j].5.as_deref(),
                            loser_w,
                        );
                        let merged_confidence = rows[i].2.max(rows[j].2);

                        outer_tx
                            .execute(
                                "UPDATE memories SET deleted = 1 WHERE id = ?1",
                                rusqlite::params![loser_id],
                            )
                            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

                        match merged_embedding.as_ref() {
                            Some(bytes) => {
                                outer_tx
                                    .execute(
                                        "UPDATE memories SET confidence = ?1, \
                                         access_count = ?2, metadata = ?3, \
                                         embedding = ?4 WHERE id = ?5",
                                        rusqlite::params![
                                            merged_confidence,
                                            merged_access,
                                            &merged_metadata,
                                            bytes,
                                            keeper_id,
                                        ],
                                    )
                                    .map_err(|e| LibreFangError::Memory(e.to_string()))?;
                            }
                            None => {
                                outer_tx
                                    .execute(
                                        "UPDATE memories SET confidence = ?1, \
                                         access_count = ?2, metadata = ?3 \
                                         WHERE id = ?4",
                                        rusqlite::params![
                                            merged_confidence,
                                            merged_access,
                                            &merged_metadata,
                                            keeper_id,
                                        ],
                                    )
                                    .map_err(|e| LibreFangError::Memory(e.to_string()))?;
                            }
                        }

                        // Update the in-memory row so subsequent merges
                        // against the same keeper see the accumulated state.
                        rows[i].2 = merged_confidence;
                        rows[i].3 = merged_metadata;
                        rows[i].4 = merged_access;
                        if merged_embedding.is_some() {
                            rows[i].5 = merged_embedding;
                        }
                        // Grow the keeper's accumulated weight by the
                        // loser's confidence so the next merge against
                        // this keeper averages over the full absorbed
                        // history (not just the most recent pair). The
                        // entry is guaranteed to exist — we read from it
                        // a few lines above via `or_insert(...)`.
                        if let Some(w) = accum_weights.get_mut(&keeper_id) {
                            *w += loser_w;
                        }
                        absorbed.insert(loser_id);
                        memories_merged += 1;

                        if memories_merged >= MAX_MERGES_PER_RUN {
                            break 'agents;
                        }
                    }
                }
            }
        }

        // Single commit for the whole batch — collapses up to
        // MAX_MERGES_PER_RUN fsyncs into one. If no merges happened the
        // outer tx is still committed (a no-op write), which is cheaper
        // than guarding the commit on `memories_merged > 0`.
        outer_tx
            .commit()
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(ConsolidationReport {
            memories_merged,
            memories_decayed: decayed as u64,
            duration_ms,
        })
    }
}

/// Merge two metadata JSON strings; on key collision keeper wins.
///
/// Both sides must parse as a JSON object for the union semantics to
/// apply. If either side is a non-object value (array, string, number,
/// malformed) we cannot safely union and instead preserve the keeper
/// verbatim — silently coercing a non-object payload to `{}` would
/// regress the pre-PR behavior of preserving the loser's metadata
/// untouched (and outright destroy a non-object keeper's payload).
fn merge_metadata_json(keeper: &str, loser: &str) -> String {
    let keeper_obj = serde_json::from_str::<serde_json::Value>(keeper)
        .ok()
        .and_then(|v| match v {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        });
    let loser_obj = serde_json::from_str::<serde_json::Value>(loser)
        .ok()
        .and_then(|v| match v {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        });
    match (keeper_obj, loser_obj) {
        (Some(keeper_map), Some(loser_map)) => {
            let mut merged = loser_map;
            for (k, v) in keeper_map {
                merged.insert(k, v); // keeper wins on conflict
            }
            serde_json::to_string(&merged).unwrap_or_else(|_| keeper.to_string())
        }
        // Either side is non-object → cannot safely union; keep keeper.
        _ => keeper.to_string(),
    }
}

/// Decode embedding bytes (LE f32) into a `Vec<f32>`.
fn decode_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

/// Encode `Vec<f32>` back to LE bytes for SQLite BLOB storage.
fn encode_embedding(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Merge two embeddings via weighted average.
///
/// `keeper_w` is the keeper's *running* accumulated weight (sum of every
/// confidence it has absorbed so far, not just the original row's
/// confidence). `loser_w` is the new loser's confidence. The caller is
/// responsible for growing `keeper_w` after each successful merge so a
/// keeper that absorbs N losers averages over the full history rather
/// than re-blending pairwise from its original weight every time.
///
/// - both present, same dim → weighted average bytes
/// - both present, dim mismatch → keeper (the one with higher accum weight)
/// - keeper has bytes, loser does not → keeper verbatim
/// - keeper has none, loser has bytes → **loser is adopted** (asymmetric:
///   the keeper inherits the loser's vector rather than staying empty,
///   because some embedding is strictly more useful for downstream
///   ranking than none, and the loser is about to be soft-deleted so
///   its vector would otherwise be lost)
/// - neither → `None`
///
/// Negative or zero weights are clamped to a small positive epsilon so
/// the average remains well-defined.
fn merge_embeddings_weighted(
    keeper: Option<&[u8]>,
    keeper_w: f32,
    loser: Option<&[u8]>,
    loser_w: f32,
) -> Option<Vec<u8>> {
    match (keeper, loser) {
        (Some(k), Some(l)) => {
            let kv = decode_embedding(k);
            let lv = decode_embedding(l);
            match (kv, lv) {
                (Some(kv), Some(lv)) if kv.len() == lv.len() && !kv.is_empty() => {
                    let kw = keeper_w.max(f32::EPSILON);
                    let lw = loser_w.max(f32::EPSILON);
                    let total = kw + lw;
                    let merged: Vec<f32> = kv
                        .iter()
                        .zip(lv.iter())
                        .map(|(a, b)| (a * kw + b * lw) / total)
                        .collect();
                    Some(encode_embedding(&merged))
                }
                // Dim mismatch or decode failure → preserve keeper.
                _ => Some(k.to_vec()),
            }
        }
        (Some(k), None) => Some(k.to_vec()),
        (None, Some(l)) => Some(l.to_vec()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn setup() -> ConsolidationEngine {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        ConsolidationEngine::new(Arc::new(Mutex::new(conn)), 0.1)
    }

    #[test]
    fn test_consolidation_empty() {
        let engine = setup();
        let report = engine.consolidate().unwrap();
        assert_eq!(report.memories_decayed, 0);
    }

    #[test]
    fn test_consolidation_decays_old_memories() {
        let engine = setup();
        let conn = engine.conn.lock().unwrap();
        // Insert an old memory
        let old_date = (Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted)
             VALUES ('test-id', 'agent-1', 'old memory', '\"conversation\"', 'episodic', 0.9, '{}', ?1, ?1, 0, 0)",
            rusqlite::params![old_date],
        ).unwrap();
        drop(conn);

        let report = engine.consolidate().unwrap();
        assert_eq!(report.memories_decayed, 1);

        // Verify confidence was reduced
        let conn = engine.conn.lock().unwrap();
        let confidence: f64 = conn
            .query_row(
                "SELECT confidence FROM memories WHERE id = 'test-id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(confidence < 0.9);
    }

    // --- Phase 2 memory merge tests --------------------------------------

    /// Helper: insert a memory with the given id, content, and confidence.
    fn insert_memory(conn: &Connection, id: &str, content: &str, confidence: f64) {
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted)
             VALUES (?1, 'agent-1', ?2, '\"conversation\"', 'episodic', ?3, '{}', ?4, ?4, 0, 0)",
            rusqlite::params![id, content, confidence, now],
        ).unwrap();
    }

    /// Helper: check whether a memory is soft-deleted.
    fn is_deleted(conn: &Connection, id: &str) -> bool {
        conn.query_row(
            "SELECT deleted FROM memories WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get::<_, i32>(0),
        )
        .unwrap()
            == 1
    }

    #[test]
    fn test_merge_similar_memories() {
        let engine = setup();
        {
            let conn = engine.conn.lock().unwrap();
            // Two memories with >90% word overlap (identical content).
            insert_memory(
                &conn,
                "mem-a",
                "the quick brown fox jumps over the lazy dog",
                0.8,
            );
            insert_memory(
                &conn,
                "mem-b",
                "the quick brown fox jumps over the lazy dog",
                0.7,
            );
        }

        let report = engine.consolidate().unwrap();
        assert_eq!(report.memories_merged, 1);

        let conn = engine.conn.lock().unwrap();
        // Higher-confidence memory (mem-a, 0.8) is kept; lower one is soft-deleted.
        assert!(!is_deleted(&conn, "mem-a"));
        assert!(is_deleted(&conn, "mem-b"));
    }

    #[test]
    fn test_no_merge_dissimilar_memories() {
        let engine = setup();
        {
            let conn = engine.conn.lock().unwrap();
            // Two completely different memories — Jaccard similarity ≈ 0.
            insert_memory(
                &conn,
                "mem-x",
                "the quick brown fox jumps over the lazy dog",
                0.8,
            );
            insert_memory(
                &conn,
                "mem-y",
                "a completely unrelated sentence about space travel and rockets",
                0.7,
            );
        }

        let report = engine.consolidate().unwrap();
        assert_eq!(report.memories_merged, 0);

        let conn = engine.conn.lock().unwrap();
        assert!(!is_deleted(&conn, "mem-x"));
        assert!(!is_deleted(&conn, "mem-y"));
    }

    #[test]
    fn test_merge_keeps_higher_confidence() {
        let engine = setup();
        {
            let conn = engine.conn.lock().unwrap();
            // mem-lo has lower confidence but is inserted first.
            // mem-hi has higher confidence.
            // Since rows are sorted by confidence DESC, mem-hi is the keeper
            // and mem-lo gets absorbed. mem-hi keeps its higher confidence.
            insert_memory(
                &conn,
                "mem-lo",
                "the quick brown fox jumps over the lazy dog",
                0.5,
            );
            insert_memory(
                &conn,
                "mem-hi",
                "the quick brown fox jumps over the lazy dog",
                0.9,
            );
        }

        let report = engine.consolidate().unwrap();
        assert_eq!(report.memories_merged, 1);

        let conn = engine.conn.lock().unwrap();
        // mem-hi (0.9) is sorted first and is the keeper.
        assert!(!is_deleted(&conn, "mem-hi"));
        assert!(is_deleted(&conn, "mem-lo"));

        let confidence: f64 = conn
            .query_row(
                "SELECT confidence FROM memories WHERE id = 'mem-hi'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!((confidence - 0.9).abs() < f64::EPSILON);
    }

    /// Helper: insert a memory belonging to a specific agent_id.
    fn insert_memory_for_agent(
        conn: &Connection,
        id: &str,
        agent_id: &str,
        content: &str,
        confidence: f64,
    ) {
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted)
             VALUES (?1, ?2, ?3, '\"conversation\"', 'episodic', ?4, '{}', ?5, ?5, 0, 0)",
            rusqlite::params![id, agent_id, content, confidence, now],
        ).unwrap();
    }

    /// Identical content belonging to two different agents must NOT be merged.
    /// Before the fix, the SELECT had no agent_id filter and would load all
    /// tenants' memories into the same comparison set, causing cross-tenant
    /// soft-deletes (data leak / data loss).
    #[test]
    fn test_no_cross_tenant_merge() {
        let engine = setup();
        {
            let conn = engine.conn.lock().unwrap();
            // Same content, same high similarity — but different agents.
            insert_memory_for_agent(
                &conn,
                "agent-a-mem",
                "agent-a",
                "the quick brown fox jumps over the lazy dog",
                0.8,
            );
            insert_memory_for_agent(
                &conn,
                "agent-b-mem",
                "agent-b",
                "the quick brown fox jumps over the lazy dog",
                0.7,
            );
        }

        let report = engine.consolidate().unwrap();
        // Cross-tenant merge must not happen — 0 merges expected.
        assert_eq!(report.memories_merged, 0);

        let conn = engine.conn.lock().unwrap();
        // Both memories from different agents must survive intact.
        assert!(!is_deleted(&conn, "agent-a-mem"));
        assert!(!is_deleted(&conn, "agent-b-mem"));
    }

    /// Helper: insert with explicit metadata, access_count, and embedding.
    fn insert_memory_full(
        conn: &Connection,
        id: &str,
        content: &str,
        confidence: f64,
        metadata: &str,
        access_count: i64,
        embedding: Option<&[f32]>,
    ) {
        let now = Utc::now().to_rfc3339();
        let emb_bytes: Option<Vec<u8>> = embedding.map(|v| {
            let mut out = Vec::with_capacity(v.len() * 4);
            for f in v {
                out.extend_from_slice(&f.to_le_bytes());
            }
            out
        });
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted, embedding)
             VALUES (?1, 'agent-1', ?2, '\"conversation\"', 'episodic', ?3, ?4, ?5, ?5, ?6, 0, ?7)",
            rusqlite::params![id, content, confidence, metadata, now, access_count, emb_bytes],
        ).unwrap();
    }

    /// #3537: merging duplicates must preserve metadata, sum access_count,
    /// and combine embeddings — not silently drop them with the loser row.
    #[test]
    fn test_merge_preserves_metadata_access_count_and_embedding() {
        let engine = setup();
        {
            let conn = engine.conn.lock().unwrap();
            // Same content; keeper has higher confidence so it wins. The
            // loser carries unique metadata, a non-zero access_count, and
            // a real embedding — all of which would be lost pre-fix.
            insert_memory_full(
                &conn,
                "mem-keeper",
                "the quick brown fox jumps over the lazy dog",
                0.9,
                r#"{"source":"keeper","tag":"a"}"#,
                3,
                Some(&[1.0_f32, 0.0, 0.0, 0.0]),
            );
            insert_memory_full(
                &conn,
                "mem-loser",
                "the quick brown fox jumps over the lazy dog",
                0.5,
                r#"{"loser_only":"value","tag":"b"}"#,
                7,
                Some(&[0.0_f32, 1.0, 0.0, 0.0]),
            );
        }

        let report = engine.consolidate().unwrap();
        assert_eq!(report.memories_merged, 1);

        let conn = engine.conn.lock().unwrap();
        assert!(!is_deleted(&conn, "mem-keeper"));
        assert!(is_deleted(&conn, "mem-loser"));

        // access_count must be the SUM (3 + 7 = 10), not just keeper's.
        let access: i64 = conn
            .query_row(
                "SELECT access_count FROM memories WHERE id = 'mem-keeper'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(access, 10, "access_count should sum keeper + loser");

        // Loser-only metadata key must survive; keeper wins on conflict.
        let metadata: String = conn
            .query_row(
                "SELECT metadata FROM memories WHERE id = 'mem-keeper'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let parsed: HashMap<String, serde_json::Value> = serde_json::from_str(&metadata).unwrap();
        assert_eq!(
            parsed.get("loser_only").and_then(|v| v.as_str()),
            Some("value"),
            "loser-only metadata key must be preserved"
        );
        assert_eq!(
            parsed.get("source").and_then(|v| v.as_str()),
            Some("keeper"),
            "keeper wins on metadata key conflict"
        );
        assert_eq!(
            parsed.get("tag").and_then(|v| v.as_str()),
            Some("a"),
            "keeper wins on metadata key conflict (tag)"
        );

        // Embedding must be non-null and a real weighted blend of both.
        let emb_bytes: Option<Vec<u8>> = conn
            .query_row(
                "SELECT embedding FROM memories WHERE id = 'mem-keeper'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let emb_bytes = emb_bytes.expect("embedding must not be null after merge");
        assert_eq!(emb_bytes.len(), 16, "4 f32 = 16 bytes");
        let emb: Vec<f32> = emb_bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        // Both axes should be > 0 since we blended (1,0,0,0) and (0,1,0,0).
        assert!(
            emb[0] > 0.0 && emb[1] > 0.0,
            "weighted blend should mix both vectors"
        );
    }

    /// Non-object metadata (JSON array, string, number, malformed) on
    /// either side must NOT silently coerce the merged value to `{}`.
    /// Pre-fix-fixup the code parsed both sides as a `HashMap<String, _>`
    /// and `unwrap_or_default()`'d on failure — destroying whatever the
    /// keeper had.
    #[test]
    fn test_merge_metadata_preserves_non_object_keeper() {
        // Loser is a JSON array (legacy data). The keeper's object
        // metadata must survive untouched, NOT be wiped to `{}`.
        let merged = merge_metadata_json(r#"{"k":"v"}"#, r#"[1,2,3]"#);
        let parsed: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(parsed["k"], "v", "keeper key must survive non-object loser");

        // Keeper is a non-object value. We can't safely union, so the
        // keeper must be returned verbatim — neither side's data lost.
        let merged = merge_metadata_json(r#""legacy_string""#, r#"{"k":"v"}"#);
        assert_eq!(
            merged, r#""legacy_string""#,
            "non-object keeper must be preserved verbatim"
        );

        // Malformed JSON on either side falls into the same branch.
        let merged = merge_metadata_json(r#"{"k":"v"}"#, "not-json-at-all");
        let parsed: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(parsed["k"], "v");
    }

    /// #3537 follow-up: with three or more losers absorbed by the same
    /// keeper, the resulting embedding must reflect a running confidence
    /// -weighted average — not a chain of pairwise blends biased toward
    /// whichever loser arrived last.
    ///
    /// We exploit a degenerate case to detect the bug deterministically:
    /// keeper has confidence 0.9, two losers (each confidence 0.45) carry
    /// IDENTICAL embeddings on a different axis from the keeper. With a
    /// proper running average the keeper's accumulated weight after the
    /// first merge becomes 0.9 + 0.45 = 1.35, so the second merge weighs
    /// the keeper at 1.35 vs the loser at 0.45 — keeper still dominates.
    /// Pre-fix the second merge would re-blend with the original 0.9
    /// keeper weight, letting the loser axis grow disproportionately.
    #[test]
    fn test_merge_embeddings_running_weighted_average_across_multiple_losers() {
        let engine = setup();
        {
            let conn = engine.conn.lock().unwrap();
            // Keeper points along x with high confidence.
            insert_memory_full(
                &conn,
                "k",
                "the quick brown fox jumps over the lazy dog",
                0.9,
                "{}",
                1,
                Some(&[1.0_f32, 0.0]),
            );
            // Two losers with the same content + same embedding (along y).
            // Lower confidence each.
            insert_memory_full(
                &conn,
                "l1",
                "the quick brown fox jumps over the lazy dog",
                0.45,
                "{}",
                1,
                Some(&[0.0_f32, 1.0]),
            );
            insert_memory_full(
                &conn,
                "l2",
                "the quick brown fox jumps over the lazy dog",
                0.45,
                "{}",
                1,
                Some(&[0.0_f32, 1.0]),
            );
        }

        let report = engine.consolidate().unwrap();
        assert_eq!(report.memories_merged, 2);

        let conn = engine.conn.lock().unwrap();
        let emb_bytes: Vec<u8> = conn
            .query_row("SELECT embedding FROM memories WHERE id = 'k'", [], |row| {
                row.get::<_, Option<Vec<u8>>>(0)
            })
            .unwrap()
            .expect("embedding present");
        let emb: Vec<f32> = emb_bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();

        // After running weighted average:
        //   step 1: (1,0)*0.9 + (0,1)*0.45 / 1.35 = (0.667, 0.333)
        //   step 2: (0.667, 0.333)*1.35 + (0,1)*0.45 / 1.8 = (0.5, 0.5)
        // Pre-fix (pairwise re-blend with original 0.9 weight each step):
        //   step 1: (0.667, 0.333)
        //   step 2: re-blend with (0.9, 0.45) weights →
        //     (0.667*0.9 + 0*0.45)/1.35 = 0.444 on x, similar drift on y
        // The running-average path keeps x dominant; the buggy path lets
        // x decay further. Assert x >= 0.45 — true under running avg
        // (0.5), false under pre-fix pairwise (~0.44).
        assert!(
            emb[0] >= 0.45,
            "running weighted average should keep keeper axis dominant; got {:?}",
            emb
        );
    }

    /// A dim-mismatched loser must not corrupt the keeper's embedding,
    /// must not crash the merge, and must not poison subsequent merges
    /// against the same keeper. The keeper's bytes flow through verbatim
    /// (the `(Some, Some, dim mismatch)` branch in
    /// `merge_embeddings_weighted`), and the next same-dim loser is then
    /// blended in normally.
    #[test]
    fn test_merge_embeddings_handles_dim_mismatch_then_same_dim() {
        let engine = setup();
        {
            let conn = engine.conn.lock().unwrap();
            // Sorted by confidence DESC: k → l_bad → l_ok.
            insert_memory_full(
                &conn,
                "k",
                "the quick brown fox jumps over the lazy dog",
                0.9,
                "{}",
                1,
                Some(&[1.0_f32, 0.0, 0.0, 0.0]),
            );
            insert_memory_full(
                &conn,
                "l_bad",
                "the quick brown fox jumps over the lazy dog",
                0.5,
                "{}",
                1,
                Some(&[0.0_f32, 1.0, 0.0]), // 3-dim → mismatch with keeper's 4-dim
            );
            insert_memory_full(
                &conn,
                "l_ok",
                "the quick brown fox jumps over the lazy dog",
                0.45,
                "{}",
                1,
                Some(&[0.0_f32, 1.0, 0.0, 0.0]),
            );
        }

        let report = engine.consolidate().unwrap();
        assert_eq!(report.memories_merged, 2, "both losers must be absorbed");

        let conn = engine.conn.lock().unwrap();
        // Keeper still holds a 4-dim embedding (no dim corruption from
        // the mismatched loser).
        let emb_bytes: Vec<u8> = conn
            .query_row("SELECT embedding FROM memories WHERE id = 'k'", [], |row| {
                row.get::<_, Option<Vec<u8>>>(0)
            })
            .unwrap()
            .expect("embedding must remain present");
        assert_eq!(
            emb_bytes.len(),
            16,
            "keeper must stay 4-dim after dim-mismatched merge"
        );

        let emb: Vec<f32> = emb_bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        // After the dim-mismatched merge the keeper bytes are preserved
        // verbatim; after the subsequent same-dim merge, x stays the
        // dominant axis but y picks up some loser contribution.
        assert!(
            emb[0] > 0.0 && emb[1] > 0.0,
            "expected blended axes, got {:?}",
            emb
        );
        assert!(
            emb[2] == 0.0 && emb[3] == 0.0,
            "no contribution to unused axes, got {:?}",
            emb
        );

        // Both losers soft-deleted; access_count summed across all three.
        assert!(is_deleted(&conn, "l_bad"));
        assert!(is_deleted(&conn, "l_ok"));
        let access: i64 = conn
            .query_row(
                "SELECT access_count FROM memories WHERE id = 'k'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            access, 3,
            "access_count must sum 1+1+1 across keeper + 2 losers"
        );
    }

    /// `merge_embeddings_weighted` is asymmetric on the (None, Some) edge:
    /// when the keeper has no embedding but the loser does, the loser's
    /// bytes are adopted by the keeper rather than left as `None`. Pre-fix
    /// the loser's embedding was unconditionally lost on soft-delete; this
    /// path is what rescues it. Asserted at the helper level so a future
    /// refactor that flips the asymmetry (e.g., "keeper always wins, even
    /// when empty") fails loudly here instead of silently regressing #3537.
    #[test]
    fn test_merge_embeddings_keeper_none_adopts_loser() {
        let loser_bytes = encode_embedding(&[0.25_f32, 0.5, 0.75, 1.0]);
        let merged = merge_embeddings_weighted(None, 0.0, Some(&loser_bytes), 0.5)
            .expect("loser-only path must produce Some");
        let decoded = decode_embedding(&merged).expect("merged bytes decode");
        assert_eq!(
            decoded,
            vec![0.25_f32, 0.5, 0.75, 1.0],
            "keeper-without-embedding must adopt the loser's vector verbatim"
        );

        // Sanity: the symmetric (Some, None) path still wins for the keeper.
        let keeper_bytes = encode_embedding(&[1.0_f32, 0.0]);
        let merged = merge_embeddings_weighted(Some(&keeper_bytes), 0.9, None, 0.0)
            .expect("keeper-only path must produce Some");
        assert_eq!(decode_embedding(&merged).unwrap(), vec![1.0_f32, 0.0]);

        // And (None, None) stays None.
        assert!(merge_embeddings_weighted(None, 0.0, None, 0.0).is_none());
    }
}
