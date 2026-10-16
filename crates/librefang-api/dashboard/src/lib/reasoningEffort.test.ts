import { describe, expect, it } from "vitest";
import {
  CANONICAL_REASONING_EFFORTS,
  modelReasoningEfforts,
  reasoningEffortLabel,
  resolveReasoningEfforts,
} from "./reasoningEffort";

// The dashboard offers the levels a model declares (`supported_efforts` on the
// catalog entry, alongside `default_effort`, `off_allowed` and `mandatory`).
// An older daemon — or a provider that declares nothing — has no such list, and
// an empty menu would read as "reasoning does not exist" rather than "nobody
// said". The canonical set is the fallback contract.

describe("resolveReasoningEfforts", () => {
  it("falls back to the canonical levels when the model declares none", () => {
    expect(resolveReasoningEfforts(undefined)).toEqual([...CANONICAL_REASONING_EFFORTS]);
    expect(resolveReasoningEfforts(null)).toEqual([...CANONICAL_REASONING_EFFORTS]);
    expect(resolveReasoningEfforts({})).toEqual([...CANONICAL_REASONING_EFFORTS]);
    // An empty list is "nobody said", not "no levels allowed" — the same
    // absent-vs-empty reading the rest of the dashboard gives optional lists.
    expect(resolveReasoningEfforts({ supported_efforts: [] })).toEqual([
      ...CANONICAL_REASONING_EFFORTS,
    ]);
  });

  it("keeps the model's own levels, in the declared order", () => {
    expect(
      resolveReasoningEfforts({ supported_efforts: ["minimal", "medium", "xhigh", "max"] }),
    ).toEqual(["minimal", "medium", "xhigh", "max"]);
    // A model narrower than the canonical set narrows the menu.
    expect(resolveReasoningEfforts({ supported_efforts: ["low", "high"] })).toEqual([
      "low",
      "high",
    ]);
  });

  it("drops blanks, duplicates and non-strings from a declared list", () => {
    expect(
      resolveReasoningEfforts({
        supported_efforts: ["low", "low", "", "   ", undefined as unknown as string],
      }),
    ).toEqual(["low"]);
    // A list that cleans to nothing is the same as an absent one.
    expect(resolveReasoningEfforts({ supported_efforts: ["", " "] })).toEqual([
      ...CANONICAL_REASONING_EFFORTS,
    ]);
  });
});

// "Not thinking" is a capability of its own: when the catalog says the model
// cannot be switched off (`off_allowed: false`, or `mandatory: true` — xAI,
// Kimi K3, Gemini 3 and friends), `none` is not offered, in a declared list or
// in the canonical fallback.
describe("resolveReasoningEfforts — off is not always available", () => {
  it("drops none from a declared list when off is disallowed", () => {
    expect(
      resolveReasoningEfforts({
        supported_efforts: ["none", "low", "high"],
        off_allowed: false,
      }),
    ).toEqual(["low", "high"]);
    expect(
      resolveReasoningEfforts({
        supported_efforts: ["none", "low", "high"],
        mandatory: true,
      }),
    ).toEqual(["low", "high"]);
  });

  it("drops none from the canonical fallback when off is disallowed", () => {
    const withoutOff = CANONICAL_REASONING_EFFORTS.filter((level) => level !== "none");
    expect(resolveReasoningEfforts({ off_allowed: false })).toEqual(withoutOff);
    expect(resolveReasoningEfforts({ mandatory: true })).toEqual(withoutOff);
  });

  it("keeps none when the catalog allows it or says nothing", () => {
    expect(
      resolveReasoningEfforts({ supported_efforts: ["none", "max"], off_allowed: true }),
    ).toEqual(["none", "max"]);
    // Absent flags are not a statement — the canonical contract stands.
    expect(resolveReasoningEfforts({})).toContain("none");
  });
});

describe("modelReasoningEfforts", () => {
  const models = [
    {
      provider: "openai",
      id: "gpt-5",
      supported_efforts: ["minimal", "low", "medium", "high"],
    },
    { provider: "anthropic", id: "gpt-5", supported_efforts: ["low", "max"] },
    { provider: "xai", id: "grok-think", supported_efforts: ["none", "low", "high"], off_allowed: false },
    { provider: "openai", id: "gpt-4o" },
  ];

  it("matches on provider and id together", () => {
    expect(modelReasoningEfforts(models, "gpt-5", "anthropic")).toEqual(["low", "max"]);
    expect(modelReasoningEfforts(models, "gpt-5", "openai")).toEqual([
      "minimal",
      "low",
      "medium",
      "high",
    ]);
  });

  it("matches on id alone when the provider is unknown", () => {
    expect(modelReasoningEfforts(models, "gpt-5", "")).toEqual([
      "minimal",
      "low",
      "medium",
      "high",
    ]);
  });

  it("accepts a `provider/model` spelled key", () => {
    expect(modelReasoningEfforts(models, "anthropic/gpt-5", "anthropic")).toEqual(["low", "max"]);
  });

  it("carries the capability flags through the catalog lookup", () => {
    // The model declares `none` but cannot be switched off — the menu must
    // not offer it.
    expect(modelReasoningEfforts(models, "grok-think", "xai")).toEqual(["low", "high"]);
  });

  it("falls back to the canonical set when the model is not in the catalog", () => {
    expect(modelReasoningEfforts(models, "unknown-model", "openai")).toEqual([
      ...CANONICAL_REASONING_EFFORTS,
    ]);
    // No model chosen yet — canonical, not empty.
    expect(modelReasoningEfforts(models, "", "")).toEqual([...CANONICAL_REASONING_EFFORTS]);
    // The model is known but declares no levels — canonical, not empty.
    expect(modelReasoningEfforts(models, "gpt-4o", "openai")).toEqual([
      ...CANONICAL_REASONING_EFFORTS,
    ]);
  });
});

describe("reasoningEffortLabel", () => {
  it("labels the canonical levels with their technical names", () => {
    expect(reasoningEffortLabel("none")).toBe("None");
    expect(reasoningEffortLabel("minimal")).toBe("Minimal");
    expect(reasoningEffortLabel("low")).toBe("Low");
    expect(reasoningEffortLabel("medium")).toBe("Medium");
    expect(reasoningEffortLabel("high")).toBe("High");
    expect(reasoningEffortLabel("xhigh")).toBe("xHigh");
    expect(reasoningEffortLabel("max")).toBe("Max");
  });

  it("is case-insensitive on the way in", () => {
    expect(reasoningEffortLabel("XHigh")).toBe("xHigh");
    expect(reasoningEffortLabel(" MAX ")).toBe("Max");
  });

  it("passes an unrecognized level through, capitalized, rather than hiding it", () => {
    // The kernel's vocabulary can grow ahead of the dashboard; a stored value
    // the map does not know must still render as itself.
    expect(reasoningEffortLabel("ultra")).toBe("Ultra");
  });
});
