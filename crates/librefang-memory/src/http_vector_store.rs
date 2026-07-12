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
use std::time::Duration;

/// Total request timeout for a single vector-store call.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Connection-establishment timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Maximum size (bytes) of a remote response body we will buffer before
/// deserializing. The 30s request timeout alone does not bound memory: a
/// misbehaving or hostile backend can stream a slow, unbounded body and OOM
/// the daemon. 64 MiB is far above any realistic batch of search hits /
/// embeddings while still capping the blast radius.
const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

/// Read a response body into memory, refusing to buffer more than
/// [`MAX_RESPONSE_BYTES`].
///
/// Mirrors the streaming cap in `librefang-runtime`'s `web_fetch`: an honest
/// `Content-Length` is rejected up front, and the chunk loop enforces the true
/// ceiling for chunked / mis-declared responses (where `content_length()` is
/// `None`). This is what makes the cap hold against a server that omits or
/// lies about the header.
async fn read_capped_body(mut resp: reqwest::Response) -> LibreFangResult<Vec<u8>> {
    if let Some(len) = resp.content_length() {
        if len > MAX_RESPONSE_BYTES {
            return Err(LibreFangError::Internal(format!(
                "HTTP vector response too large: {len} bytes (max {MAX_RESPONSE_BYTES})"
            )));
        }
    }
    let mut body: Vec<u8> = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() as u64 + chunk.len() as u64 > MAX_RESPONSE_BYTES {
                    return Err(LibreFangError::Internal(format!(
                        "HTTP vector response too large: exceeds max {MAX_RESPONSE_BYTES} bytes (server omitted or misreported Content-Length)"
                    )));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) => {
                return Err(LibreFangError::Internal(format!(
                    "HTTP vector response read failed: {e}"
                )));
            }
        }
    }
    Ok(body)
}

/// Read a (capped) error-response body as a lossy UTF-8 string for inclusion
/// in an error message. Bounded by [`read_capped_body`] so an error path
/// cannot be used to force an unbounded read either.
async fn read_capped_error_text(resp: reqwest::Response) -> String {
    read_capped_body(resp)
        .await
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

/// Build the reqwest client with bounded connect and total request time so a
/// backend that accepts the TCP connection but never responds cannot pin a
/// `spawn_blocking` pool thread forever (this store sits on the hot
/// recall/remember path via `block_on(vs.search())`).
fn build_client(request_timeout: Duration, connect_timeout: Duration) -> Client {
    Client::builder()
        .timeout(request_timeout)
        .connect_timeout(connect_timeout)
        .build()
        .unwrap_or_else(|_| Client::new())
}

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
            client: build_client(REQUEST_TIMEOUT, CONNECT_TIMEOUT),
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    /// Build the full URL for an endpoint.
    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
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
            let text = read_capped_error_text(resp).await;
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
            let text = read_capped_error_text(resp).await;
            return Err(LibreFangError::Internal(format!(
                "HTTP vector search returned {status}: {text}"
            )));
        }

        let body = read_capped_body(resp).await?;
        let items: Vec<SearchResponseItem> = serde_json::from_slice(&body)
            .map_err(|e| LibreFangError::Internal(format!("HTTP vector search parse: {e}")))?;

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
            let text = read_capped_error_text(resp).await;
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
            let text = read_capped_error_text(resp).await;
            return Err(LibreFangError::Internal(format!(
                "HTTP vector get_embeddings returned {status}: {text}"
            )));
        }

        let body = read_capped_body(resp).await?;
        let map: HashMap<String, Vec<f32>> = serde_json::from_slice(&body).map_err(|e| {
            LibreFangError::Internal(format!("HTTP vector get_embeddings parse: {e}"))
        })?;
        Ok(map)
    }

    fn backend_name(&self) -> &str {
        "http"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A backend that accepts the TCP connection but never sends a response
    /// must not pin the caller forever: the request timeout has to fire and
    /// surface an `Err`. Without a bounded client this call hangs
    /// indefinitely and the outer guard trips instead. A short injected
    /// timeout keeps the regression deterministic and fast while exercising
    /// the exact `build_client` path `new` uses in production.
    #[tokio::test]
    async fn test_hung_backend_returns_error_not_hang() {
        // Listener that accepts connections and then holds them open without
        // ever writing a response, simulating a stalled backend.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback listener");
        let addr = listener.local_addr().expect("local addr");
        let _accept = tokio::spawn(async move {
            // Keep every accepted socket alive so the client waits on a
            // response that never comes.
            let mut held = Vec::new();
            while let Ok((sock, _)) = listener.accept().await {
                held.push(sock);
            }
        });

        let store = HttpVectorStore {
            client: build_client(Duration::from_millis(300), Duration::from_millis(300)),
            base_url: format!("http://{addr}"),
        };
        // Bounded well above the client's 300ms request timeout; the outer
        // guard only trips if the client timeout regresses to none.
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            store.search(&[0.1, 0.2, 0.3], 5, None),
        )
        .await
        .expect("search must resolve within the request timeout, not hang");

        assert!(
            result.is_err(),
            "hung backend must surface an Err from the bounded client"
        );
    }

    /// A backend that declares a body far larger than `MAX_RESPONSE_BYTES`
    /// must be rejected on the `Content-Length` fast path rather than
    /// buffered into memory. Without the cap, `resp.json()` / `resp.text()`
    /// would read the whole body and let a hostile backend OOM the daemon.
    #[tokio::test]
    async fn search_rejects_oversized_response_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback listener");
        let addr = listener.local_addr().expect("local addr");
        let _srv = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                // Best-effort drain of the request so the client finishes
                // sending before we respond.
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                // Declare a body one byte past the cap. The Content-Length
                // fast path must reject before any body is read, so we never
                // actually send the body.
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    MAX_RESPONSE_BYTES + 1
                );
                let _ = sock.write_all(headers.as_bytes()).await;
                let _ = sock.flush().await;
                // Hold the socket briefly so the client can read the headers.
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });

        let store = HttpVectorStore {
            client: build_client(Duration::from_secs(5), Duration::from_secs(5)),
            base_url: format!("http://{addr}"),
        };
        let err = store
            .search(&[0.1, 0.2, 0.3], 5, None)
            .await
            .expect_err("oversized response must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("too large"),
            "expected a size-cap error, got: {msg}"
        );
    }
}
