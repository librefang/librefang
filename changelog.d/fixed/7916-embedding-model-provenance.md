A stored memory vector now records which embedding model produced it, and the daemon warns at boot when the model it is configured with is not the one the store was built with.
`memories.embedding` was a bare BLOB, and the only guard on the vector path was the length check inside `cosine_similarity` — which catches a change in dimensionality and nothing else, so switching between two models of the same size turned every pre-existing row's similarity into a meaningless number with no error anywhere.
Vectors from a different model are now left unscored during recall and withheld from the deduplicator rather than being trusted, and rows written before the stamp existed keep working exactly as they did.
(#7916) (@houko)
