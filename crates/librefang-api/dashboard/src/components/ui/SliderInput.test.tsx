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

  /**
   * The legend is a reading aid for the track above it, so a label has to sit
   * over the position its own value occupies. Even spacing (`justify-between`)
   * looks tidy and lies: on the real context-window row — min 1024, max 2M,
   * ticks 32K/128K/512K/1M — it drew "1M" hard right when 1M is the midpoint,
   * and "128K" a third of the way across when its true position is 6%.
   */
  describe("tick legend", () => {
    const ladder = {
      label: "Context window",
      value: 131072,
      min: 1024,
      max: 2097152,
      ticks: [32768, 131072, 524288, 1048576],
      formatTick: (v: number) =>
        v >= 1048576 ? `${Math.round(v / 1048576)}M` : `${Math.round(v / 1024)}K`,
    };

    const leftOf = (text: string) =>
      Number.parseFloat((screen.getByText(text) as HTMLElement).style.left);

    it("places each label at the position its value maps to", () => {
      render(<SliderInput {...ladder} onChange={() => {}} />);

      // (value - min) / (max - min), the same expression the filled track uses.
      expect(leftOf("32K")).toBeCloseTo(1.51, 1);
      expect(leftOf("128K")).toBeCloseTo(6.2, 1);
      expect(leftOf("512K")).toBeCloseTo(24.96, 1);
      expect(leftOf("1M")).toBeCloseTo(49.97, 1);
    });

    it("does not fall back to even spacing when the values are not evenly spread", () => {
      render(<SliderInput {...ladder} onChange={() => {}} />);

      // The exact regression: evenly spaced would put these at 0 / 33 / 66 / 100.
      expect(leftOf("128K")).toBeLessThan(20);
      expect(leftOf("1M")).toBeLessThan(60);
    });

    it("pulls the end labels back inside the track instead of centring them", () => {
      render(
        <SliderInput
          label="Temperature"
          value={1}
          min={0}
          max={2}
          ticks={[0, 1, 2]}
          onChange={() => {}}
        />,
      );

      // Centring at 0% and 100% would hang half of each label off the track.
      expect(screen.getByText("0").className).toMatch(/translate-x-0(\s|$)/);
      expect(screen.getByText("2").className).toMatch(/-translate-x-full/);
      expect(screen.getByText("1").className).toMatch(/-translate-x-1\/2/);
    });
  });
});
