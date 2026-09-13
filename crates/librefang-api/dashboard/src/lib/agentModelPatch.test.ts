import { describe, expect, it } from "vitest";
import {
  buildModelConfigPatch,
  emptyModelNumerics,
  seedModelNumerics,
  MODEL_NUMERIC_FIELDS,
  type ModelDraft,
} from "./agentModelPatch";

// startModelEdit seeds an empty string when the backend sends `null`, so a
// draft that reflects "no user edit" carries empty strings for every knob.
const draftOf = (over: Partial<ModelDraft> = {}): ModelDraft => ({
  provider: "anthropic",
  model: "claude-sonnet",
  ...emptyModelNumerics(),
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

  it("sends only model when the provider is unchanged", () => {
    const persisted = { provider: "openai", model: "gpt-4o", max_tokens: 4096, temperature: 0.7 };
    const draft = draftOf({
      provider: "openai",
      model: "gpt-4o-mini",
      max_tokens: "4096",
      temperature: "0.7",
    });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ model: "gpt-4o-mini" });
  });

  it("sends model with a provider change because the API applies them together", () => {
    const persisted = { provider: "openai", model: "gpt-4o", max_tokens: 4096, temperature: 0.7 };
    // The pinned values are carried into the draft so this isolates what the
    // test names — the model riding along with a provider change. Leaving them
    // empty would make the draft say "clear both as well", which is a
    // different edit and now correctly produces two more keys.
    const draft = draftOf({
      provider: "openrouter",
      model: "gpt-4o",
      max_tokens: "4096",
      temperature: "0.7",
    });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({ provider: "openrouter", model: "gpt-4o" });
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
    expect(buildModelConfigPatch(draftOf({ max_tokens: "4096abc" }), persisted).patch).toBeNull();
    expect(buildModelConfigPatch(draftOf({ temperature: "0.7xyz" }), persisted).patch).toBeNull();
  });

  it("normalizes persisted provider and model whitespace before comparing", () => {
    const persisted = { provider: " openai ", model: " gpt-4o " };
    const draft = draftOf({ provider: "openai", model: "gpt-4o" });

    expect(buildModelConfigPatch(draft, persisted).patch).toEqual({});
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

  // The five parameters the drawer could not reach before. The route has
  // accepted them as tri-state all along (`PatchAgentConfigRequest` in
  // routes/agents/config.rs); only the form was missing.
  it("sends every sampling parameter the operator pinned", () => {
    const persisted = { provider: "openai", model: "gpt-4o" };
    const draft = draftOf({
      provider: "openai",
      model: "gpt-4o",
      top_p: "0.9",
      frequency_penalty: "-0.5",
      presence_penalty: "1.25",
      context_window: "200000",
      max_output_tokens: "8192",
    });

    const { patch } = buildModelConfigPatch(draft, persisted);

    expect(patch).toEqual({
      top_p: 0.9,
      frequency_penalty: -0.5,
      presence_penalty: 1.25,
      context_window: 200000,
      max_output_tokens: 8192,
    });
  });

  it("keeps the sign on a negative penalty", () => {
    // The penalties are the only fields with a negative half, and losing the
    // sign silently persists the opposite of what was asked for.
    const { patch } = buildModelConfigPatch(
      draftOf({ frequency_penalty: "-1.5", presence_penalty: "-0.25" }),
      { provider: "anthropic", model: "claude-sonnet" },
    );
    expect(patch).toMatchObject({ frequency_penalty: -1.5, presence_penalty: -0.25 });
  });

  it("clears a pinned parameter when its box is emptied", () => {
    const { patch } = buildModelConfigPatch(draftOf({ top_p: "" }), {
      provider: "anthropic",
      model: "claude-sonnet",
      top_p: 0.5,
    });
    // `null`, not absent: absent would leave the pinned 0.5 in place, and the
    // empty box is a deliberate "hand this back to the model's setting".
    expect(patch).toEqual({ top_p: null });
  });

  it("refuses the whole draft when one field is out of range", () => {
    // Partially applying it would save the fields that parsed and silently
    // drop the one that did not.
    for (const [field, bad] of [
      ["top_p", "1.5"],
      ["frequency_penalty", "-3"],
      ["presence_penalty", "9"],
      ["temperature", "2.5"],
      ["context_window", "0"],
      ["max_output_tokens", "0"],
      ["max_tokens", "0"],
    ] as const) {
      const { patch } = buildModelConfigPatch(draftOf({ [field]: bad }), undefined);
      expect(patch, `${field}=${bad} must invalidate the draft`).toBeNull();
    }
  });

  it("keeps a clear made in the same edit as a provider switch", () => {
    // A provider switch does NOT reset these server-side, whatever the client
    // used to assume: `set_agent_model` (kernel/agent_state.rs) clears only
    // `api_key_env` and `base_url`, and `patch_agent_config` writes model and
    // provider *before* the sampling fields, so a `null` sent alongside a
    // provider change is applied rather than overwritten.
    // Dropping it lost the only edit it could ever have described — an
    // untouched pinned field seeds to its own value and never reaches here.
    const { patch } = buildModelConfigPatch(
      draftOf({ provider: "anthropic", model: "claude-sonnet" }),
      { provider: "openai", model: "gpt-4o", top_p: 0.8, presence_penalty: 0.2 },
    );
    expect(patch).toEqual({
      provider: "anthropic",
      model: "claude-sonnet",
      top_p: null,
      presence_penalty: null,
    });
  });

  it("still omits an inherited parameter the operator never touched across a provider switch", () => {
    // The companion to the test above: `null` reaches the patch because it is
    // an edit, not because provider switching sweeps every field into it.
    const { patch } = buildModelConfigPatch(
      draftOf({ provider: "anthropic", model: "claude-sonnet" }),
      { provider: "openai", model: "gpt-4o" },
    );
    expect(patch).toEqual({ provider: "anthropic", model: "claude-sonnet" });
  });

  it("refuses a fractional value for a whole-number field instead of truncating it", () => {
    // `parseInt` accepted these and silently stored the truncation, so an
    // operator who typed 4096.7 got 4096 saved with no indication.
    for (const [field, bad] of [
      ["max_tokens", "4096.7"],
      ["context_window", "1.5"],
      ["max_output_tokens", "8192.01"],
    ] as const) {
      const { patch } = buildModelConfigPatch(draftOf({ [field]: bad }), undefined);
      expect(patch, `${field}=${bad} must invalidate the draft`).toBeNull();
    }
  });

  it("reads exponent notation as the number it denotes", () => {
    // `parseInt("1e5", 10)` stops at the `e` and yields 1, so this used to
    // store 1 for an operator who typed 1e5 into a field that accepts it.
    const { patch } = buildModelConfigPatch(draftOf({ context_window: "1e5" }), undefined);
    expect(patch).toMatchObject({ context_window: 100000 });
  });
});

describe("seedModelNumerics", () => {
  it("turns an explicit null on the wire into the inherit state, not a compiled default", () => {
    // Seeding a number here is what used to make an untouched field look like
    // a deliberate choice and pin it on the next save (#5917).
    // The nulls are spelled out rather than omitted so this stays honest under
    // a seeder that only handles `undefined`.
    const seeded = seedModelNumerics({
      provider: "openai",
      model: "gpt-4o",
      max_tokens: null,
      temperature: null,
      top_p: null,
      frequency_penalty: null,
      presence_penalty: null,
      context_window: null,
      max_output_tokens: null,
    });
    for (const field of MODEL_NUMERIC_FIELDS) {
      expect(seeded[field], field).toBe("");
    }
  });

  it("treats an absent key as inherit too, which is the shape the daemon actually sends", () => {
    // `ModelConfig` carries `skip_serializing_if = "Option::is_none"`, so an
    // unset field arrives missing rather than null.
    const seeded = seedModelNumerics({ provider: "openai", model: "gpt-4o" });
    for (const field of MODEL_NUMERIC_FIELDS) {
      expect(seeded[field], field).toBe("");
    }
  });

  it("carries a pinned zero through rather than reading it as absent", () => {
    const seeded = seedModelNumerics({ temperature: 0, presence_penalty: 0 });
    expect(seeded.temperature).toBe("0");
    expect(seeded.presence_penalty).toBe("0");
  });
});
