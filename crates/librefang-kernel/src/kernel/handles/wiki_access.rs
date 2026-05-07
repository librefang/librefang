//! [`kernel_handle::WikiAccess`] — durable markdown knowledge vault
//! (#3329). Routes to the boot-time `wiki_vault: Option<Arc<WikiVault>>`
//! field on `LibreFangKernel`. When `[memory_wiki]` is disabled the
//! field is `None` and every method short-circuits to
//! `KernelOpError::unavailable(...)`.
//!
//! Restored after the kernel/mod split (#4713) silently dropped the
//! method bodies — the empty `impl WikiAccess for LibreFangKernel {}`
//! that landed in phase 1 made the trait defaults take over, which
//! return `unavailable("wiki_*")` regardless of the configured vault,
//! disabling the wiki feature for any deployment with `[memory_wiki]`
//! enabled. Bodies are ported verbatim from
//! `crates/librefang-kernel/src/kernel/mod.rs` pre-split.

use librefang_runtime::kernel_handle;

use super::super::LibreFangKernel;

impl kernel_handle::WikiAccess for LibreFangKernel {
    fn wiki_get(&self, topic: &str) -> Result<serde_json::Value, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        let vault = self
            .wiki_vault
            .as_ref()
            .ok_or_else(|| KernelOpError::unavailable("wiki_get"))?;
        match vault.get(topic) {
            Ok(page) => serde_json::to_value(&page)
                .map_err(|e| KernelOpError::Internal(format!("Wiki get serialize: {e}"))),
            Err(librefang_memory_wiki::WikiError::NotFound(_)) => Err(KernelOpError::Internal(
                format!("wiki topic `{topic}` not found"),
            )),
            Err(err) => Err(KernelOpError::Internal(format!("Wiki get failed: {err}"))),
        }
    }

    fn wiki_search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<serde_json::Value, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        let vault = self
            .wiki_vault
            .as_ref()
            .ok_or_else(|| KernelOpError::unavailable("wiki_search"))?;
        let hits = vault
            .search(query, limit)
            .map_err(|e| KernelOpError::Internal(format!("Wiki search failed: {e}")))?;
        serde_json::to_value(&hits)
            .map_err(|e| KernelOpError::Internal(format!("Wiki search serialize: {e}")))
    }

    fn wiki_write(
        &self,
        topic: &str,
        body: &str,
        provenance: serde_json::Value,
        force: bool,
    ) -> Result<serde_json::Value, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        let vault = self
            .wiki_vault
            .as_ref()
            .ok_or_else(|| KernelOpError::unavailable("wiki_write"))?;
        let prov: librefang_memory_wiki::ProvenanceEntry = serde_json::from_value(provenance)
            .map_err(|e| {
                KernelOpError::InvalidInput(format!(
                    "wiki_write `provenance` must be {{agent, [session], [channel], [turn], at}}: {e}"
                ))
            })?;
        match vault.write(topic, body, prov, force) {
            Ok(outcome) => serde_json::to_value(&outcome)
                .map_err(|e| KernelOpError::Internal(format!("Wiki write serialize: {e}"))),
            Err(librefang_memory_wiki::WikiError::HandEditConflict { topic }) => {
                Err(KernelOpError::Internal(format!(
                    "wiki page `{topic}` was edited externally; re-read the file or pass force=true"
                )))
            }
            Err(librefang_memory_wiki::WikiError::InvalidTopic { topic, reason }) => Err(
                KernelOpError::InvalidInput(format!("wiki_write topic `{topic}`: {reason}")),
            ),
            Err(librefang_memory_wiki::WikiError::BodyTooLarge { topic, size, cap }) => {
                Err(KernelOpError::InvalidInput(format!(
                    "wiki_write body for `{topic}` is {size} bytes; exceeds the {cap}-byte cap"
                )))
            }
            Err(err) => Err(KernelOpError::Internal(format!("Wiki write failed: {err}"))),
        }
    }
}
