import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, it, expect, vi } from "vitest";
import { MultiSelectCmdk } from "./MultiSelectCmdk";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, opts?: { defaultValue?: string }) =>
      opts?.defaultValue ?? _key,
  }),
}));

const OPTIONS = ["alpha", "beta", "gamma", "delta"];

function Harness({
  initial = [] as string[],
  options = OPTIONS,
  placeholder = "Pick one…",
  disabled = false,
}: {
  initial?: string[];
  options?: string[];
  placeholder?: string;
  disabled?: boolean;
}) {
  const [value, setValue] = useState<string[]>(initial);
  return (
    <MultiSelectCmdk
      options={options}
      value={value}
      onChange={(next) =>
        setValue((prev) => (typeof next === "function" ? next(prev) : next))
      }
      placeholder={placeholder}
      disabled={disabled}
    />
  );
}

/** Grab the component's own search input via its placeholder text. */
function getInput() {
  // When no chips are selected the placeholder prop is shown; when chips exist "Add more…" is shown.
  const el =
    screen.queryByPlaceholderText("Pick one…") ??
    screen.getByPlaceholderText("Add more…");
  return el;
}

describe("MultiSelectCmdk", () => {
  it("renders placeholder when value is empty", () => {
    render(<Harness placeholder="Pick one…" />);
    // At least one input has the placeholder
    const inputs = screen.getAllByPlaceholderText("Pick one…");
    expect(inputs.length).toBeGreaterThan(0);
  });

  it("clicking an option calls onChange with the item added", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(getInput());

    const option = await screen.findByText("alpha");
    await user.click(option);

    // chip remove button for "alpha" should now be present
    expect(screen.getByRole("button", { name: "Remove alpha" })).toBeInTheDocument();
  });

  it("clicking × on a chip removes that item", async () => {
    const user = userEvent.setup();
    render(<Harness initial={["alpha", "beta"]} />);

    await user.click(screen.getByRole("button", { name: "Remove alpha" }));

    expect(screen.queryByRole("button", { name: "Remove alpha" })).not.toBeInTheDocument();
    // beta chip remains
    expect(screen.getByRole("button", { name: "Remove beta" })).toBeInTheDocument();
  });

  it("Backspace in empty input removes the last selected value", async () => {
    const user = userEvent.setup();
    render(<Harness initial={["alpha", "beta"]} />);

    const input = getInput();
    await user.click(input);
    await user.keyboard("{Backspace}");

    expect(screen.queryByRole("button", { name: "Remove beta" })).not.toBeInTheDocument();
    // alpha chip remains
    expect(screen.getByRole("button", { name: "Remove alpha" })).toBeInTheDocument();
  });

  it("search input filters options — non-matching options are not rendered", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    const input = getInput();
    await user.click(input);
    await user.type(input, "al");

    // "alpha" matches "al" — should be in the listbox
    const list = await screen.findByRole("listbox");
    expect(within(list).getByText("alpha")).toBeInTheDocument();

    // "beta", "gamma", "delta" do not match "al"
    expect(within(list).queryByText("beta")).not.toBeInTheDocument();
    expect(within(list).queryByText("gamma")).not.toBeInTheDocument();
    expect(within(list).queryByText("delta")).not.toBeInTheDocument();
  });

  it("already-selected options are not shown in the dropdown list", async () => {
    const user = userEvent.setup();
    render(<Harness initial={["alpha"]} />);

    await user.click(getInput());

    const list = await screen.findByRole("listbox");
    // "alpha" is already selected so it should not appear as a selectable item
    expect(within(list).queryByText("alpha")).not.toBeInTheDocument();
    // other options should appear
    expect(within(list).getByText("beta")).toBeInTheDocument();
  });

  it("renders description metadata under each option (#5049)", async () => {
    const user = userEvent.setup();
    function MetaHarness() {
      const [value, setValue] = useState<string[]>([]);
      return (
        <MultiSelectCmdk
          options={["read_file", "write_file"]}
          optionMeta={{
            read_file: { description: "Read a file from the agent workspace" },
            write_file: { description: "Write bytes to a file" },
          }}
          value={value}
          onChange={(next) =>
            setValue((prev) => (typeof next === "function" ? next(prev) : next))
          }
          placeholder="Search tools…"
        />
      );
    }
    render(<MetaHarness />);
    await user.click(screen.getByPlaceholderText("Search tools…"));
    const list = await screen.findByRole("listbox");
    expect(within(list).getByText("read_file")).toBeInTheDocument();
    expect(
      within(list).getByText("Read a file from the agent workspace"),
    ).toBeInTheDocument();
  });

  it("description text matches the search query (#5049)", async () => {
    const user = userEvent.setup();
    function MetaHarness() {
      const [value, setValue] = useState<string[]>([]);
      return (
        <MultiSelectCmdk
          options={["read_file", "write_file"]}
          optionMeta={{
            read_file: { description: "Read a workspace file" },
            write_file: { description: "Persist bytes" },
          }}
          value={value}
          onChange={(next) =>
            setValue((prev) => (typeof next === "function" ? next(prev) : next))
          }
          placeholder="Search…"
        />
      );
    }
    render(<MetaHarness />);
    const input = screen.getByPlaceholderText("Search…");
    await user.click(input);
    // Search by description keyword that only matches "read_file"
    await user.type(input, "workspace");
    const list = await screen.findByRole("listbox");
    expect(within(list).getByText("read_file")).toBeInTheDocument();
    expect(within(list).queryByText("write_file")).not.toBeInTheDocument();
  });

  it("allowFreeText=true commits a typed value with no match on Enter (#5049)", async () => {
    const user = userEvent.setup();
    function FreeTextHarness() {
      const [value, setValue] = useState<string[]>([]);
      return (
        <MultiSelectCmdk
          options={["alpha", "beta"]}
          value={value}
          onChange={(next) =>
            setValue((prev) => (typeof next === "function" ? next(prev) : next))
          }
          placeholder="Search…"
          allowFreeText
        />
      );
    }
    render(<FreeTextHarness />);
    const input = screen.getByPlaceholderText("Search…");
    await user.click(input);
    await user.type(input, "custom_tool");
    await user.keyboard("{Enter}");
    expect(
      screen.getByRole("button", { name: "Remove custom_tool" }),
    ).toBeInTheDocument();
  });

  it("allowFreeText=true does not duplicate an already-selected chip on Enter (#5049)", async () => {
    const user = userEvent.setup();
    function FreeTextHarness() {
      const [value, setValue] = useState<string[]>(["custom_tool"]);
      return (
        <MultiSelectCmdk
          options={["alpha", "beta"]}
          value={value}
          onChange={(next) =>
            setValue((prev) => (typeof next === "function" ? next(prev) : next))
          }
          placeholder="Search…"
          allowFreeText
        />
      );
    }
    render(<FreeTextHarness />);
    // Chip starts pre-selected — the input now uses the "Add more…" placeholder.
    const input = screen.getByPlaceholderText("Add more…");
    await user.click(input);
    await user.type(input, "custom_tool");
    await user.keyboard("{Enter}");

    // Only one chip — Enter on a duplicate must NOT create a second one.
    expect(
      screen.getAllByRole("button", { name: "Remove custom_tool" }),
    ).toHaveLength(1);
    // Input is cleared so the user can keep typing.
    expect(input).toHaveValue("");
  });

  it("allowFreeText=false does NOT commit free text on Enter (#5049)", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const input = getInput();
    await user.click(input);
    await user.type(input, "not_in_catalog");
    await user.keyboard("{Enter}");
    expect(
      screen.queryByRole("button", { name: "Remove not_in_catalog" }),
    ).not.toBeInTheDocument();
  });
});
