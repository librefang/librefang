import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Select } from "./Select";

const options = [{ value: "one", label: "One" }];

describe("Select", () => {
  it("associates a visible label with the select", () => {
    render(<Select label="Mode" options={options} />);

    expect(screen.getByRole("combobox", { name: "Mode" })).toBeInTheDocument();
  });

  it("supports an aria-label when a visible label is omitted", () => {
    render(<Select aria-label="Mode" options={options} />);

    expect(screen.getByRole("combobox", { name: "Mode" })).toBeInTheDocument();
    expect(screen.queryByText("Mode")).not.toBeInTheDocument();
  });

  it("supports an external aria-labelledby name", () => {
    render(
      <>
        <span id="mode-label">Mode</span>
        <Select aria-labelledby="mode-label" options={options} />
      </>,
    );

    expect(screen.getByRole("combobox", { name: "Mode" })).toBeInTheDocument();
  });
});
