import { describe, expect, it } from "vitest";
import { resolveLimitDraft } from "./modelOverrideDraft";

describe("resolveLimitDraft", () => {
  it("treats an empty field as clearing the override", () => {
    expect(resolveLimitDraft("", 8000)).toEqual({
      value: null,
      invalid: false,
      dirty: true,
    });
    // Already clear: nothing to save.
    expect(resolveLimitDraft("", undefined).dirty).toBe(false);
  });

  it("sets the override to any positive whole number", () => {
    expect(resolveLimitDraft("8192", undefined)).toEqual({
      value: 8192,
      invalid: false,
      dirty: true,
    });
    expect(resolveLimitDraft("8192", 8192).dirty).toBe(false);
  });

  /**
   * `max_tokens`: since #8502 an absent override resolves to the model's output
   * ceiling (the catalog entry's `max_output_tokens`, or the operator's
   * correction of it) before it falls back to `DEFAULT_MODEL_MAX_TOKENS`. A
   * typed value equal to that ceiling is therefore a redundant override that
   * would pin it and shadow a later registry or discovery correction, exactly
   * like `context_window`, so it clears rather than persists.
   *
   * With no declared ceiling there is nothing to compare against — the daemon
   * default is not passed in — so a typed value stands on its own.
   */
  it("clears a max_tokens value equal to the ceiling an absent override resolves to", () => {
    expect(resolveLimitDraft("16384", undefined, 16384)).toEqual({
      value: null,
      invalid: false,
      dirty: false,
    });
    // Clearing an already-stored redundant override is a real change.
    expect(resolveLimitDraft("16384", 16384, 16384).dirty).toBe(true);
    // A deliberate different number still stores.
    expect(resolveLimitDraft("8192", undefined, 16384)).toEqual({
      value: 8192,
      invalid: false,
      dirty: true,
    });
    // Nothing declares a ceiling: the typed value stands.
    expect(resolveLimitDraft("32768", undefined).value).toBe(32768);
  });

  /**
   * `context_window`: an absent override *does* resolve to the catalog figure
   * (`resolve_context_window` ranks agent manifest → model_overrides.json →
   * ModelCatalog), so the caller passes that figure as `catalogValue`. Typing it
   * is a redundant override that pins the window and shadows a later catalog
   * correction, so it clears rather than persists.
   */
  it("clears a context_window value equal to the figure an absent override resolves to", () => {
    expect(resolveLimitDraft("131072", undefined, 131072)).toEqual({
      value: null,
      invalid: false,
      dirty: false,
    });
    // Clearing an already-stored redundant override is a real change.
    expect(resolveLimitDraft("131072", 131072, 131072).dirty).toBe(true);
    // A deliberate different number still stores.
    expect(resolveLimitDraft("200000", undefined, 131072)).toEqual({
      value: 200000,
      invalid: false,
      dirty: true,
    });
    // With nothing to compare against, the typed value stands.
    expect(resolveLimitDraft("131072", undefined).value).toBe(131072);
  });

  it("rejects values that are not positive whole numbers", () => {
    for (const bad of ["0", "-1", "1.5", "lots"]) {
      expect(resolveLimitDraft(bad, undefined).invalid).toBe(true);
    }
  });

  it("ignores surrounding whitespace", () => {
    expect(resolveLimitDraft("  4096  ", undefined).value).toBe(4096);
    expect(resolveLimitDraft("   ", 4096)).toEqual({
      value: null,
      invalid: false,
      dirty: true,
    });
  });

  /** A value equal to what is already stored is not dirty, whatever it equals. */
  it("is not dirty when the typed value matches the stored override", () => {
    expect(resolveLimitDraft("131072", 131072).dirty).toBe(false);
  });
});
