import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ListField } from "./ListField";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) =>
      opts?.defaultValue ?? (key === "common.remove" ? "Remove" : key),
  }),
}));

function Harness({
  initial = [],
  disabled,
  splitOnComma,
}: {
  initial?: string[];
  disabled?: boolean;
  splitOnComma?: boolean;
}) {
  const [value, setValue] = useState(initial);
  return (
    <ListField
      value={value}
      onChange={setValue}
      placeholder="Add item"
      ariaLabel="Items"
      disabled={disabled}
      splitOnComma={splitOnComma}
    />
  );
}

const getInput = () => screen.getByRole("textbox", { name: "Items" });

describe("ListField", () => {
  it("commits the typed value on Enter", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(getInput(), "alpha{Enter}");

    expect(screen.getByText("alpha")).toBeInTheDocument();
    expect(getInput()).toHaveValue("");
  });

  it("commits the typed value on comma", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(getInput(), "alpha,");

    expect(screen.getByText("alpha")).toBeInTheDocument();
    expect(getInput()).toHaveValue("");
  });

  it("splits pasted comma, semicolon and newline separated text into chips", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    const input = getInput();
    await user.click(input);
    await user.paste("alpha, beta\ngamma; delta");

    for (const item of ["alpha", "beta", "gamma", "delta"]) {
      expect(screen.getByText(item)).toBeInTheDocument();
    }
    expect(getInput()).toHaveValue("");
  });

  it("drops duplicates already in the list and within the paste", async () => {
    const user = userEvent.setup();
    render(<Harness initial={["alpha"]} />);

    await user.type(getInput(), "alpha{Enter}");
    expect(screen.getAllByText("alpha")).toHaveLength(1);

    const input = getInput();
    await user.click(input);
    await user.paste("beta, alpha, beta");
    expect(screen.getAllByText("alpha")).toHaveLength(1);
    expect(screen.getAllByText("beta")).toHaveLength(1);
  });

  it("removes the last chip with Backspace when the input is empty", async () => {
    const user = userEvent.setup();
    render(<Harness initial={["alpha", "beta"]} />);

    await user.click(getInput());
    await user.keyboard("{Backspace}");

    expect(screen.queryByText("beta")).not.toBeInTheDocument();
    expect(screen.getByText("alpha")).toBeInTheDocument();
  });

  it("commits the typed value when the field loses focus", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(getInput(), "gamma");
    await user.click(document.body);

    expect(screen.getByText("gamma")).toBeInTheDocument();
    expect(getInput()).toHaveValue("");
  });

  it("disables the input and hides the remove buttons when disabled", () => {
    render(<Harness initial={["alpha"]} disabled />);

    expect(getInput()).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Remove alpha" })).not.toBeInTheDocument();
  });

  it("removes a chip when its remove button is clicked", async () => {
    const user = userEvent.setup();
    render(<Harness initial={["alpha", "beta"]} />);

    await user.click(screen.getByRole("button", { name: "Remove alpha" }));

    expect(screen.queryByText("alpha")).not.toBeInTheDocument();
    expect(screen.getByText("beta")).toBeInTheDocument();
  });

  it("keeps the draft when a blur is followed by a remove click", async () => {
    const user = userEvent.setup();
    render(<Harness initial={["alpha", "beta"]} />);

    await user.type(getInput(), "gamma");
    await user.click(screen.getByRole("button", { name: "Remove alpha" }));

    expect(screen.queryByText("alpha")).not.toBeInTheDocument();
    expect(screen.getAllByText("beta")).toHaveLength(1);
    expect(screen.getAllByText("gamma")).toHaveLength(1);
  });

  it("does nothing on Enter when the input is empty", async () => {
    const user = userEvent.setup();
    render(<Harness initial={["alpha"]} />);

    await user.click(getInput());
    await user.keyboard("{Enter}");

    expect(getInput()).toHaveValue("");
    expect(screen.getAllByText("alpha")).toHaveLength(1);
  });

  it("keeps a separator-free paste as text until it is confirmed", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    const input = getInput();
    await user.click(input);
    await user.paste("alpha beta");

    expect(input).toHaveValue("alpha beta");
    expect(screen.queryByText("alpha beta")).not.toBeInTheDocument();

    await user.keyboard("{Enter}");
    expect(screen.getByText("alpha beta")).toBeInTheDocument();
  });

  it("renders a single chip for duplicate values without rewriting the prop", () => {
    render(<Harness initial={["a", "a", "b"]} />);

    expect(screen.getAllByText("a")).toHaveLength(1);
    expect(screen.getAllByText("b")).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Remove a" })).toBeInTheDocument();
  });

  it("splits a comma-bearing paste by default", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    const input = getInput();
    await user.click(input);
    await user.paste("^(foo|bar){2,3}$");

    expect(screen.getByText("^(foo|bar){2")).toBeInTheDocument();
    expect(screen.getByText("3}$")).toBeInTheDocument();
  });

  it("keeps commas inside the value when splitOnComma is false", async () => {
    const user = userEvent.setup();
    render(<Harness splitOnComma={false} />);

    const input = getInput();
    await user.click(input);
    await user.paste("^(foo|bar){2,3}$");

    expect(input).toHaveValue("^(foo|bar){2,3}$");
    await user.keyboard("{Enter}");

    expect(screen.getByText("^(foo|bar){2,3}$")).toBeInTheDocument();
    expect(screen.queryByText("^(foo|bar){2")).not.toBeInTheDocument();
    expect(screen.queryByText("3}$")).not.toBeInTheDocument();
  });
});
