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

  it("keeps the model's own graded levels, in the declared order", () => {
    // `supported_efforts` is the graded declaration (`none` is a capability,
    // not a rung), and the offer mirrors `ReasoningEffortPolicy::offered()`:
    // graded rungs, with `none` in front when the model allows it.
    expect(
      resolveReasoningEfforts({ supported_efforts: ["minimal", "medium", "xhigh", "max"] }),
    ).toEqual(["none", "minimal", "medium", "xhigh", "max"]);
    // A model narrower than the canonical set narrows the menu.
    expect(resolveReasoningEfforts({ supported_efforts: ["low", "high"] })).toEqual([
      "none",
      "low",
      "high",
    ]);
  });

  it("drops blanks, duplicates and non-strings from a declared list", () => {
    expect(
      resolveReasoningEfforts({
        supported_efforts: ["low", "low", "", "   ", undefined as unknown as string],
      }),
    ).toEqual(["none", "low"]);
    // A list that cleans to nothing is the same as an absent one.
    expect(resolveReasoningEfforts({ supported_efforts: ["", " "] })).toEqual([
      ...CANONICAL_REASONING_EFFORTS,
    ]);
  });
});

// `offered_efforts` is the backend's prepared answer (#8616). When the payload
// carries it — even as `[]` — it wins: re-deriving from `supported_efforts`
// would second-guess the offer rules and could put `none` back on a model
// that cannot be switched off.
describe("resolveReasoningEfforts — the backend's own offer", () => {
  it("uses offered_efforts verbatim, declaration or not", () => {
    expect(
      resolveReasoningEfforts({
        supported_efforts: ["low", "high"],
        offered_efforts: ["low", "medium"],
      }),
    ).toEqual(["low", "medium"]);
    expect(
      resolveReasoningEfforts({
        supported_efforts: ["none", "low", "high"],
        offered_efforts: ["none", "low", "high"],
        off_allowed: true,
      }),
    ).toEqual(["none", "low", "high"]);
  });

  it("honours an empty offer instead of widening to the canonical set", () => {
    // An undeclared model on #8616: the backend says it offers no known
    // level, and guessing the canonical seven would offer exactly the levels
    // the policy says it cannot name.
    expect(resolveReasoningEfforts({ offered_efforts: [] })).toEqual([]);
    expect(
      resolveReasoningEfforts({ supported_efforts: ["low"], offered_efforts: [] }),
    ).toEqual([]);
  });

  it("still cleans blanks and duplicates out of the payload's list", () => {
    expect(
      resolveReasoningEfforts({ offered_efforts: ["low", "low", " ", "high"] }),
    ).toEqual(["low", "high"]);
  });
});

// "Not thinking" is a capability of its own: when the catalog says the model
// cannot be switched off (`off_allowed: false`, or `mandatory: true` — xAI,
// Kimi K3, Gemini 3 and friends), `none` is not offered, in a declared list or
// in the canonical fallback.
describe("resolveReasoningEfforts — off is not always available", () => {
  it("prepends none to a declared set that omits it, when off is allowed", () => {
    expect(
      resolveReasoningEfforts({ supported_efforts: ["low", "medium", "high"] }),
    ).toEqual(["none", "low", "medium", "high"]);
    expect(
      resolveReasoningEfforts({
        supported_efforts: ["low", "medium", "high"],
        off_allowed: true,
      }),
    ).toEqual(["none", "low", "medium", "high"]);
  });

  it("offers a mandatory model's declared set exactly, with no none", () => {
    expect(
      resolveReasoningEfforts({
        supported_efforts: ["low", "medium", "high"],
        mandatory: true,
      }),
    ).toEqual(["low", "medium", "high"]);
  });

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
  it("offers nothing on a catalog miss once the payload serves offered_efforts", () => {
    // A #8616 daemon answers for every model it knows; a miss means this one
    // was filtered out (custom model, `available: true`) or the query has not
    // resolved — not that the model declared the canonical seven.
    const models = [
      { id: "declared", provider: "openai", offered_efforts: ["low", "high"] },
    ];
    expect(modelReasoningEfforts(models, "not-in-the-list", "openai")).toEqual([]);
  });

  it("keeps the canonical fallback while the payload has no offered_efforts", () => {
    // Older daemon: nothing carries the field, so main compatibility still
    // needs the canonical list for a model the filtered catalog omits.
    const models = [{ id: "declared", provider: "openai" }];
    expect(modelReasoningEfforts(models, "not-in-the-list", "openai")).toEqual([
      "none",
      "minimal",
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
    ]);
  });

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
    expect(modelReasoningEfforts(models, "gpt-5", "anthropic")).toEqual(["none", "low", "max"]);
    expect(modelReasoningEfforts(models, "gpt-5", "openai")).toEqual([
      "none",
      "minimal",
      "low",
      "medium",
      "high",
    ]);
  });

  it("matches on id alone when the provider is unknown", () => {
    expect(modelReasoningEfforts(models, "gpt-5", "")).toEqual([
      "none",
      "minimal",
      "low",
      "medium",
      "high",
    ]);
  });

  it("accepts a `provider/model` spelled key", () => {
    expect(modelReasoningEfforts(models, "anthropic/gpt-5", "anthropic")).toEqual([
      "none",
      "low",
      "max",
    ]);
  });

  it("carries the capability flags through the catalog lookup", () => {
    // The model declares `none` but cannot be switched off — the menu must
    // not offer it.
    expect(modelReasoningEfforts(models, "grok-think", "xai")).toEqual(["low", "high"]);
  });

  it("carries the payload's offered_efforts through the catalog lookup", () => {
    const withOffer = [
      ...models,
      { provider: "xai", id: "grok-offer", offered_efforts: ["low", "medium"] },
    ];
    expect(modelReasoningEfforts(withOffer, "grok-offer", "xai")).toEqual(["low", "medium"]);
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
