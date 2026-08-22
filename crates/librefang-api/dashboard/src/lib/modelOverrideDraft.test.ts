import { describe, expect, it } from "vitest";
import { resolveMaxTokensDraft } from "./modelOverrideDraft";

describe("resolveMaxTokensDraft", () => {
  it("treats an empty field as clearing the override", () => {
    expect(resolveMaxTokensDraft("", 8000)).toEqual({
      value: null,
      invalid: false,
      dirty: true,
    });
    // Already clear: nothing to save.
    expect(resolveMaxTokensDraft("", undefined).dirty).toBe(false);
  });

  it("sets the override to any positive whole number", () => {
    expect(resolveMaxTokensDraft("8192", undefined)).toEqual({
      value: 8192,
      invalid: false,
      dirty: true,
    });
    expect(resolveMaxTokensDraft("8192", 8192).dirty).toBe(false);
  });

  /**
   * The rule this function exists to correct. Typing the model's catalog
   * capacity used to be read as "same as the default, so clear it" — but an
   * absent override does not request the capacity, it falls through to the
   * kernel's own default. The old rule therefore discarded a deliberate
   * setting and left the model somewhere the operator never chose.
   *
   * Capacity is not a parameter here, so there is no value of it that can make
   * a typed number vanish.
   */
  it("keeps a typed value that happens to equal the model's capacity", () => {
    const draft = resolveMaxTokensDraft("16384", undefined);
    expect(draft.value).toBe(16384);
    expect(draft.dirty).toBe(true);
  });

  it("rejects values that are not positive whole numbers", () => {
    for (const bad of ["0", "-1", "1.5", "lots"]) {
      expect(resolveMaxTokensDraft(bad, undefined).invalid).toBe(true);
    }
  });

  it("ignores surrounding whitespace", () => {
    expect(resolveMaxTokensDraft("  4096  ", undefined).value).toBe(4096);
    expect(resolveMaxTokensDraft("   ", 4096)).toEqual({
      value: null,
      invalid: false,
      dirty: true,
    });
  });
});
