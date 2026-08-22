import { describe, expect, it } from "vitest";
import { buildModelConfigPatch } from "./agentModelPatch";

// startModelEdit seeds an empty string when the backend sends `null`, so a
// draft that reflects "no user edit" carries empty strings for both knobs.
const draftOf = (
  over: Partial<{ provider: string; model: string; max_tokens: string; temperature: string }> = {},
) => ({
  provider: "anthropic",
  model: "claude-sonnet",
  max_tokens: "",
  temperature: "",
  ...over,
});

describe("buildModelConfigPatch", () => {
  it("provider-only change does NOT include max_tokens/temperature when the agent inherits them (#5917 regression)", () => {
    const persisted = { provider: "openai", model: "gpt-4o" };
    const draft = draftOf({ provider: "anthropic", model: "claude-sonnet" });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ provider: "anthropic", model: "claude-sonnet" });
    expect(patch).not.toHaveProperty("max_tokens");
    expect(patch).not.toHaveProperty("temperature");
  });

  it("leaves a pinned value alone when the user only switches provider", () => {
    const persisted = { provider: "openai", model: "gpt-4o", max_tokens: 4096, temperature: 0.7 };
    const draft = draftOf({
      provider: "anthropic",
      model: "claude-sonnet",
      max_tokens: "4096",
      temperature: "0.7",
    });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ provider: "anthropic", model: "claude-sonnet" });
  });

  it("includes a genuinely changed max_tokens", () => {
    const persisted = { provider: "openai", model: "gpt-4o", max_tokens: 8000, temperature: 0.5 };
    const draft = draftOf({
      provider: "openai",
      model: "gpt-4o",
      max_tokens: "12000",
      temperature: "0.5",
    });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ max_tokens: 12000 });
  });

  it("includes a genuinely changed temperature", () => {
    const persisted = { provider: "openai", model: "gpt-4o", max_tokens: 8000, temperature: 0.5 };
    const draft = draftOf({
      provider: "openai",
      model: "gpt-4o",
      max_tokens: "8000",
      temperature: "0.9",
    });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ temperature: 0.9 });
  });

  /**
   * The point of the tri-state: clearing a field is an edit, and it has to
   * reach the backend as `null`. With the old seeded-default comparison an
   * emptied field was indistinguishable from "no change", so an agent could
   * never be handed back to the per-model override from this form.
   */
  it("sends null when the user clears a pinned value", () => {
    const persisted = { provider: "openai", model: "gpt-4o", max_tokens: 8000, temperature: 0.5 };
    const draft = draftOf({ provider: "openai", model: "gpt-4o" });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ max_tokens: null, temperature: null });
  });

  it("sends a number when the user pins a previously inherited knob", () => {
    const persisted = { provider: "openai", model: "gpt-4o" };
    const draft = draftOf({ provider: "openai", model: "gpt-4o", temperature: "0.2" });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ temperature: 0.2 });
  });

  it("sends both provider and model together when either changes", () => {
    const persisted = { provider: "openai", model: "gpt-4o", max_tokens: 4096, temperature: 0.7 };
    const draft = draftOf({
      provider: "openai",
      model: "gpt-4o-mini",
      max_tokens: "4096",
      temperature: "0.7",
    });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ provider: "openai", model: "gpt-4o-mini" });
  });

  it("persists the global-default sentinel as a provider/model pair", () => {
    const persisted = { provider: "openrouter", model: "acme/current:free" };
    const draft = draftOf({ provider: "default", model: "default" });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ provider: "default", model: "default" });
  });

  it("returns no fields when nothing changed", () => {
    const persisted = { provider: "openai", model: "gpt-4o", max_tokens: 4096, temperature: 0.7 };
    const draft = draftOf({
      provider: "openai",
      model: "gpt-4o",
      max_tokens: "4096",
      temperature: "0.7",
    });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({});
  });

  it("returns null for invalid drafts", () => {
    const persisted = { provider: "openai", model: "gpt-4o" };
    expect(buildModelConfigPatch(draftOf({ model: "" }), persisted).patch).toBeNull();
    expect(buildModelConfigPatch(draftOf({ max_tokens: "0" }), persisted).patch).toBeNull();
    expect(buildModelConfigPatch(draftOf({ temperature: "3" }), persisted).patch).toBeNull();
    expect(buildModelConfigPatch(draftOf({ max_tokens: "abc" }), persisted).patch).toBeNull();
  });

  it("treats an entirely-undefined persisted model as all-inherit", () => {
    const draft = draftOf({ provider: "anthropic", model: "claude-sonnet" });

    const { patch } = buildModelConfigPatch(draft, undefined);

    expect(patch).toEqual({ provider: "anthropic", model: "claude-sonnet" });
  });

  // temperature === 0 is the `??` vs `||` tripwire: with `|| null` a persisted
  // explicit 0 collapses to the inherit state and these assertions go red.
  it("keeps an unchanged persisted temperature of 0 out of the patch", () => {
    const persisted = {
      provider: "anthropic",
      model: "claude-sonnet",
      max_tokens: 4096,
      temperature: 0,
    };
    const draft = draftOf({
      provider: "anthropic",
      model: "claude-sonnet",
      max_tokens: "4096",
      temperature: "0",
    });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({});
    expect(patch).not.toHaveProperty("temperature");
  });

  it("distinguishes an explicit 0 from the inherit state", () => {
    const persisted = { provider: "anthropic", model: "claude-sonnet" };
    const draft = draftOf({ provider: "anthropic", model: "claude-sonnet", temperature: "0" });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ temperature: 0 });
  });

  it("does not flag an unchanged pinned max_tokens as changed", () => {
    const persisted = {
      provider: "anthropic",
      model: "claude-sonnet",
      max_tokens: 4096,
      temperature: 0.7,
    };
    const draft = draftOf({
      provider: "anthropic",
      model: "claude-sonnet",
      max_tokens: "4096",
      temperature: "0.7",
    });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({});
    expect(patch).not.toHaveProperty("max_tokens");
  });
});
