import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import { StepLadderInput } from "./StepLadderInput";
import {
  MAX_OUTPUT_TOKENS_LADDER,
  PENALTY_LADDER,
  TEMPERATURE_LADDER,
} from "../../lib/modelParamLadders";

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

/**
 * A call site that stores the parsed number instead of the typed text.
 *
 * This is what the model settings drawer does — its reducer holds
 * `temperature: number` rather than the string the field emits. These guard
 * that a fractional and a negative value survive that round trip, which is the
 * shape the token-count parameters never exercised: every prefix of "128000"
 * is already a whole number, so `String(Number(x))` returned what was typed.
 */
function NumericHarness({ ladder }: { ladder: readonly number[] }) {
  const [num, setNum] = useState<number | null>(null);
  return (
    <>
      <StepLadderInput
        label="Temperature"
        value={num === null ? "" : String(num)}
        onChange={(next) => {
          if (next.trim() === "") {
            setNum(null);
            return;
          }
          const parsed = Number(next);
          // The drawer refuses a value it cannot store, exactly as here.
          if (!Number.isFinite(parsed)) return;
          setNum(parsed);
        }}
        ladder={ladder}
        inheritLabel="inherit"
        customLabel="custom"
        min={-2}
        max={2}
        step={0.01}
      />
      <output data-testid="value">{num === null ? "<empty>" : String(num)}</output>
    </>
  );
}

describe("StepLadderInput — custom entry against a value-parsing call site", () => {
  it("lets a decimal be typed one character at a time", async () => {
    render(<NumericHarness ladder={TEMPERATURE_LADDER} />);
    await userEvent.click(screen.getByRole("button", { name: "custom" }));

    const field = screen.getByRole("spinbutton", { name: "Temperature — custom" });
    await userEvent.type(field, "0.65");

    expect(field).toHaveValue(0.65);
    expect(screen.getByTestId("value")).toHaveTextContent("0.65");
  });

  it("lets a negative penalty be typed, whose first character is not a number", async () => {
    render(<NumericHarness ladder={PENALTY_LADDER} />);
    await userEvent.click(screen.getByRole("button", { name: "custom" }));

    const field = screen.getByRole("spinbutton", { name: "Temperature — custom" });
    await userEvent.type(field, "-1.25");

    expect(field).toHaveValue(-1.25);
    expect(screen.getByTestId("value")).toHaveTextContent("-1.25");
  });

  it("accepts a value the old hardcoded min would have marked invalid", async () => {
    render(<NumericHarness ladder={TEMPERATURE_LADDER} />);
    await userEvent.click(screen.getByRole("button", { name: "custom" }));

    const field = screen.getByRole("spinbutton", { name: "Temperature — custom" });
    // `min="1"` is right for a token count and wrong for a temperature: 0 is a
    // deliberate setting, not an empty field.
    expect(field).toHaveAttribute("min", "-2");
    await userEvent.type(field, "0");
    expect(field).toBeValid();
  });
});

describe("StepLadderInput — a value that passes through negative zero", () => {
  /**
   * `userEvent.type` cannot show this one. A real `<input type="number">`
   * sanitizes its `value` to `""` for anything that is not yet a valid float,
   * so typing `-0.25` reaches the handler as `"-0"`, `""`, `"-0.2"`, `"-0.25"`;
   * jsdom does not implement that sanitization and hands the raw buffer over,
   * so the round trip that breaks in a browser never happens in the test.
   *
   * `fireEvent.change` with the exact strings a browser reports is what makes
   * it visible. The defect: a parent that stores the parsed number takes
   * `Number("-0")` — a legitimate penalty — and hands back `String(-0)`, which
   * is `"0"`. The field is controlled, so the minus sign is erased from under
   * the operator mid-keystroke, and the rest of what they type lands on a
   * positive number. They save `+0.25` having typed `-0.25`, with no error.
   */
  it("does not erase the minus sign when the parent normalises -0 to \"0\"", () => {
    render(<NumericHarness ladder={PENALTY_LADDER} />);
    fireEvent.click(screen.getByRole("button", { name: "custom" }));

    const field = screen.getByRole("spinbutton", {
      name: "Temperature — custom",
    }) as HTMLInputElement;
    fireEvent.change(field, { target: { value: "-0" } });

    // What the operator can still see and keep typing into.
    expect(field.value).toBe("-0");
  });
});
