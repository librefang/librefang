The `MemoryProvider` / `MemoryManager` plugin API is removed from `librefang-memory`.
It had no call site anywhere in the tree — the only reference outside its own module was the re-export in `lib.rs` — and it could not acquire one without regressing isolation guards that already ship: `prefetch(&self, query, session_id)` can carry one of the five parameters `auto_retrieve` takes, and it returns opaque text, on which `MemoryFilter.peer_id`, the cross-chat filter (#5227) and the cross-session filter (#7605) cannot run.
Wiring it would have traded three enforced guards for a seam nobody was using, so the 764 lines are gone and the retrieval seam that is wired and tenancy-aware — `VectorStore` — is the one being invested in instead.
(#7883) (@houko)
