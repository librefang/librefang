import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import {
  JsonEditor,
  configSavePresentation,
  configSectionTabClass,
  configValuesEqual,
  effectiveConfigTab,
} from "./ConfigPage";

describe("ConfigPage form state helpers", () => {
  it("preserves an invalid JSON draft across server value updates", () => {
    const onChange = vi.fn();
    const view = render(<JsonEditor value={{ enabled: true }} onChange={onChange} />);
    const editor = screen.getByRole("textbox");

    fireEvent.change(editor, { target: { value: '{"enabled":' } });
    view.rerender(<JsonEditor value={{ enabled: false }} onChange={onChange} />);

    expect(editor).toHaveValue('{"enabled":');
    expect(onChange).not.toHaveBeenCalled();
  });

  it("compares nested config values without object key-order sensitivity", () => {
    expect(
      configValuesEqual(
        { outer: { alpha: 1, beta: [2, 3] } },
        { outer: { beta: [2, 3], alpha: 1 } },
      ),
    ).toBe(true);
    expect(configValuesEqual({ alpha: 1 }, { alpha: 2 })).toBe(false);
  });

  it("selects explicit save status branches", () => {
    const t = (key: string, fallback: string) => `${key}:${fallback}`;

    expect(
      configSavePresentation(
        { status: "saved_reload_failed", reload_error: "bad TOML" },
        t,
      ),
    ).toEqual({
      ok: false,
      msg: "config.saved_reload_failed:Saved but reload failed: bad TOML",
    });
    expect(configSavePresentation({ restart_required: true }, t).ok).toBe(true);
  });

  it("derives effective sections and tab styling without nested branches", () => {
    expect(effectiveConfigTab(true, "general", ["general"])).toBeNull();
    expect(effectiveConfigTab(false, "missing", ["general"])).toBe("general");
    expect(configSectionTabClass(true, false)).toBe("border-brand text-brand");
    expect(configSectionTabClass(false, true)).toContain("cursor-not-allowed");
  });
});
