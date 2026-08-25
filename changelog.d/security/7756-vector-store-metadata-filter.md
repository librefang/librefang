The metadata component of a caller's `MemoryFilter` is now re-applied when recall runs through an external vector-store backend.
The hydrate path already re-checked agent, scope, source, confidence floor and created-at bounds against the rows it fetched — precisely so tenant isolation never depends on an untrusted backend honouring the filter it was handed — but `metadata` was the one field the SQLite path enforced (as `json_extract(metadata, '$.key') = ?`) and the external path did not.
A deployment scoping recall by a metadata key therefore got that scope on the default backend and silently lost it on an attached one.
No caller sets `MemoryFilter.metadata` on a recall path today, so this closes the divergence before it can be reached rather than fixing a live leak.
(#7883) (@houko)
