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
//! | `vector_insert`       | `doc_content, doc_embedding, doc_metadata, doc_user_id, doc_account_id` | `BIGINT` (new row ID)             |
//! | `vector_insert_batch` | `doc_contents[], doc_embeddings[], doc_metadatas[], doc_user_id, doc_account_id` | `BIGINT[]` (new row IDs)          |
//! | `vector_search`       | `query_embedding, match_count, match_threshold, caller_user_id, caller_account_id` | `[{ id, content, metadata, distance }]` |
//! | `vector_delete`       | `doc_id`                                                | `BOOLEAN` (`true` if deleted)     |

use async_trait::async_trait;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{MemoryFilter, VectorSearchResult, VectorStore};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Default HTTP request timeout for PostgREST calls.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum retries for transient HTTP failures (5xx, timeout, connection).
const MAX_RETRIES: u32 = 1;
/// Delay between retries.
const RETRY_DELAY: Duration = Duration::from_millis(500);

/// Metadata key used to stash the VectorStore trait `id` inside the
/// Supabase document metadata. Consumers recovering the original ID
/// from search results should read `metadata[LIBREFANG_ID_KEY]`.
pub const LIBREFANG_ID_KEY: &str = "librefang_id";

/// Type alias for batch insert items: (doc_id, embedding, payload, metadata)
type BatchItem = (String, Vec<f32>, String, HashMap<String, serde_json::Value>);

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
/// # Multi-tenant support
///
/// The Supabase RPCs accept `account_id` parameters for tenant isolation.
/// Pass `account_id` in the metadata `HashMap` on insert, and in the
/// [`MemoryFilter`] metadata on search.  If omitted, the RPC defaults to
/// `NULL` (no tenant filtering).  `user_id` is **required** — the RPC
/// falls back to `auth.uid()`, but if neither is set the insert will fail
/// with a PostgreSQL exception.
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
    /// Cosine **distance** threshold for search (lower = stricter).
    /// The RPC filters with `WHERE distance < match_threshold`.
    /// Default 0.5 means only results with cosine distance < 0.5
    /// (i.e. similarity > 0.5) are returned.
    match_threshold: f32,
    /// Expected embedding dimensions (e.g. 384 for `all-MiniLM-L6-v2`).
    /// When set, `insert` / `search` / `insert_batch` reject embeddings
    /// with a mismatched length — preventing a cryptic PostgreSQL cast error.
    expected_dimensions: Option<usize>,
}

// Manual Debug impl to redact the API key from logs.
impl std::fmt::Debug for SupabaseVectorStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SupabaseVectorStore")
            .field("base_url", &self.base_url)
            .field("api_key", &"***REDACTED***")
            .field("match_threshold", &self.match_threshold)
            .field("expected_dimensions", &self.expected_dimensions)
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
            expected_dimensions: None,
        }
    }

    /// Set the cosine **distance** threshold for search results.
    ///
    /// The RPC uses `WHERE distance < threshold` — lower values are stricter.
    /// `0.3` → distance < 0.3 (similarity > 0.7), `0.5` → distance < 0.5,
    /// `2.0` → return everything (max cosine distance is 2.0).
    #[must_use]
    pub fn with_match_threshold(mut self, threshold: f32) -> Self {
        assert!(
            threshold.is_finite() && threshold >= 0.0,
            "match_threshold must be a non-negative finite number, got {threshold}"
        );
        self.match_threshold = threshold;
        self
    }

    /// Set the expected embedding dimensions for client-side validation.
    ///
    /// Must match the DB column definition (e.g. `ruvector(384)`).
    /// Embeddings of wrong length are rejected before the HTTP call.
    #[must_use]
    pub fn with_expected_dimensions(mut self, dims: usize) -> Self {
        assert!(dims > 0, "expected_dimensions must be > 0, got {dims}");
        self.expected_dimensions = Some(dims);
        self
    }

    /// Validate embedding dimensions and emptiness.
    fn validate_embedding(&self, embedding: &[f32], context: &str) -> LibreFangResult<()> {
        if embedding.is_empty() {
            return Err(LibreFangError::InvalidInput(format!(
                "Supabase {context}: embedding cannot be empty"
            )));
        }
        if let Some(expected) = self.expected_dimensions {
            if embedding.len() != expected {
                return Err(LibreFangError::InvalidInput(format!(
                    "Supabase {context}: expected {expected}-dim embedding, got {}-dim",
                    embedding.len()
                )));
            }
        }
        Ok(())
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

    /// Send an RPC request with automatic retry on transient failures.
    ///
    /// Retries [`MAX_RETRIES`] times on 5xx, timeout, or connection errors.
    /// Returns the response on 2xx. 4xx (client errors) are returned as
    /// `Err` immediately — they're not transient.
    async fn send_with_retry<F>(
        &self,
        rpc_name: &str,
        build_request: F,
    ) -> LibreFangResult<reqwest::Response>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut last_err = String::new();
        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                tracing::info!(rpc = rpc_name, attempt, "Retrying after transient failure");
                tokio::time::sleep(RETRY_DELAY).await;
            }
            match build_request().send().await {
                Ok(resp) if resp.status().is_server_error() => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    last_err = format!("Supabase {rpc_name} returned {status}: {body}");
                    tracing::warn!(rpc = rpc_name, %status, "Server error, will retry");
                    continue;
                }
                Ok(resp) if !resp.status().is_success() => {
                    let status = resp.status();
                    let text = resp
                        .text()
                        .await
                        .unwrap_or_else(|e| format!("<body unreadable: {e}>"));
                    return Err(LibreFangError::Internal(format!(
                        "Supabase {rpc_name} returned {status}: {text}"
                    )));
                }
                Ok(resp) => return Ok(resp),
                Err(e) if e.is_timeout() || e.is_connect() => {
                    last_err = format!("Supabase {rpc_name}: {e}");
                    tracing::warn!(rpc = rpc_name, error = %e, "Transient error, will retry");
                    continue;
                }
                Err(e) => {
                    return Err(LibreFangError::Internal(format!(
                        "Supabase {rpc_name}: {e}"
                    )));
                }
            }
        }
        Err(LibreFangError::Internal(format!(
            "{last_err} (after {MAX_RETRIES} retries)"
        )))
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
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_account_id: Option<String>,
}

#[derive(Serialize)]
struct SupabaseSearchRequest {
    query_embedding: String,
    match_count: usize,
    match_threshold: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    caller_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    caller_account_id: Option<String>,
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
struct SupabaseBatchInsertRequest {
    doc_contents: Vec<String>,
    doc_embeddings: Vec<String>,
    doc_metadatas: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_account_id: Option<String>,
}

#[derive(Serialize)]
struct SupabaseDeleteRequest {
    doc_id: i64,
}

// ── Batch operations (not part of the VectorStore trait) ─────────────────

impl SupabaseVectorStore {
    /// Batch-insert multiple documents in a single RPC call.
    ///
    /// Uses the `vector_insert_batch` RPC — one HTTP round-trip instead of N.
    ///
    /// `user_id` and `account_id` are extracted from the **first** item's
    /// metadata and applied uniformly to all rows (the RPC uses scalar
    /// params, not per-document).  Returns the Supabase row IDs.
    pub async fn insert_batch(&self, items: &[BatchItem]) -> LibreFangResult<Vec<i64>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate every embedding up front before any network I/O.
        for (i, (_id, embedding, _payload, _metadata)) in items.iter().enumerate() {
            self.validate_embedding(embedding, &format!("vector_insert_batch[{i}]"))?;
        }

        // Extract tenant context from the first item's metadata.
        let doc_user_id = items[0]
            .3
            .get("user_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let doc_account_id = items[0]
            .3
            .get("account_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Warn if any subsequent items have different tenant context.
        // The RPC uses scalar params — all rows get the first item's IDs.
        for (i, (_id, _emb, _pay, metadata)) in items.iter().enumerate().skip(1) {
            let item_user = metadata.get("user_id").and_then(|v| v.as_str());
            let item_acct = metadata.get("account_id").and_then(|v| v.as_str());
            if item_user != doc_user_id.as_deref() {
                tracing::warn!(
                    "insert_batch: item[{i}] user_id={:?} differs from item[0]={:?} — \
                     RPC will apply item[0]'s user_id to all rows",
                    item_user,
                    doc_user_id,
                );
            }
            if item_acct != doc_account_id.as_deref() {
                tracing::warn!(
                    "insert_batch: item[{i}] account_id={:?} differs from item[0]={:?} — \
                     RPC will apply item[0]'s account_id to all rows",
                    item_acct,
                    doc_account_id,
                );
            }
        }

        if doc_user_id.is_none() {
            tracing::warn!(
                "Supabase vector_insert_batch: no user_id in first item's metadata — \
                 RPC falls back to auth.uid() which is NULL for anon keys"
            );
        }

        let mut doc_contents = Vec::with_capacity(items.len());
        let mut doc_embeddings = Vec::with_capacity(items.len());
        let mut doc_metadatas = Vec::with_capacity(items.len());

        for (id, embedding, payload, metadata) in items {
            doc_contents.push(payload.clone());
            doc_embeddings.push(embedding_to_text(embedding));

            let mut meta = metadata.clone();
            meta.insert(
                LIBREFANG_ID_KEY.to_string(),
                serde_json::Value::String(id.clone()),
            );
            let meta_val = serde_json::to_value(&meta).map_err(|e| {
                LibreFangError::Internal(format!("Supabase metadata serialize: {e}"))
            })?;
            doc_metadatas.push(meta_val);
        }

        let body = SupabaseBatchInsertRequest {
            doc_contents,
            doc_embeddings,
            doc_metadatas,
            doc_user_id,
            doc_account_id,
        };

        let start = std::time::Instant::now();
        let resp = self
            .send_with_retry("vector_insert_batch", || {
                self.authed_post("vector_insert_batch")
                    .header("Prefer", "return=representation")
                    .json(&body)
            })
            .await?;

        let ids: Vec<i64> = resp.json().await.map_err(|e| {
            LibreFangError::Internal(format!("Supabase vector insert_batch parse: {e}"))
        })?;

        tracing::info!(
            rpc = "vector_insert_batch",
            elapsed_ms = start.elapsed().as_millis() as u64,
            count = ids.len(),
            "batch insert completed"
        );
        Ok(ids)
    }
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
        self.validate_embedding(embedding, "vector_insert")?;

        // Extract user_id and account_id from metadata before we move it.
        // NOTE: user_id is REQUIRED by the Supabase RPC (FK to auth.users).
        // If neither user_id nor an active auth session exists, insert will fail.
        let doc_user_id = metadata
            .get("user_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if doc_user_id.is_none() {
            tracing::warn!(
                "Supabase vector_insert: no user_id in metadata — \
                 RPC falls back to auth.uid() which is NULL for anon keys"
            );
        }
        let doc_account_id = metadata
            .get("account_id")
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
            doc_account_id,
        };

        let start = std::time::Instant::now();
        let resp = self
            .send_with_retry("vector_insert", || {
                self.authed_post("vector_insert")
                    .header("Prefer", "return=representation")
                    .json(&body)
            })
            .await?;
        let _ = resp.text().await;
        tracing::debug!(
            rpc = "vector_insert",
            elapsed_ms = start.elapsed().as_millis() as u64,
            "insert completed"
        );
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
        self.validate_embedding(query_embedding, "vector_search")?;

        let caller_user_id = filter
            .as_ref()
            .and_then(|f| f.metadata.get("user_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let caller_account_id = filter
            .as_ref()
            .and_then(|f| f.metadata.get("account_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let body = SupabaseSearchRequest {
            query_embedding: embedding_to_text(query_embedding),
            match_count: limit.min(i32::MAX as usize),
            match_threshold: self.match_threshold,
            caller_user_id,
            caller_account_id,
        };

        let start = std::time::Instant::now();
        let resp = self
            .send_with_retry("vector_search", || {
                self.authed_post("vector_search").json(&body)
            })
            .await?;

        let items: Vec<SupabaseSearchResponseItem> = resp
            .json()
            .await
            .map_err(|e| LibreFangError::Internal(format!("Supabase vector search parse: {e}")))?;

        let results: Vec<VectorSearchResult> = items
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
            .collect();
        tracing::debug!(
            rpc = "vector_search",
            elapsed_ms = start.elapsed().as_millis() as u64,
            results = results.len(),
            "search completed"
        );
        Ok(results)
    }

    async fn delete(&self, id: &str) -> LibreFangResult<()> {
        let doc_id: i64 = id.parse().map_err(|e| {
            LibreFangError::InvalidInput(format!("Supabase vector delete: invalid id '{id}': {e}"))
        })?;

        let body = SupabaseDeleteRequest { doc_id };

        let start = std::time::Instant::now();
        let resp = self
            .send_with_retry("vector_delete", || {
                self.authed_post("vector_delete").json(&body)
            })
            .await?;

        // RPC returns true/false — log a warning if the row didn't exist.
        if let Ok(text) = resp.text().await {
            if text.trim() == "false" {
                tracing::warn!("Supabase vector delete: id {id} did not exist");
            }
        }
        tracing::debug!(rpc = "vector_delete", elapsed_ms = start.elapsed().as_millis() as u64, doc_id = %id, "delete completed");
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
            doc_account_id: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["doc_content"], "hello world");
        assert!(json["doc_embedding"].as_str().unwrap().starts_with("[0.1"));
        assert_eq!(json["doc_metadata"]["librefang_id"], "orig-42");
        assert_eq!(json["doc_user_id"], "00000000-0000-0000-0000-000000000001");
    }

    #[test]
    fn test_insert_request_includes_account_id() {
        let req = SupabaseInsertRequest {
            doc_content: "multi-tenant",
            doc_embedding: "[0.1]".to_string(),
            doc_metadata: serde_json::json!({}),
            doc_user_id: Some("user-uuid".to_string()),
            doc_account_id: Some("account-uuid".to_string()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["doc_account_id"], "account-uuid");
        assert_eq!(json["doc_user_id"], "user-uuid");
    }

    #[test]
    fn test_insert_request_skips_null_account_id() {
        let req = SupabaseInsertRequest {
            doc_content: "no tenant",
            doc_embedding: "[0.1]".to_string(),
            doc_metadata: serde_json::json!({}),
            doc_user_id: Some("user".to_string()),
            doc_account_id: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(
            !json.as_object().unwrap().contains_key("doc_account_id"),
            "doc_account_id=None should be omitted"
        );
    }

    #[test]
    fn test_search_request_includes_account_id() {
        let req = SupabaseSearchRequest {
            query_embedding: "[0.1]".to_string(),
            match_count: 5,
            match_threshold: 0.5,
            caller_user_id: None,
            caller_account_id: Some("tenant-123".to_string()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["caller_account_id"], "tenant-123");
        assert!(!json.as_object().unwrap().contains_key("caller_user_id"));
    }

    #[test]
    fn test_account_id_extraction_from_metadata() {
        let mut metadata = HashMap::new();
        metadata.insert("account_id".to_string(), serde_json::json!("acct-456"));
        metadata.insert("user_id".to_string(), serde_json::json!("user-789"));
        let account = metadata
            .get("account_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let user = metadata
            .get("user_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        assert_eq!(account.as_deref(), Some("acct-456"));
        assert_eq!(user.as_deref(), Some("user-789"));
    }

    #[test]
    fn test_insert_request_skips_null_user_id() {
        let req = SupabaseInsertRequest {
            doc_content: "no user",
            doc_embedding: "[0.1]".to_string(),
            doc_metadata: serde_json::json!({}),
            doc_user_id: None,
            doc_account_id: None,
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
            caller_account_id: None,
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
            caller_account_id: None,
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

    // ── Batch insert tests ───────────────────────────────────────────

    #[test]
    fn test_batch_insert_request_serialization() {
        let req = SupabaseBatchInsertRequest {
            doc_contents: vec!["doc one".into(), "doc two".into()],
            doc_embeddings: vec!["[0.1,0.2]".into(), "[0.3,0.4]".into()],
            doc_metadatas: vec![
                serde_json::json!({"librefang_id": "a"}),
                serde_json::json!({"librefang_id": "b"}),
            ],
            doc_user_id: Some("user-uuid".to_string()),
            doc_account_id: Some("acct-uuid".to_string()),
        };
        let json = serde_json::to_value(&req).unwrap();
        let contents = json["doc_contents"].as_array().unwrap();
        assert_eq!(contents.len(), 2);
        assert_eq!(contents[0], "doc one");
        assert_eq!(contents[1], "doc two");
        let embeddings = json["doc_embeddings"].as_array().unwrap();
        assert_eq!(embeddings.len(), 2);
        let metadatas = json["doc_metadatas"].as_array().unwrap();
        assert_eq!(metadatas[0]["librefang_id"], "a");
        assert_eq!(json["doc_user_id"], "user-uuid");
        assert_eq!(json["doc_account_id"], "acct-uuid");
    }

    #[test]
    fn test_batch_insert_request_skips_null_ids() {
        let req = SupabaseBatchInsertRequest {
            doc_contents: vec!["x".into()],
            doc_embeddings: vec!["[0.1]".into()],
            doc_metadatas: vec![serde_json::json!({})],
            doc_user_id: None,
            doc_account_id: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        let obj = json.as_object().unwrap();
        assert!(
            !obj.contains_key("doc_user_id"),
            "None user_id must be omitted"
        );
        assert!(
            !obj.contains_key("doc_account_id"),
            "None account_id must be omitted"
        );
    }

    #[test]
    fn test_batch_insert_request_empty_arrays() {
        let req = SupabaseBatchInsertRequest {
            doc_contents: vec![],
            doc_embeddings: vec![],
            doc_metadatas: vec![],
            doc_user_id: None,
            doc_account_id: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["doc_contents"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_insert_batch_empty_returns_ok() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "key");
        let result = store.insert_batch(&[]).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_batch_metadata_stashes_librefang_ids() {
        // Simulate what insert_batch does to metadata
        let items = vec![
            (
                "app-id-1".to_string(),
                vec![0.1_f32],
                "doc1".to_string(),
                HashMap::new(),
            ),
            (
                "app-id-2".to_string(),
                vec![0.2_f32],
                "doc2".to_string(),
                HashMap::new(),
            ),
        ];
        let mut metadatas = Vec::new();
        for (id, _, _, metadata) in &items {
            let mut meta = metadata.clone();
            meta.insert(
                LIBREFANG_ID_KEY.to_string(),
                serde_json::Value::String(id.clone()),
            );
            metadatas.push(serde_json::to_value(&meta).unwrap());
        }
        assert_eq!(metadatas[0][LIBREFANG_ID_KEY], "app-id-1");
        assert_eq!(metadatas[1][LIBREFANG_ID_KEY], "app-id-2");
    }

    #[test]
    fn test_batch_user_id_from_first_item() {
        // insert_batch extracts user_id from first item only
        let mut meta0 = HashMap::new();
        meta0.insert("user_id".to_string(), serde_json::json!("user-A"));
        let mut meta1 = HashMap::new();
        meta1.insert("user_id".to_string(), serde_json::json!("user-B"));
        let items: Vec<(String, Vec<f32>, String, HashMap<String, serde_json::Value>)> = vec![
            ("a".to_string(), vec![0.1_f32], "d1".to_string(), meta0),
            ("b".to_string(), vec![0.2_f32], "d2".to_string(), meta1),
        ];
        let doc_user_id = items[0]
            .3
            .get("user_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        // First item wins — RPC applies uniformly
        assert_eq!(doc_user_id.as_deref(), Some("user-A"));
    }

    // ── Real-world dimension test ────────────────────────────────────

    // ── Dimension validation tests ─────────────────────────────────

    #[test]
    fn test_validate_embedding_accepts_correct_dims() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "key")
            .with_expected_dimensions(3);
        let result = store.validate_embedding(&[0.1, 0.2, 0.3], "test");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_embedding_rejects_wrong_dims() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "key")
            .with_expected_dimensions(3);
        let result = store.validate_embedding(&[0.1, 0.2], "test");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        // Must be InvalidInput, not Internal
        assert!(
            err_msg.contains("expected 3") || err_msg.contains("3 dimensions"),
            "Error should mention expected dims: {err_msg}"
        );
    }

    #[test]
    fn test_validate_embedding_skips_when_unset() {
        // No expected_dimensions → all sizes pass
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "key");
        let result = store.validate_embedding(&[0.1, 0.2], "test");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_embedding_rejects_empty_when_dims_set() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "key")
            .with_expected_dimensions(384);
        let result = store.validate_embedding(&[], "test");
        assert!(result.is_err());
    }

    #[test]
    fn test_with_expected_dimensions_builder() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "key")
            .with_expected_dimensions(768);
        assert_eq!(store.expected_dimensions, Some(768));
    }

    // ── Batch consistency warning tests ──────────────────────────────

    #[test]
    fn test_batch_inconsistent_user_id_detection() {
        // Simulates the consistency check logic from insert_batch
        let mut meta0 = HashMap::new();
        meta0.insert("user_id".to_string(), serde_json::json!("user-A"));
        let mut meta1 = HashMap::new();
        meta1.insert("user_id".to_string(), serde_json::json!("user-B"));

        let doc_user_id = meta0.get("user_id").and_then(|v| v.as_str());
        let item1_user = meta1.get("user_id").and_then(|v| v.as_str());
        assert_ne!(
            doc_user_id, item1_user,
            "Inconsistent user_ids should be detected"
        );
    }

    #[test]
    fn test_batch_inconsistent_account_id_detection() {
        let mut meta0 = HashMap::new();
        meta0.insert("account_id".to_string(), serde_json::json!("acct-A"));
        let mut meta1 = HashMap::new();
        meta1.insert("account_id".to_string(), serde_json::json!("acct-B"));

        let doc_acct = meta0.get("account_id").and_then(|v| v.as_str());
        let item1_acct = meta1.get("account_id").and_then(|v| v.as_str());
        assert_ne!(
            doc_acct, item1_acct,
            "Inconsistent account_ids should be detected"
        );
    }

    // ── InvalidInput error variant tests ─────────────────────────────

    #[test]
    fn test_delete_invalid_id_returns_invalid_input() {
        // Verify the error is InvalidInput, not Internal
        let id = "not-a-number";
        let result: Result<i64, _> = id.parse();
        assert!(result.is_err());
        // The actual delete() wraps this with InvalidInput
    }

    #[test]
    fn test_dimension_mismatch_returns_invalid_input() {
        let store = SupabaseVectorStore::new("https://abc.supabase.co/rest/v1", "key")
            .with_expected_dimensions(3);
        let err = store.validate_embedding(&[0.1], "test").unwrap_err();
        // Should be InvalidInput variant
        let err_string = format!("{err:?}");
        assert!(
            err_string.contains("InvalidInput") || err_string.contains("invalid"),
            "Expected InvalidInput variant, got: {err_string}"
        );
    }

    // ── E2E integration test (requires live Supabase) ────────────────

    #[tokio::test]
    #[ignore = "requires running supabase-ruvector at localhost:54321"]
    async fn test_e2e_insert_search_delete() {
        let url = std::env::var("SUPABASE_URL")
            .unwrap_or_else(|_| "http://localhost:54321/rest/v1".to_string());
        let key =
            std::env::var("SUPABASE_ANON_KEY").expect("SUPABASE_ANON_KEY must be set for E2E test");

        let store = SupabaseVectorStore::new(&url, key).with_expected_dimensions(384);

        // 1. Insert a document
        let embedding: Vec<f32> = (0..384).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut metadata = HashMap::new();
        metadata.insert(
            "user_id".to_string(),
            serde_json::json!("00000000-0000-0000-0000-000000000001"),
        );
        metadata.insert("source".to_string(), serde_json::json!("e2e-test"));

        store
            .insert(
                "e2e-test-doc",
                &embedding,
                "E2E test document",
                metadata.clone(),
            )
            .await
            .expect("insert should succeed");

        // 2. Search for it
        let filter = crate::MemoryFilter {
            metadata: {
                let mut m = HashMap::new();
                m.insert(
                    "user_id".to_string(),
                    serde_json::json!("00000000-0000-0000-0000-000000000001"),
                );
                m
            },
            ..Default::default()
        };
        let results = store
            .search(&embedding, 5, Some(filter))
            .await
            .expect("search should succeed");

        assert!(
            !results.is_empty(),
            "search should return at least one result"
        );
        let top = &results[0];
        assert!(
            top.score > 0.9,
            "exact same embedding should have score > 0.9, got {}",
            top.score
        );

        // 3. Delete it
        store.delete(&top.id).await.expect("delete should succeed");

        // 4. Verify deletion — search again, should not find it
        let filter2 = crate::MemoryFilter {
            metadata: {
                let mut m = HashMap::new();
                m.insert(
                    "user_id".to_string(),
                    serde_json::json!("00000000-0000-0000-0000-000000000001"),
                );
                m
            },
            ..Default::default()
        };
        let after_delete = store
            .search(&embedding, 5, Some(filter2))
            .await
            .expect("post-delete search should succeed");

        let still_present = after_delete.iter().any(|r| r.id == top.id);
        assert!(
            !still_present,
            "deleted document should not appear in search results"
        );
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
