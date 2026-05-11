// Known embedding model names per provider. Populates the embedding model
// `<select>` options in MemoryConfigDialog. Local providers
// (ollama / vllm / lmstudio) load arbitrary user-pulled models, so listed
// entries there are just common defaults — users with non-listed models pick
// the "Custom…" option to reveal a free-text input.
export const KNOWN_EMBEDDING_MODELS: Record<string, string[]> = {
  openai: ["text-embedding-3-small", "text-embedding-3-large", "text-embedding-ada-002"],
  gemini: ["text-embedding-004", "embedding-001"],
  minimax: ["embo-01"],
  ollama: ["nomic-embed-text", "mxbai-embed-large", "all-minilm"],
  vllm: ["nomic-embed-text", "BAAI/bge-large-en-v1.5"],
  lmstudio: ["nomic-embed-text", "text-embedding-nomic-embed-text-v1.5"],
};

// Display labels for the embedding-provider optgroups shown when the Provider
// field is "Auto-detect". Keys mirror KNOWN_EMBEDDING_MODELS.
export const EMBEDDING_PROVIDER_LABELS: Record<string, string> = {
  openai: "OpenAI",
  gemini: "Gemini",
  minimax: "MiniMax",
  ollama: "Ollama",
  vllm: "vLLM",
  lmstudio: "LM Studio",
};

// Sentinel value for the "Custom…" option in the model `<select>`s. Picking
// it switches the field into a free-text input rendered alongside the select;
// an existing stored value that isn't in the catalog is also treated as custom
// so the user can see and edit it.
export const CUSTOM_OPTION = "__custom__";

// Cap KV table cell rendering — full value still available via the `title`
// attribute. KV blobs can be multi-KB JSON; clamp both the visible cell and
// the hover preview so a single row doesn't bloat the DOM.
export const KV_VALUE_TRUNCATE = 200;
export const KV_TITLE_TRUNCATE = 2000;

// Memory page URL search-param schema. Both keys are optional — absent
// `agent` means the "All agents" aggregate scope; absent `tab` falls back to
// "records".
export type MemoryTab = "records" | "kv" | "dreams" | "health";

export const MEMORY_TABS: readonly MemoryTab[] = ["records", "kv", "dreams", "health"] as const;
