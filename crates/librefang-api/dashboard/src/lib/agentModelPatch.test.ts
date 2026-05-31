import { describe, expect, it } from "vitest";
import {
  buildModelConfigPatch,
  MODEL_MAX_TOKENS_DEFAULT,
  MODEL_TEMPERATURE_DEFAULT,
} from "./agentModelPatch";

// startModelEdit seeds the draft with these defaults when the backend omits
// the field, so a draft that reflects "no user edit" carries the default string.
const seededDraft = (over: Partial<{ provider: string; model: string; max_tokens: string; temperature: string }> = {}) => ({
  provider: "anthropic",
  model: "claude-sonnet",
  max_tokens: String(MODEL_MAX_TOKENS_DEFAULT),
  temperature: String(MODEL_TEMPERATURE_DEFAULT),
  ...over,
});

describe("buildModelConfigPatch", () => {
  it("provider-only change does NOT include max_tokens/temperature when backend omitted them (#5917 regression)", () => {
    // Backend omits the optional fields; user only switches the provider/model.
    const persisted = { provider: "openai", model: "gpt-4o" };
    const draft = seededDraft({ provider: "anthropic", model: "claude-sonnet" });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ provider: "anthropic", model: "claude-sonnet" });
    expect(patch).not.toHaveProperty("max_tokens");
    expect(patch).not.toHaveProperty("temperature");
  });

  it("does not include max_tokens/temperature when the persisted value already equals the seeded default", () => {
    const persisted = { provider: "openai", model: "gpt-4o", max_tokens: 4096, temperature: 0.7 };
    const draft = seededDraft({ provider: "anthropic", model: "claude-sonnet" });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ provider: "anthropic", model: "claude-sonnet" });
  });

  it("includes a genuinely changed max_tokens", () => {
    const persisted = { provider: "openai", model: "gpt-4o", max_tokens: 8000, temperature: 0.5 };
    const draft = seededDraft({ provider: "openai", model: "gpt-4o", max_tokens: "12000", temperature: "0.5" });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ max_tokens: 12000 });
  });

  it("includes a genuinely changed temperature", () => {
    const persisted = { provider: "openai", model: "gpt-4o", max_tokens: 8000, temperature: 0.5 };
    const draft = seededDraft({ provider: "openai", model: "gpt-4o", max_tokens: "8000", temperature: "0.9" });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ temperature: 0.9 });
  });

  it("sends both provider and model together when either changes", () => {
    const persisted = { provider: "openai", model: "gpt-4o", max_tokens: 4096, temperature: 0.7 };
    const draft = seededDraft({ provider: "openai", model: "gpt-4o-mini" });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ provider: "openai", model: "gpt-4o-mini" });
  });

  it("returns no fields when nothing changed", () => {
    const persisted = { provider: "openai", model: "gpt-4o", max_tokens: 4096, temperature: 0.7 };
    const draft = seededDraft({ provider: "openai", model: "gpt-4o" });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({});
  });

  it("returns null for invalid drafts", () => {
    const persisted = { provider: "openai", model: "gpt-4o" };
    expect(buildModelConfigPatch(seededDraft({ model: "" }), persisted).patch).toBeNull();
    expect(buildModelConfigPatch(seededDraft({ max_tokens: "0" }), persisted).patch).toBeNull();
    expect(buildModelConfigPatch(seededDraft({ temperature: "3" }), persisted).patch).toBeNull();
    expect(buildModelConfigPatch(seededDraft({ max_tokens: "abc" }), persisted).patch).toBeNull();
  });

  it("treats an entirely-undefined persisted model as all-default baseline", () => {
    // No model object at all: provider/model are the only real changes.
    const draft = seededDraft({ provider: "anthropic", model: "claude-sonnet" });

    const { patch } = buildModelConfigPatch(draft, undefined);

    expect(patch).toEqual({ provider: "anthropic", model: "claude-sonnet" });
  });

  // temperature === 0 is the `??` vs `||` tripwire: with `|| MODEL_TEMPERATURE_DEFAULT`
  // a persisted/explicit 0 collapses to 0.7 and these two assertions go red.
  it("keeps an unchanged persisted temperature of 0 out of the patch", () => {
    const persisted = { provider: "anthropic", model: "claude-sonnet", max_tokens: 4096, temperature: 0 };
    const draft = seededDraft({ provider: "anthropic", model: "claude-sonnet", temperature: "0" });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({});
    expect(patch).not.toHaveProperty("temperature");
  });

  it("sends temperature: 0 when the user lowers an omitted (default 0.7) temperature to an explicit 0", () => {
    // Backend omitted temperature, so the baseline is the default 0.7; "0" is a real change.
    const persisted = { provider: "anthropic", model: "claude-sonnet" };
    const draft = seededDraft({ provider: "anthropic", model: "claude-sonnet", temperature: "0" });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ temperature: 0 });
  });

  it("does not flag max_tokens as changed when the persisted value equals the default 4096", () => {
    const persisted = { provider: "anthropic", model: "claude-sonnet", max_tokens: 4096, temperature: 0.7 };
    const draft = seededDraft({ provider: "anthropic", model: "claude-sonnet" });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({});
    expect(patch).not.toHaveProperty("max_tokens");
  });
});
