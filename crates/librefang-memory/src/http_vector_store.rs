//! HTTP-backed vector store implementation.
//!
//! Delegates all vector operations to a remote service over HTTP/JSON,
//! allowing LibreFang to use external vector databases (Qdrant, Weaviate,
//! a custom microservice, etc.) without linking their native clients.
//!
//! ## Expected API contract
//!
//! | Method | Path               | Body (JSON)                                     | Response (JSON)                |
//! |--------|--------------------|------------------------------------------------|--------------------------------|
//! | POST   | `/insert`          | `{ id, embedding, payload, metadata }`         | `{}`                           |
//! | POST   | `/search`          | `{ query_embedding, limit, filter? }`          | `[{ id, payload, score, metadata }]` |
//! | DELETE | `/delete`          | `{ id }`                                       | `{}`                           |
//! | POST   | `/get_embeddings`  | `{ ids }`                                      | `{ "<id>": [f32, ...], ... }` |

use async_trait::async_trait;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{MemoryFilter, VectorSearchResult, VectorStore};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A [`VectorStore`] that talks to a remote HTTP service.
#[derive(Clone)]
pub struct HttpVectorStore {
    client: Client,
    base_url: String,
}

impl HttpVectorStore {
    /// Create a new HTTP vector store pointing at `base_url`.
    ///
    /// `base_url` should include the scheme and host, e.g.
    /// `http://localhost:6333/collections/memories`.  No trailing slash.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    /// Build the full URL for an endpoint.
    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    fn expected_account_id(filter: Option<&MemoryFilter>) -> Option<&str> {
        filter
            .and_then(|filter| filter.metadata.get("account_id"))
            .and_then(|value| value.as_str())
    }

    fn validate_search_item(
        item: &SearchResponseItem,
        expected_account_id: Option<&str>,
    ) -> LibreFangResult<()> {
        if let Some(expected_account_id) = expected_account_id {
            let actual = item
                .metadata
                .get("account_id")
                .and_then(|value| value.as_str());
            if actual != Some(expected_account_id) {
                return Err(LibreFangError::Internal(format!(
                    "HTTP vector search returned mismatched account_id for id '{}'",
                    item.id
                )));
            }
        }
        Ok(())
    }

    fn validate_embedding_ids(
        ids: &[&str],
        map: &HashMap<String, Vec<f32>>,
    ) -> LibreFangResult<()> {
        let allowed: std::collections::HashSet<&str> = ids.iter().copied().collect();
        if let Some(unexpected) = map.keys().find(|id| !allowed.contains(id.as_str())) {
            return Err(LibreFangError::Internal(format!(
                "HTTP vector get_embeddings returned unexpected id '{}'",
                unexpected
            )));
        }
        Ok(())
    }
}

// ── Request / response DTOs ──────────────────────────────────────────────

#[derive(Serialize)]
struct InsertRequest<'a> {
    id: &'a str,
    embedding: &'a [f32],
    payload: &'a str,
    metadata: &'a HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct SearchRequest<'a> {
    query_embedding: &'a [f32],
    limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<&'a MemoryFilter>,
}

#[derive(Deserialize)]
struct SearchResponseItem {
    id: String,
    payload: String,
    score: f32,
    #[serde(default)]
    metadata: HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct DeleteRequest<'a> {
    id: &'a str,
}

#[derive(Serialize)]
struct GetEmbeddingsRequest<'a> {
    ids: &'a [&'a str],
}

// ── VectorStore implementation ───────────────────────────────────────────

#[async_trait]
impl VectorStore for HttpVectorStore {
    async fn insert(
        &self,
        id: &str,
        embedding: &[f32],
        payload: &str,
        metadata: HashMap<String, serde_json::Value>,
    ) -> LibreFangResult<()> {
        let body = InsertRequest {
            id,
            embedding,
            payload,
            metadata: &metadata,
        };
        let resp = self
            .client
            .post(self.url("insert"))
            .json(&body)
            .send()
            .await
            .map_err(|e| LibreFangError::Internal(format!("HTTP vector insert: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LibreFangError::Internal(format!(
                "HTTP vector insert returned {status}: {text}"
            )));
        }
        Ok(())
    }

    async fn search(
        &self,
        query_embedding: &[f32],
        limit: usize,
        filter: Option<MemoryFilter>,
    ) -> LibreFangResult<Vec<VectorSearchResult>> {
        let body = SearchRequest {
            query_embedding,
            limit,
            filter: filter.as_ref(),
        };
        let resp = self
            .client
            .post(self.url("search"))
            .json(&body)
            .send()
            .await
            .map_err(|e| LibreFangError::Internal(format!("HTTP vector search: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LibreFangError::Internal(format!(
                "HTTP vector search returned {status}: {text}"
            )));
        }

        let items: Vec<SearchResponseItem> = resp
            .json()
            .await
            .map_err(|e| LibreFangError::Internal(format!("HTTP vector search parse: {e}")))?;
        let expected_account_id = Self::expected_account_id(body.filter);

        for item in &items {
            Self::validate_search_item(item, expected_account_id)?;
        }

        Ok(items
            .into_iter()
            .map(|i| VectorSearchResult {
                id: i.id,
                payload: i.payload,
                score: i.score,
                metadata: i.metadata,
            })
            .collect())
    }

    async fn delete(&self, id: &str) -> LibreFangResult<()> {
        let body = DeleteRequest { id };
        let resp = self
            .client
            .delete(self.url("delete"))
            .json(&body)
            .send()
            .await
            .map_err(|e| LibreFangError::Internal(format!("HTTP vector delete: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LibreFangError::Internal(format!(
                "HTTP vector delete returned {status}: {text}"
            )));
        }
        Ok(())
    }

    async fn get_embeddings(&self, ids: &[&str]) -> LibreFangResult<HashMap<String, Vec<f32>>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let body = GetEmbeddingsRequest { ids };
        let resp = self
            .client
            .post(self.url("get_embeddings"))
            .json(&body)
            .send()
            .await
            .map_err(|e| LibreFangError::Internal(format!("HTTP vector get_embeddings: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LibreFangError::Internal(format!(
                "HTTP vector get_embeddings returned {status}: {text}"
            )));
        }

        let map: HashMap<String, Vec<f32>> = resp.json().await.map_err(|e| {
            LibreFangError::Internal(format!("HTTP vector get_embeddings parse: {e}"))
        })?;
        Self::validate_embedding_ids(ids, &map)?;
        Ok(map)
    }

    fn backend_name(&self) -> &str {
        "http"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};
    use librefang_types::memory::MemoryFilter;
    use tokio::net::TcpListener;

    async fn spawn_test_server(router: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, router).await.expect("serve");
        });
        format!("http://{}", addr)
    }

    #[test]
    fn test_url_building() {
        let store = HttpVectorStore::new("http://localhost:6333/v1");
        assert_eq!(store.url("search"), "http://localhost:6333/v1/search");
        assert_eq!(store.url("/insert"), "http://localhost:6333/v1/insert");
    }

    #[test]
    fn test_trailing_slash_stripped() {
        let store = HttpVectorStore::new("http://localhost:6333/v1/");
        assert_eq!(store.url("search"), "http://localhost:6333/v1/search");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn search_rejects_cross_tenant_response_items() {
        let base = spawn_test_server(Router::new().route(
            "/v1/search",
            post(|| async {
                Json(serde_json::json!([{
                    "id": "mem-1",
                    "payload": "leaked",
                    "score": 0.9,
                    "metadata": {"account_id": "tenant-b"}
                }]))
            }),
        ))
        .await;

        let store = HttpVectorStore::new(format!("{base}/v1"));
        let mut filter = MemoryFilter::default();
        filter
            .metadata
            .insert("account_id".to_string(), serde_json::json!("tenant-a"));

        let result = store.search(&[0.1, 0.2], 5, Some(filter)).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("mismatched account_id"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_embeddings_rejects_unrequested_ids() {
        let base = spawn_test_server(Router::new().route(
            "/v1/get_embeddings",
            post(|| async {
                Json(serde_json::json!({
                    "mem-1": [0.1, 0.2],
                    "mem-2": [0.3, 0.4]
                }))
            }),
        ))
        .await;

        let store = HttpVectorStore::new(format!("{base}/v1"));
        let result = store.get_embeddings(&["mem-1"]).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unexpected id"));
    }
}
