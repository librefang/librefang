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
});
