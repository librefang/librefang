import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, it, expect, vi } from "vitest";
import { AgentManifestForm, type ManifestCatalogEntry } from "./AgentManifestForm";
import {
  emptyManifestExtras,
  emptyManifestForm,
  type ManifestFormState,
} from "../lib/agentManifest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, opts?: { defaultValue?: string } | Record<string, unknown>) => {
      if (opts && typeof opts === "object" && "defaultValue" in opts) {
        return (opts as { defaultValue?: string }).defaultValue ?? _key;
      }
      return _key;
    },
  }),
}));

function Harness({
  skillCatalog,
  toolCatalog,
  mcpCatalog,
  initialState,
  invalidFields = new Set(),
}: {
  skillCatalog?: ManifestCatalogEntry[];
  toolCatalog?: ManifestCatalogEntry[];
  mcpCatalog?: ManifestCatalogEntry[];
  initialState?: ManifestFormState;
  invalidFields?: Set<string>;
}) {
  const [state, setState] = useState<ManifestFormState>(() => initialState ?? emptyManifestForm());
  return (
    <AgentManifestForm
      value={state}
      onChange={setState}
      providers={[{ name: "openai" }]}
      models={[{ provider: "openai", id: "gpt-4o" }]}
      invalidFields={invalidFields}
      extras={emptyManifestExtras()}
      skillCatalog={skillCatalog}
      toolCatalog={toolCatalog}
      mcpCatalog={mcpCatalog}
    />
  );
}

describe("AgentManifestForm — validation feedback", () => {
  it("opens scheduling errors and exposes the cron error to assistive technology", () => {
    const state = emptyManifestForm();
    state.schedule = { mode: "periodic", cron: "" };

    render(<Harness initialState={state} invalidFields={new Set(["schedule.cron"])} />);

    const input = screen.getByRole("textbox", { name: "agents.form.cron" });
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(input).toHaveAttribute("aria-required", "true");
    expect(input).toHaveAccessibleDescription("agents.form.cron_required_error");
    expect(input.closest("details")).toHaveAttribute("open");
    expect(input.closest("details")?.querySelector("summary")).toHaveAttribute(
      "aria-invalid",
      "true",
    );
  });

  it("opens response-format errors and exposes the schema error to assistive technology", () => {
    const state = emptyManifestForm();
    state.response_format = { mode: "json_schema", name: "response", schema: "", strict: false };

    render(
      <Harness
        initialState={state}
        invalidFields={new Set(["response_format.schema"])}
      />,
    );

    const textarea = screen.getByRole("textbox", { name: "agents.form.schema_body" });
    expect(textarea).toHaveAttribute("aria-invalid", "true");
    expect(textarea).toHaveAttribute("aria-required", "true");
    expect(textarea).toHaveAccessibleDescription("agents.form.schema_invalid_error");
    expect(textarea.closest("details")).toHaveAttribute("open");
    expect(textarea.closest("details")?.querySelector("summary")).toHaveAttribute(
      "aria-invalid",
      "true",
    );
  });
});

describe("AgentManifestForm — tools/skills/mcp selection (#5246)", () => {
  it("clicking a tool option from the dropdown adds it as a chip", async () => {
    const user = userEvent.setup();
    render(
      <Harness
        toolCatalog={[
          { name: "read_file", description: "Read a file" },
          { name: "write_file", description: "Write a file" },
        ]}
      />,
    );

    // Open the tools combobox: target the search input by its placeholder.
    const toolsInput = screen.getByPlaceholderText("Search tools…");
    await user.click(toolsInput);

    // Wait for the option to appear, then click it.
    const option = await screen.findByText("read_file");
    await user.click(option);

    // Chip should appear; remove button is the canonical signal.
    expect(
      screen.getByRole("button", { name: "Remove read_file" }),
    ).toBeInTheDocument();
  });

  it("clicking a skill option from the dropdown adds it as a chip", async () => {
    const user = userEvent.setup();
    render(
      <Harness
        skillCatalog={[
          { name: "summarise", description: "Summarise text" },
          { name: "translate", description: "Translate text" },
        ]}
      />,
    );

    const skillsInput = screen.getByPlaceholderText("Search installed skills…");
    await user.click(skillsInput);

    const option = await screen.findByText("summarise");
    await user.click(option);

    expect(
      screen.getByRole("button", { name: "Remove summarise" }),
    ).toBeInTheDocument();
  });

  it("clicking an MCP server option adds it as a chip (#5246)", async () => {
    const user = userEvent.setup();
    render(
      <Harness
        mcpCatalog={[
          { name: "filesystem", description: "Local filesystem MCP" },
          { name: "github", description: "GitHub MCP" },
        ]}
      />,
    );

    // The MCP field should render a combobox, not a free-text TagInput.
    const mcpInput = screen.getByPlaceholderText("Search MCP servers…");
    await user.click(mcpInput);

    const option = await screen.findByText("github");
    await user.click(option);

    expect(
      screen.getByRole("button", { name: "Remove github" }),
    ).toBeInTheDocument();
  });

  it("when no MCP catalog is supplied, falls back to a tag input (no crash)", async () => {
    render(<Harness />);
    // The mcp_servers Field always exists; without a catalog the TagInput is used
    // — verified by the absence of the cmdk search placeholder.
    expect(screen.queryByPlaceholderText("Search MCP servers…")).not.toBeInTheDocument();
  });

  it("tool dropdown options are within a listbox region after focus", async () => {
    const user = userEvent.setup();
    render(
      <Harness
        toolCatalog={[
          { name: "read_file" },
          { name: "write_file" },
        ]}
      />,
    );
    const toolsInput = screen.getByPlaceholderText("Search tools…");
    await user.click(toolsInput);

    const list = await screen.findByRole("listbox");
    expect(within(list).getByText("read_file")).toBeInTheDocument();
    expect(within(list).getByText("write_file")).toBeInTheDocument();
  });
});

describe("AgentManifestForm — compact controls", () => {
  it("clears duplicate text submitted to a tag input", async () => {
    const user = userEvent.setup();
    const state = emptyManifestForm();
    state.mcp_servers = ["filesystem"];
    render(<Harness initialState={state} />);

    const removeButton = screen.getByRole("button", { name: "remove filesystem" });
    const input = removeButton.parentElement?.parentElement?.querySelector("input");
    expect(input).toBeInstanceOf(HTMLInputElement);
    if (!(input instanceof HTMLInputElement)) return;

    await user.type(input, "filesystem{Enter}");
    expect(input).toHaveValue("");
    expect(screen.getAllByRole("button", { name: "remove filesystem" })).toHaveLength(1);
  });

  it("gives the stream-thinking checkbox an accessible name", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByRole("checkbox", { name: "agents.form.thinking_enabled" }));

    expect(
      screen.getByRole("checkbox", { name: "agents.form.stream_thinking" }),
    ).toBeInTheDocument();
  });
});
