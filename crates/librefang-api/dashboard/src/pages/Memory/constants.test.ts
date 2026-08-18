import { describe, expect, it } from "vitest";
import {
  CUSTOM_OPTION,
  EMBEDDING_PROVIDER_LABELS,
  EMBEDDING_PROVIDERS,
  KNOWN_EMBEDDING_MODELS,
  isKnownEmbeddingProvider,
} from "./constants";

describe("Memory embedding catalog", () => {
  it("tracks the providers supported by runtime embedding detection", () => {
    expect(EMBEDDING_PROVIDERS).toEqual([
      "openai",
      "openrouter",
      "mistral",
      "together",
      "fireworks",
      "cohere",
      "ollama",
      "vllm",
      "lmstudio",
    ]);
    expect(Object.keys(EMBEDDING_PROVIDER_LABELS)).toEqual(EMBEDDING_PROVIDERS);
  });

  it("includes backend defaults and excludes stale MiniMax suggestions", () => {
    expect(KNOWN_EMBEDDING_MODELS.openrouter).toContain("text-embedding-3-small");
    expect(KNOWN_EMBEDDING_MODELS.mistral).toContain("mistral-embed");
    expect(KNOWN_EMBEDDING_MODELS.cohere).toContain("embed-english-v3.0");
    expect(isKnownEmbeddingProvider("minimax")).toBe(false);
  });

  it("keeps the custom-option sentinel out of every provider catalog", () => {
    expect(Object.values(KNOWN_EMBEDDING_MODELS).flat()).not.toContain(CUSTOM_OPTION);
  });
});
