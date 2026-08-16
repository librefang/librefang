// Known embedding model names per provider. Populates the embedding model
// `<select>` options in MemoryConfigDialog. Local providers
// (ollama / vllm / lmstudio) load arbitrary user-pulled models, so listed
// entries there are just common defaults — users with non-listed models pick
// the "Custom…" option to reveal a free-text input.
export const KNOWN_EMBEDDING_MODELS = {
  openai: ["text-embedding-3-small", "text-embedding-3-large", "text-embedding-ada-002"],
  openrouter: ["text-embedding-3-small", "openai/text-embedding-3-small"],
  mistral: ["mistral-embed"],
  together: ["BAAI/bge-large-en-v1.5"],
  fireworks: ["nomic-ai/nomic-embed-text-v1.5"],
  cohere: [
    "embed-multilingual-v3.0",
    "embed-english-v3.0",
    "embed-multilingual-light-v3.0",
    "embed-english-light-v3.0",
  ],
  ollama: ["nomic-embed-text", "mxbai-embed-large", "all-minilm"],
  vllm: ["nomic-embed-text", "BAAI/bge-large-en-v1.5"],
  lmstudio: ["nomic-embed-text", "text-embedding-nomic-embed-text-v1.5"],
} as const satisfies Record<string, readonly string[]>;

export type EmbeddingProvider = keyof typeof KNOWN_EMBEDDING_MODELS;

export const EMBEDDING_PROVIDERS = Object.keys(
  KNOWN_EMBEDDING_MODELS,
) as EmbeddingProvider[];

export function isKnownEmbeddingProvider(value: string): value is EmbeddingProvider {
  return Object.prototype.hasOwnProperty.call(KNOWN_EMBEDDING_MODELS, value);
}

// Display labels for the embedding-provider optgroups shown when the Provider
// field is "Auto-detect". Keys mirror KNOWN_EMBEDDING_MODELS.
export const EMBEDDING_PROVIDER_LABELS: Record<EmbeddingProvider, string> = {
  openai: "OpenAI",
  openrouter: "OpenRouter",
  mistral: "Mistral",
  together: "Together AI",
  fireworks: "Fireworks AI",
  cohere: "Cohere",
  ollama: "Ollama",
  vllm: "vLLM",
  lmstudio: "LM Studio",
};

export const EMBEDDING_PROVIDER_API_KEY_ENVS: Record<EmbeddingProvider, string> = {
  openai: "OPENAI_API_KEY",
  openrouter: "OPENROUTER_API_KEY",
  mistral: "MISTRAL_API_KEY",
  together: "TOGETHER_API_KEY",
  fireworks: "FIREWORKS_API_KEY",
  cohere: "COHERE_API_KEY",
  ollama: "",
  vllm: "",
  lmstudio: "",
};

// Sentinel value for the "Custom…" option in the model `<select>`s. Picking
// it switches the field into a free-text input rendered alongside the select;
// an existing stored value that isn't in the catalog is also treated as custom
// so the user can see and edit it. `__custom__` is reserved for this UI control
// and must not be added to a provider's model catalog.
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
