Exact tokens such as URLs, ids and proper nouns are findable again when an embedding provider is configured.
Recall switched to vector similarity alone and never consulted the BM25 index; memory search, `memory_semantic_search` and auto-recall now rank by combined cosine and BM25 scores (#8587, #8521) (@houko)
