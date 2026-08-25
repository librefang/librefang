import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SliderInput } from "./SliderInput";

describe("SliderInput", () => {
  it("normalizes inverted bounds for both controls and the track fill", () => {
    render(
      <SliderInput
        label="Temperature"
        value={7}
        min={10}
        max={0}
        onChange={() => {}}
      />,
    );

    const numberInput = screen.getByRole("spinbutton");
    const rangeInput = screen.getByRole("slider");
    expect(numberInput).toHaveAttribute("min", "0");
    expect(numberInput).toHaveAttribute("max", "10");
    expect(rangeInput).toHaveAttribute("min", "0");
    expect(rangeInput).toHaveAttribute("max", "10");
    expect(rangeInput.style.background).toContain("70%");
  });

  it("clamps values from either input path", () => {
    const onChange = vi.fn();
    render(
      <SliderInput label="Temperature" value={5} min={0} max={10} onChange={onChange} />,
    );

    fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "12" } });
    expect(onChange).toHaveBeenLastCalledWith(10);

    fireEvent.change(screen.getByRole("slider"), { target: { value: "-4" } });
    expect(onChange).toHaveBeenLastCalledWith(0);
  });

  it("ignores non-finite number input and renders duplicate ticks", () => {
    const onChange = vi.fn();
    render(
      <SliderInput
        label="Temperature"
        value={5}
        min={0}
        max={10}
        ticks={[5, 5]}
        onChange={onChange}
      />,
    );

    fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "Infinity" } });
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getAllByText("5")).toHaveLength(2);
  });

  describe("inherited (disabled) state", () => {
    const renderInherited = (onToggle = vi.fn()) => {
      render(
        <SliderInput
          label="Temperature"
          value={0.7}
          min={0}
          max={2}
          step={0.1}
          ticks={[0, 1, 2]}
          enabled={false}
          onToggle={onToggle}
          onChange={() => {}}
        />,
      );
      return onToggle;
    };

    it("leaves the switch undimmed and clickable so the row can be activated", () => {
      const onToggle = renderInherited();
      const toggle = screen.getByRole("switch");

      expect(toggle).toBeEnabled();
      expect(toggle).toHaveAttribute("aria-checked", "false");
      // No ancestor may fade the switch either: the row container used to carry
      // the opacity, which dimmed the only control able to leave this state.
      expect(toggle.closest("[class*='opacity-']")).toBeNull();
      expect(toggle.className).not.toMatch(/\bopacity-/);
      // The off track needs a control-weight token, not the divider hairline.
      expect(toggle.className).not.toMatch(/bg-border-subtle/);

      fireEvent.click(toggle);
      expect(onToggle).toHaveBeenCalledWith(true);
    });

    it("still dims the values the row is inheriting", () => {
      renderInherited();

      expect(screen.getByText("Temperature").className).toMatch(/opacity-40/);
      expect(screen.getByRole("spinbutton").className).toMatch(/opacity-40/);
      expect(screen.getByRole("slider").className).toMatch(/opacity-40/);
      expect(screen.getByText("1").parentElement?.className).toMatch(/opacity-40/);
    });

    it("undims the values and checks the switch once the row is active", () => {
      render(
        <SliderInput
          label="Temperature"
          value={0.7}
          min={0}
          max={2}
          step={0.1}
          ticks={[0, 1, 2]}
          enabled
          onToggle={vi.fn()}
          onChange={() => {}}
        />,
      );

      expect(screen.getByRole("switch")).toHaveAttribute("aria-checked", "true");
      expect(screen.getByText("Temperature").className).not.toMatch(/opacity-40/);
      expect(screen.getByRole("spinbutton").className).not.toMatch(/opacity-40/);
      expect(screen.getByRole("slider").className).not.toMatch(/opacity-40/);
    });
  });
});
