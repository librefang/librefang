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
                        // Keep the one with higher confidence (rows are sorted desc),
                        // so rows[i] is the keeper. Soft-delete rows[j].
                        conn.execute(
                            "UPDATE memories SET deleted = 1 WHERE id = ?1",
                            rusqlite::params![rows[j].0],
                        )
                        .map_err(|e| LibreFangError::Memory(e.to_string()))?;

                        // If the absorbed memory had higher confidence somehow,
                        // update the keeper.
                        if rows[j].2 > rows[i].2 {
                            conn.execute(
                                "UPDATE memories SET confidence = ?1 WHERE id = ?2",
                                rusqlite::params![rows[j].2, rows[i].0],
                            )
                            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
                        }

                        absorbed.insert(rows[j].0.clone());
                        memories_merged += 1;
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
}
