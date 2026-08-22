import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import { StepLadderInput } from "./StepLadderInput";
import { MAX_OUTPUT_TOKENS_LADDER } from "../../lib/modelParamLadders";

function Harness({ initial = "", cap }: { initial?: string; cap?: number }) {
  const [value, setValue] = useState(initial);
  return (
    <>
      <StepLadderInput
        label="Response length"
        value={value}
        onChange={setValue}
        ladder={MAX_OUTPUT_TOKENS_LADDER}
        cap={cap}
        inheritLabel="inherit"
        customLabel="custom"
      />
      <output data-testid="value">{value === "" ? "<empty>" : value}</output>
    </>
  );
}

const pressed = (name: string): boolean =>
  screen.getByRole("button", { name }).getAttribute("aria-pressed") === "true";

describe("StepLadderInput", () => {
  it("offers inherit as a rung rather than an empty box", async () => {
    render(<Harness />);
    // "This agent has no opinion" is something the operator can point at, not
    // something they infer from a blank field.
    expect(pressed("inherit")).toBe(true);
    expect(screen.getByTestId("value")).toHaveTextContent("<empty>");

    await userEvent.click(screen.getByRole("button", { name: "8K" }));
    expect(screen.getByTestId("value")).toHaveTextContent("8192");
    expect(pressed("inherit")).toBe(false);

    await userEvent.click(screen.getByRole("button", { name: "inherit" }));
    expect(screen.getByTestId("value")).toHaveTextContent("<empty>");
  });

  it("renders every rung of the ladder", () => {
    render(<Harness />);
    for (const label of ["1K", "4K", "8K", "16K", "32K", "64K", "128K"]) {
      expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
    }
  });

  it("opens a field for a value that is not on the ladder", async () => {
    render(<Harness />);
    expect(screen.queryByRole("spinbutton")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "custom" }));
    const field = screen.getByRole("spinbutton");
    await userEvent.clear(field);
    await userEvent.type(field, "50000");

    expect(screen.getByTestId("value")).toHaveTextContent("50000");
    // The custom rung stays selected while the value is off-ladder, so the
    // control does not flicker back to a preset mid-edit.
    expect(pressed("custom")).toBe(true);
  });

  it("seeds the custom field from the current preset instead of an empty box", async () => {
    render(<Harness initial="8192" />);
    await userEvent.click(screen.getByRole("button", { name: "custom" }));
    expect(screen.getByRole("spinbutton")).toHaveValue(8192);
  });

  it("re-selects a preset when a typed value lands back on the ladder", async () => {
    render(<Harness initial="50000" />);
    expect(pressed("custom")).toBe(true);

    await userEvent.click(screen.getByRole("button", { name: "16K" }));
    expect(pressed("16K")).toBe(true);
    expect(pressed("custom")).toBe(false);
    expect(screen.queryByRole("spinbutton")).not.toBeInTheDocument();
  });

  it("hides rungs above a declared cap and keeps the cap itself selectable", () => {
    render(<Harness cap={20_000} />);
    expect(screen.getByRole("button", { name: "16K" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "32K" })).not.toBeInTheDocument();
    // The cap sits between two rungs, so it is offered on its own.
    expect(screen.getByRole("button", { name: "20000" })).toBeInTheDocument();
  });

  // An unknown limit is not a ceiling (#7780): with no cap the operator keeps
  // the whole ladder rather than being fenced in by a placeholder.
  it("keeps the whole ladder when no cap was sourced", () => {
    render(<Harness />);
    expect(screen.getByRole("button", { name: "128K" })).toBeInTheDocument();
  });

  it("shows an advisory without disabling anything", async () => {
    render(
      <StepLadderInput
        label="Response length"
        value="65536"
        onChange={() => {}}
        ladder={MAX_OUTPUT_TOKENS_LADDER}
        inheritLabel="inherit"
        customLabel="custom"
        warning="Above this model's limit of 16K."
      />,
    );
    expect(screen.getByText(/Above this model's limit of 16K\./)).toBeInTheDocument();
    // Advisory, not a block: every rung stays clickable.
    for (const button of screen.getAllByRole("button")) {
      expect(button).not.toBeDisabled();
    }
  });
});
