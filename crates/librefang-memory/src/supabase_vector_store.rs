//! Supabase-backed vector store implementation.
//!
//! Delegates all vector operations to Supabase PostgREST RPC endpoints,
//! allowing LibreFang to use a Supabase project as its vector database
//! via the `vector_insert`, `vector_search`, and `vector_delete` RPCs.
//!
//! ## Expected RPC contract
//!
//! | RPC function       | Parameters                                              | Response                          |
//! |--------------------|---------------------------------------------------------|-----------------------------------|
//! | `vector_insert`    | `doc_content, doc_embedding, doc_metadata, doc_user_id` | `BIGINT` (new row ID)             |
//! | `vector_search`    | `query_embedding, match_count, match_threshold, caller_user_id` | `[{ id, content, metadata, distance }]` |
//! | `vector_delete`    | `doc_id`                                                | `{}`                              |

use async_trait::async_trait;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{MemoryFilter, VectorSearchResult, VectorStore};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A [`VectorStore`] that talks to Supabase PostgREST RPC endpoints.
#[derive(Clone)]
pub struct SupabaseVectorStore {
    client: Client,
    base_url: String,
    api_key: String,
    /// Cosine distance threshold for search (lower = stricter).
    /// Default 0.5 means results with similarity ≥ 0.5 are returned.
    match_threshold: f32,
}

impl SupabaseVectorStore {
    /// Create a new Supabase vector store pointing at `base_url`.
    ///
    /// `base_url` should be the Supabase REST URL, e.g.
    /// `https://<project>.supabase.co/rest/v1`.  Trailing slashes are stripped.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            match_threshold: 0.5,
        }
    }

    /// Set the cosine distance threshold for search results.
    ///
    /// Lower values are stricter (closer matches only).
    /// `0.3` = similarity ≥ 0.7, `0.5` = similarity ≥ 0.5, `1.0` = return everything.
    pub fn with_match_threshold(mut self, threshold: f32) -> Self {
        self.match_threshold = threshold;
        self
    }

    /// Build the full URL for an RPC function.
    fn rpc_url(&self, function_name: &str) -> String {
        format!("{}/rpc/{}", self.base_url, function_name)
    }
}

/// Convert an embedding slice to a PostgreSQL-compatible TEXT string.
///
/// Returns a string like `"[0.1,0.2,0.3]"` — a plain text representation
/// that the Supabase RPC casts to `vector` on the server side.
fn embedding_to_text(embedding: &[f32]) -> String {
    let inner: Vec<String> = embedding.iter().map(|v| v.to_string()).collect();
    format!("[{}]", inner.join(","))
}

// ── Request / response DTOs ──────────────────────────────────────────────

#[derive(Serialize)]
struct SupabaseInsertRequest<'a> {
    doc_content: &'a str,
    doc_embedding: String,
    doc_metadata: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_user_id: Option<String>,
}

#[derive(Serialize)]
struct SupabaseSearchRequest {
    query_embedding: String,
    match_count: usize,
    match_threshold: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    caller_user_id: Option<String>,
}

#[derive(Deserialize)]
struct SupabaseSearchResponseItem {
    id: i64,
    content: String,
    #[serde(default)]
    metadata: serde_json::Value,
    distance: f32,
}

#[derive(Serialize)]
struct SupabaseDeleteRequest {
    doc_id: i64,
}

// ── VectorStore implementation ───────────────────────────────────────────

#[async_trait]
impl VectorStore for SupabaseVectorStore {
    async fn insert(
        &self,
        id: &str,
        embedding: &[f32],
        payload: &str,
        metadata: HashMap<String, serde_json::Value>,
    ) -> LibreFangResult<()> {
        // Extract user_id from metadata before we move it.
        let doc_user_id = metadata
            .get("user_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Merge the trait-level `id` into metadata so it survives the round-trip.
        let mut meta_map = metadata;
        meta_map.insert(
            "librefang_id".to_string(),
            serde_json::Value::String(id.to_string()),
        );
        let doc_metadata = serde_json::to_value(&meta_map)
            .map_err(|e| LibreFangError::Internal(format!("Supabase metadata serialize: {e}")))?;

        let body = SupabaseInsertRequest {
            doc_content: payload,
            doc_embedding: embedding_to_text(embedding),
            doc_metadata,
            doc_user_id,
        };

        let resp = self
            .client
            .post(self.rpc_url("vector_insert"))
            .header("apikey", &self.api_key)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Prefer", "return=representation")
            .json(&body)
            .send()
            .await
            .map_err(|e| LibreFangError::Internal(format!("Supabase vector insert: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LibreFangError::Internal(format!(
                "Supabase vector insert returned {status}: {text}"
            )));
        }
        // RPC returns the new BIGINT row ID. We discard it because the
        // VectorStore trait returns () on insert. The ID is recoverable
        // via search (returned as VectorSearchResult.id).
        Ok(())
    }

    async fn search(
        &self,
        query_embedding: &[f32],
        limit: usize,
        filter: Option<MemoryFilter>,
    ) -> LibreFangResult<Vec<VectorSearchResult>> {
        let caller_user_id = filter
            .as_ref()
            .and_then(|f| f.metadata.get("user_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let body = SupabaseSearchRequest {
            query_embedding: embedding_to_text(query_embedding),
            match_count: limit,
            match_threshold: self.match_threshold,
            caller_user_id,
        };

        let resp = self
            .client
            .post(self.rpc_url("vector_search"))
            .header("apikey", &self.api_key)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| LibreFangError::Internal(format!("Supabase vector search: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LibreFangError::Internal(format!(
                "Supabase vector search returned {status}: {text}"
            )));
        }

        let items: Vec<SupabaseSearchResponseItem> = resp
            .json()
            .await
            .map_err(|e| LibreFangError::Internal(format!("Supabase vector search parse: {e}")))?;

        Ok(items
            .into_iter()
            .map(|item| {
                let metadata: HashMap<String, serde_json::Value> =
                    serde_json::from_value(item.metadata).unwrap_or_default();
                VectorSearchResult {
                    id: item.id.to_string(),
                    payload: item.content,
                    score: (1.0 - item.distance).max(0.0),
                    metadata,
                }
            })
            .collect())
    }

    async fn delete(&self, id: &str) -> LibreFangResult<()> {
        let doc_id: i64 = id.parse().map_err(|e| {
            LibreFangError::Internal(format!("Supabase vector delete: invalid id '{id}': {e}"))
        })?;

        let body = SupabaseDeleteRequest { doc_id };

        let resp = self
            .client
            .post(self.rpc_url("vector_delete"))
            .header("apikey", &self.api_key)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| LibreFangError::Internal(format!("Supabase vector delete: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LibreFangError::Internal(format!(
                "Supabase vector delete returned {status}: {text}"
            )));
        }
        Ok(())
    }

    async fn get_embeddings(&self, _ids: &[&str]) -> LibreFangResult<HashMap<String, Vec<f32>>> {
        tracing::debug!("SupabaseVectorStore::get_embeddings not supported, returning empty");
        Ok(HashMap::new())
    }

    fn backend_name(&self) -> &str {
        "supabase"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpc_url_building() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "test-key");
        assert_eq!(
            store.rpc_url("vector_search"),
            "https://abc.supabase.co/rest/v1/rpc/vector_search"
        );
        assert_eq!(
            store.rpc_url("vector_insert"),
            "https://abc.supabase.co/rest/v1/rpc/vector_insert"
        );
    }

    #[test]
    fn test_trailing_slash_stripped() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1/", "test-key");
        assert_eq!(
            store.rpc_url("vector_search"),
            "https://abc.supabase.co/rest/v1/rpc/vector_search"
        );
    }

    #[test]
    fn test_embedding_to_text() {
        assert_eq!(embedding_to_text(&[0.1, 0.2, 0.3]), "[0.1,0.2,0.3]");
    }

    #[test]
    fn test_embedding_to_text_empty() {
        assert_eq!(embedding_to_text(&[]), "[]");
    }

    #[test]
    fn test_score_inversion() {
        let score_close = (1.0_f32 - 0.2).max(0.0);
        assert!(
            (score_close - 0.8).abs() < 1e-6,
            "expected ~0.8, got {score_close}"
        );

        let score_far = (1.0_f32 - 1.5).max(0.0);
        assert_eq!(score_far, 0.0);
    }

    #[test]
    fn test_backend_name() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "test-key");
        assert_eq!(store.backend_name(), "supabase");
    }

    #[test]
    fn test_with_match_threshold() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "key")
            .with_match_threshold(0.3);
        assert!((store.match_threshold - 0.3).abs() < 1e-6);
    }

    #[test]
    fn test_default_match_threshold() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "key");
        assert!((store.match_threshold - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_embedding_to_text_precision() {
        // Verify f32 edge cases don't produce scientific notation surprises
        let result = embedding_to_text(&[0.0, 1.0, -1.0, 0.00001]);
        assert!(result.starts_with('['));
        assert!(result.ends_with(']'));
        assert_eq!(result.matches(',').count(), 3);
    }

    #[test]
    fn test_embedding_to_text_single() {
        assert_eq!(embedding_to_text(&[0.5]), "[0.5]");
    }
}
