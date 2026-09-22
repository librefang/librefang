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
   * The rule this function exists to correct. Typing the model's catalog figure
   * used to be read as "same as the default, so clear it" — but an absent
   * override does not request that figure, it falls through to the kernel's own
   * default. The old rule therefore discarded a deliberate setting and left the
   * model somewhere the operator never chose.
   *
   * Capacity is not a parameter here, so there is no value of it that can make a
   * typed number vanish. The call sites pass it to the display path only.
   */
  it("keeps a typed value that happens to equal the model's catalog figure", () => {
    const draft = resolveLimitDraft("16384", undefined);
    expect(draft.value).toBe(16384);
    expect(draft.dirty).toBe(true);
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
