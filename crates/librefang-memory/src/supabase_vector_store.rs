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
//! | `vector_delete`    | `doc_id`                                                | `BOOLEAN` (`true` if deleted)     |

use async_trait::async_trait;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{MemoryFilter, VectorSearchResult, VectorStore};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Default HTTP request timeout for PostgREST calls.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Metadata key used to stash the VectorStore trait `id` inside the
/// Supabase document metadata. Consumers recovering the original ID
/// from search results should read `metadata[LIBREFANG_ID_KEY]`.
pub const LIBREFANG_ID_KEY: &str = "librefang_id";

/// A [`VectorStore`] that talks to Supabase PostgREST RPC endpoints.
///
/// # ID semantics
///
/// Supabase uses auto-increment `BIGINT` row IDs internally. The trait-level
/// `id` passed to [`insert()`] is stashed in the document metadata under
/// [`LIBREFANG_ID_KEY`] for callers that need the original application ID.
/// [`search()`] returns the Supabase DB row ID (as a string), which is what
/// [`delete()`] expects.  Delete is **idempotent** — deleting a non-existent
/// ID logs a warning and returns `Ok(())`.
///
/// # Embedding precision
///
/// Embeddings are formatted with 8 fixed decimal places. Values below `1e-8`
/// are truncated to zero.  This is acceptable for standard embedding models
/// whose outputs are in the `[-1, 1]` range.
#[derive(Clone)]
pub struct SupabaseVectorStore {
    client: Client,
    base_url: String,
    api_key: String,
    /// Cosine distance threshold for search (lower = stricter).
    /// Default 0.5 means results with similarity ≥ 0.5 are returned.
    match_threshold: f32,
}

// Manual Debug impl to redact the API key from logs.
impl std::fmt::Debug for SupabaseVectorStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SupabaseVectorStore")
            .field("base_url", &self.base_url)
            .field("api_key", &"***REDACTED***")
            .field("match_threshold", &self.match_threshold)
            .finish()
    }
}

impl SupabaseVectorStore {
    /// Create a new Supabase vector store pointing at `base_url`.
    ///
    /// `base_url` should be the Supabase REST URL, e.g.
    /// `https://<project>.supabase.co/rest/v1`.  Trailing slashes are stripped.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            client,
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
        assert!(
            threshold.is_finite() && threshold >= 0.0,
            "match_threshold must be a non-negative finite number, got {threshold}"
        );
        self.match_threshold = threshold;
        self
    }

    /// Build the full URL for an RPC function.
    fn rpc_url(&self, function_name: &str) -> String {
        format!("{}/rpc/{}", self.base_url, function_name)
    }

    /// Build an authenticated POST request to a Supabase RPC endpoint.
    fn authed_post(&self, function_name: &str) -> reqwest::RequestBuilder {
        self.client
            .post(self.rpc_url(function_name))
            .header("apikey", &self.api_key)
            .header("Authorization", format!("Bearer {}", self.api_key))
    }
}

/// Convert an embedding slice to a PostgreSQL-compatible TEXT string.
///
/// Returns a string like `"[0.1,0.2,0.3]"` — a plain text representation
/// that the Supabase RPC casts to `vector` on the server side.
fn embedding_to_text(embedding: &[f32]) -> String {
    use std::fmt::Write;
    let mut buf = String::with_capacity(embedding.len() * 10 + 2);
    buf.push('[');
    for (i, v) in embedding.iter().enumerate() {
        if i > 0 {
            buf.push(',');
        }
        // Use fixed-point notation to avoid scientific notation (e.g. "1e-7")
        // which PostgreSQL's ruvector TEXT cast may reject.
        // Then trim trailing zeros to keep payloads compact
        // (384-dim: ~2KB instead of ~5KB).
        let formatted = format!("{:.8}", v);
        let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
        let _ = buf.write_str(trimmed);
    }
    buf.push(']');
    buf
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
            LIBREFANG_ID_KEY.to_string(),
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
            .authed_post("vector_insert")
            .header("Prefer", "return=representation")
            .json(&body)
            .send()
            .await
            .map_err(|e| LibreFangError::Internal(format!("Supabase vector insert: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp
                .text()
                .await
                .unwrap_or_else(|e| format!("<body unreadable: {e}>"));
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
            .authed_post("vector_search")
            .json(&body)
            .send()
            .await
            .map_err(|e| LibreFangError::Internal(format!("Supabase vector search: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp
                .text()
                .await
                .unwrap_or_else(|e| format!("<body unreadable: {e}>"));
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
                    score: (1.0 - item.distance).clamp(0.0, 1.0),
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
            .authed_post("vector_delete")
            .json(&body)
            .send()
            .await
            .map_err(|e| LibreFangError::Internal(format!("Supabase vector delete: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp
                .text()
                .await
                .unwrap_or_else(|e| format!("<body unreadable: {e}>"));
            return Err(LibreFangError::Internal(format!(
                "Supabase vector delete returned {status}: {text}"
            )));
        }

        // RPC returns true/false — log a warning if the row didn't exist.
        if let Ok(text) = resp.text().await {
            if text.trim() == "false" {
                tracing::warn!("Supabase vector delete: id {id} did not exist");
            }
        }
        Ok(())
    }

    async fn get_embeddings(&self, _ids: &[&str]) -> LibreFangResult<HashMap<String, Vec<f32>>> {
        // Honest failure: the Supabase RPC set does not include an embedding
        // retrieval endpoint. Returning Ok(empty) would silently degrade;
        // returning an error forces callers to handle the gap explicitly.
        Err(LibreFangError::Internal(
            "SupabaseVectorStore does not support get_embeddings — \
             no corresponding RPC exists. Use search() to find documents."
                .into(),
        ))
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
        let result = embedding_to_text(&[0.1, 0.2, 0.3]);
        // Trimmed fixed-point: compact output
        assert!(result.starts_with("[0.1"));
        assert!(result.contains(",0.2"));
        assert!(result.contains(",0.3"));
        // Trimmed: no patterns like "0.10000000" (8-digit zero padding)
        assert!(
            !result.ends_with("00]") && !result.contains("00,"),
            "Trailing zeros not trimmed: {result}"
        );
        // Verify round-trip
        let inner = &result[1..result.len() - 1];
        let vals: Vec<f32> = inner.split(',').map(|s| s.parse().unwrap()).collect();
        assert_eq!(vals.len(), 3);
        assert!((vals[0] - 0.1).abs() < 1e-6);
        assert!((vals[1] - 0.2).abs() < 1e-6);
        assert!((vals[2] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn test_embedding_to_text_compact() {
        // Verify trailing zero trimming produces compact output
        let result = embedding_to_text(&[0.5, 1.0, 0.0]);
        assert_eq!(result, "[0.5,1,0]");
    }

    #[test]
    fn test_embedding_to_text_empty() {
        assert_eq!(embedding_to_text(&[]), "[]");
    }

    #[test]
    fn test_score_inversion() {
        // Mirrors the formula in the VectorStore::search impl: .clamp(0.0, 1.0)
        let score_close = (1.0_f32 - 0.2).clamp(0.0, 1.0);
        assert!(
            (score_close - 0.8).abs() < 1e-6,
            "expected ~0.8, got {score_close}"
        );

        let score_far = (1.0_f32 - 1.5).clamp(0.0, 1.0);
        assert_eq!(score_far, 0.0);
    }

    #[test]
    fn test_backend_name() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "test-key");
        assert_eq!(store.backend_name(), "supabase");
    }

    #[test]
    fn test_debug_redacts_api_key() {
        let store =
            SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "super-secret-key-12345");
        let debug_output = format!("{:?}", store);
        assert!(
            debug_output.contains("REDACTED"),
            "Debug output must redact API key"
        );
        assert!(
            !debug_output.contains("super-secret"),
            "API key leaked in debug output: {debug_output}"
        );
    }

    #[tokio::test]
    async fn test_get_embeddings_returns_error() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "key");
        let result = store.get_embeddings(&["1", "2"]).await;
        assert!(
            result.is_err(),
            "get_embeddings must return Err, not silent empty"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not support"),
            "Error should explain the gap: {err_msg}"
        );
    }

    #[test]
    fn test_with_match_threshold() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "key")
            .with_match_threshold(0.3);
        assert!((store.match_threshold - 0.3).abs() < 1e-6);
    }

    #[test]
    #[should_panic(expected = "non-negative finite")]
    fn test_match_threshold_rejects_nan() {
        SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "key")
            .with_match_threshold(f32::NAN);
    }

    #[test]
    #[should_panic(expected = "non-negative finite")]
    fn test_match_threshold_rejects_negative() {
        SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "key")
            .with_match_threshold(-0.1);
    }

    #[test]
    fn test_default_match_threshold() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "key");
        assert!((store.match_threshold - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_embedding_to_text_precision() {
        // Verify f32 edge cases produce fixed-point notation, never scientific
        let result = embedding_to_text(&[0.0, 1.0, -1.0, 0.00001]);
        assert!(result.starts_with('['));
        assert!(result.ends_with(']'));
        assert_eq!(result.matches(',').count(), 3);
        // Must NOT contain 'e' or 'E' (scientific notation)
        assert!(
            !result.contains('e') && !result.contains('E'),
            "Scientific notation detected in: {result}"
        );
        // Verify actual values parse back
        let inner = &result[1..result.len() - 1];
        let vals: Vec<f32> = inner.split(',').map(|s| s.parse().unwrap()).collect();
        assert_eq!(vals.len(), 4);
        assert!((vals[0] - 0.0).abs() < 1e-6);
        assert!((vals[1] - 1.0).abs() < 1e-6);
        assert!((vals[2] - (-1.0)).abs() < 1e-6);
        assert!((vals[3] - 0.00001).abs() < 1e-4);
    }

    #[test]
    fn test_embedding_no_scientific_notation() {
        // Extreme values that would produce scientific notation with Display
        let result = embedding_to_text(&[1e-10, 1e-20, -1e-15, 1e10]);
        assert!(
            !result.contains('e') && !result.contains('E'),
            "Scientific notation detected in extreme values: {result}"
        );
    }

    #[test]
    fn test_score_clamped_to_unit_range() {
        // Negative distance (shouldn't happen, but defensive)
        let score_negative = (1.0_f32 - (-0.5)).clamp(0.0, 1.0);
        assert_eq!(score_negative, 1.0, "Score must cap at 1.0");

        // Distance > 2 (extreme outlier)
        let score_extreme = (1.0_f32 - 2.5).clamp(0.0, 1.0);
        assert_eq!(score_extreme, 0.0, "Score must floor at 0.0");
    }

    #[test]
    fn test_embedding_to_text_single() {
        let result = embedding_to_text(&[0.5]);
        let inner = &result[1..result.len() - 1];
        let val: f32 = inner.parse().unwrap();
        assert!((val - 0.5).abs() < 1e-6);
    }

    // ── DTO serialization / deserialization tests ─────────────────────
    // These test the actual request/response contracts with Supabase RPCs.

    #[test]
    fn test_insert_request_serialization() {
        let req = SupabaseInsertRequest {
            doc_content: "hello world",
            doc_embedding: embedding_to_text(&[0.1, 0.2]),
            doc_metadata: serde_json::json!({"source": "test", "librefang_id": "orig-42"}),
            doc_user_id: Some("00000000-0000-0000-0000-000000000001".to_string()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["doc_content"], "hello world");
        assert!(json["doc_embedding"].as_str().unwrap().starts_with("[0.1"));
        assert_eq!(json["doc_metadata"]["librefang_id"], "orig-42");
        assert_eq!(json["doc_user_id"], "00000000-0000-0000-0000-000000000001");
    }

    #[test]
    fn test_insert_request_skips_null_user_id() {
        let req = SupabaseInsertRequest {
            doc_content: "no user",
            doc_embedding: "[0.1]".to_string(),
            doc_metadata: serde_json::json!({}),
            doc_user_id: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        // skip_serializing_if = None means the key should be absent
        assert!(
            !json.as_object().unwrap().contains_key("doc_user_id"),
            "doc_user_id=None should be omitted from JSON"
        );
    }

    #[test]
    fn test_search_request_serialization() {
        let req = SupabaseSearchRequest {
            query_embedding: embedding_to_text(&[0.5, 0.6]),
            match_count: 10,
            match_threshold: 0.3,
            caller_user_id: Some("user-uuid".to_string()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["match_count"], 10);
        assert!((json["match_threshold"].as_f64().unwrap() - 0.3).abs() < 1e-6);
        assert_eq!(json["caller_user_id"], "user-uuid");
    }

    #[test]
    fn test_search_request_skips_null_caller() {
        let req = SupabaseSearchRequest {
            query_embedding: "[0.1]".to_string(),
            match_count: 5,
            match_threshold: 0.5,
            caller_user_id: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(!json.as_object().unwrap().contains_key("caller_user_id"));
    }

    #[test]
    fn test_search_response_deserialization() {
        let json = serde_json::json!([
            {"id": 42, "content": "doc one", "metadata": {"source": "a"}, "distance": 0.15},
            {"id": 99, "content": "doc two", "metadata": {}, "distance": 0.85}
        ]);
        let items: Vec<SupabaseSearchResponseItem> = serde_json::from_value(json).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, 42);
        assert_eq!(items[0].content, "doc one");
        assert!((items[0].distance - 0.15).abs() < 1e-6);
        assert_eq!(items[1].id, 99);
        assert!((items[1].distance - 0.85).abs() < 1e-6);
    }

    #[test]
    fn test_search_response_missing_metadata_defaults() {
        // PostgREST might return null metadata
        let json = serde_json::json!(
            {"id": 1, "content": "x", "distance": 0.0}
        );
        let item: SupabaseSearchResponseItem = serde_json::from_value(json).unwrap();
        assert!(item.metadata.is_null() || item.metadata == serde_json::Value::Null);
    }

    #[test]
    fn test_search_response_to_vector_search_result() {
        // End-to-end mapping: RPC response → VectorSearchResult
        let item = SupabaseSearchResponseItem {
            id: 42,
            content: "hello".to_string(),
            metadata: serde_json::json!({"librefang_id": "orig-7", "source": "test"}),
            distance: 0.2,
        };
        let metadata: HashMap<String, serde_json::Value> =
            serde_json::from_value(item.metadata.clone()).unwrap_or_default();
        let result = VectorSearchResult {
            id: item.id.to_string(),
            payload: item.content.clone(),
            score: (1.0 - item.distance).clamp(0.0, 1.0),
            metadata: metadata.clone(),
        };
        assert_eq!(result.id, "42"); // Supabase DB ID, not original
        assert_eq!(result.payload, "hello");
        assert!((result.score - 0.8).abs() < 1e-6);
        // Original ID recoverable from metadata via LIBREFANG_ID_KEY
        assert_eq!(metadata[LIBREFANG_ID_KEY].as_str().unwrap(), "orig-7");
    }

    #[test]
    fn test_delete_request_serialization() {
        let req = SupabaseDeleteRequest { doc_id: 42 };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["doc_id"], 42);
    }

    // ── Delete ID parsing tests ──────────────────────────────────────

    #[test]
    fn test_delete_id_parsing_valid() {
        let id = "12345";
        let parsed: Result<i64, _> = id.parse();
        assert_eq!(parsed.unwrap(), 12345);
    }

    #[test]
    fn test_delete_id_parsing_invalid_string() {
        let id = "not-a-number";
        let parsed: Result<i64, _> = id.parse();
        assert!(parsed.is_err());
    }

    #[test]
    fn test_delete_id_parsing_empty() {
        let id = "";
        let parsed: Result<i64, _> = id.parse();
        assert!(parsed.is_err());
    }

    #[test]
    fn test_delete_id_parsing_overflow() {
        // i64::MAX + 1 should fail
        let id = "9223372036854775808";
        let parsed: Result<i64, _> = id.parse();
        assert!(parsed.is_err());
    }

    // ── Metadata stashing tests ──────────────────────────────────────

    #[test]
    fn test_metadata_stashes_librefang_id() {
        let mut metadata = HashMap::new();
        metadata.insert("source".to_string(), serde_json::json!("test"));
        let original_id = "my-app-uuid-42";

        // Simulate what insert() does
        let mut meta_map = metadata.clone();
        meta_map.insert(
            LIBREFANG_ID_KEY.to_string(),
            serde_json::Value::String(original_id.to_string()),
        );

        assert_eq!(
            meta_map[LIBREFANG_ID_KEY].as_str().unwrap(),
            "my-app-uuid-42"
        );
        // Original metadata preserved
        assert_eq!(meta_map["source"].as_str().unwrap(), "test");
    }

    #[test]
    fn test_user_id_extraction_from_metadata() {
        let mut metadata = HashMap::new();
        metadata.insert("user_id".to_string(), serde_json::json!("uuid-123"));
        metadata.insert("other".to_string(), serde_json::json!("val"));

        // Simulate what insert() does
        let doc_user_id = metadata
            .get("user_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        assert_eq!(doc_user_id.as_deref(), Some("uuid-123"));
    }

    #[test]
    fn test_user_id_extraction_missing() {
        let metadata: HashMap<String, serde_json::Value> = HashMap::new();
        let doc_user_id = metadata
            .get("user_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        assert_eq!(doc_user_id, None);
    }

    #[test]
    fn test_user_id_extraction_non_string() {
        let mut metadata = HashMap::new();
        metadata.insert("user_id".to_string(), serde_json::json!(12345));
        // Numeric user_id should NOT be extracted (as_str returns None)
        let doc_user_id = metadata
            .get("user_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        assert_eq!(doc_user_id, None);
    }

    // ── Real-world dimension test ────────────────────────────────────

    #[test]
    fn test_embedding_384_dimensions() {
        // all-MiniLM-L6-v2 uses 384-dim embeddings
        let embedding: Vec<f32> = (0..384).map(|i| (i as f32 * 0.01).sin()).collect();
        let result = embedding_to_text(&embedding);

        // Structural checks
        assert!(result.starts_with('['));
        assert!(result.ends_with(']'));
        assert_eq!(result.matches(',').count(), 383); // 384 values = 383 commas

        // No scientific notation
        assert!(!result.contains('e') && !result.contains('E'));

        // Round-trip: parse back and verify
        let inner = &result[1..result.len() - 1];
        let vals: Vec<f32> = inner.split(',').map(|s| s.parse().unwrap()).collect();
        assert_eq!(vals.len(), 384);
        for (i, (&original, &parsed)) in embedding.iter().zip(vals.iter()).enumerate() {
            assert!(
                (original - parsed).abs() < 1e-5,
                "Mismatch at index {i}: original={original}, parsed={parsed}"
            );
        }
    }
}
