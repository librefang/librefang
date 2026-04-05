//! Memory consolidation and decay logic.
//!
//! Reduces confidence of old, unaccessed memories and merges
//! duplicate/similar memories.

use chrono::Utc;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{text_similarity, ConsolidationReport};
use rusqlite::Connection;
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
        // Load all active memories and group pairs for merging.
        // Cap at 100 merges per consolidation run to avoid O(n²) blowup on
        // large memory stores.
        const MAX_MERGES_PER_RUN: u64 = 100;
        let mut memories_merged: u64 = 0;
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, content, confidence FROM memories WHERE deleted = 0 ORDER BY confidence DESC",
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;

            let rows: Vec<(String, String, f64)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, f64>(2)?,
                    ))
                })
                .map_err(|e| LibreFangError::Memory(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect();

            // Track which IDs have been absorbed into another memory.
            let mut absorbed: std::collections::HashSet<String> = std::collections::HashSet::new();

            'outer: for i in 0..rows.len() {
                if absorbed.contains(&rows[i].0) {
                    continue;
                }
                for j in (i + 1)..rows.len() {
                    if absorbed.contains(&rows[j].0) {
                        continue;
                    }
                    let sim = text_similarity(&rows[i].1.to_lowercase(), &rows[j].1.to_lowercase());
                    if sim > 0.9 {
                        // Keep the one with higher confidence (rows are sorted desc),
                        // so rows[i] is the keeper. Soft-delete rows[j] and, if the
                        // absorbed memory had higher confidence somehow, lift the
                        // keeper to that value. Wrap both writes in a savepoint so
                        // we never leave a keeper un-updated after its duplicate
                        // was already soft-deleted.
                        let tx = conn
                            .unchecked_transaction()
                            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
                        tx.execute(
                            "UPDATE memories SET deleted = 1 WHERE id = ?1",
                            rusqlite::params![rows[j].0],
                        )
                        .map_err(|e| LibreFangError::Memory(e.to_string()))?;

                        if rows[j].2 > rows[i].2 {
                            tx.execute(
                                "UPDATE memories SET confidence = ?1 WHERE id = ?2",
                                rusqlite::params![rows[j].2, rows[i].0],
                            )
                            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
                        }
                        tx.commit()
                            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

                        absorbed.insert(rows[j].0.clone());
                        memories_merged += 1;

                        if memories_merged >= MAX_MERGES_PER_RUN {
                            break 'outer;
                        }
                    }
                }
            }
        }

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(ConsolidationReport {
            memories_merged,
            memories_decayed: decayed as u64,
            duration_ms,
        })
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
}
