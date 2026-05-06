//! [`kernel_handle::WikiAccess`] — durable markdown knowledge vault
//! (issue #3329). Returns `KernelOpError::unavailable("wiki_*")` per
//! method when `[memory_wiki] enabled = false` (the kernel didn't wire
//! a vault at boot); otherwise serialises the vault's owned `WikiPage`
//! / `SearchHit` / `WikiWriteOutcome` into the JSON shape the trait
//! contract documents.

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
            Err(librefang_memory_wiki::WikiError::HandEditConflict { topic }) => Err(
                KernelOpError::Internal(format!(
                    "wiki page `{topic}` was edited externally; re-read the file or pass force=true"
                )),
            ),
            Err(librefang_memory_wiki::WikiError::InvalidTopic { topic, reason }) => Err(
                KernelOpError::InvalidInput(format!("wiki_write topic `{topic}`: {reason}")),
            ),
            Err(librefang_memory_wiki::WikiError::BodyTooLarge { topic, size, cap }) => Err(
                KernelOpError::InvalidInput(format!(
                    "wiki_write body for `{topic}` is {size} bytes; exceeds the {cap}-byte cap"
                )),
            ),
            Err(err) => Err(KernelOpError::Internal(format!("Wiki write failed: {err}"))),
        }
    }
}
