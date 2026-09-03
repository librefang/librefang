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

interface HarnessModel {
  provider: string;
  id: string;
  context_window?: number;
  max_output_tokens?: number;
  limits_known?: boolean;
}

function Harness({
  skillCatalog,
  toolCatalog,
  mcpCatalog,
  initialState,
  invalidFields = new Set(),
  models = [{ provider: "openai", id: "gpt-4o" }],
}: {
  skillCatalog?: ManifestCatalogEntry[];
  toolCatalog?: ManifestCatalogEntry[];
  mcpCatalog?: ManifestCatalogEntry[];
  initialState?: ManifestFormState;
  invalidFields?: Set<string>;
  models?: HarnessModel[];
}) {
  const [state, setState] = useState<ManifestFormState>(() => initialState ?? emptyManifestForm());
  return (
    <AgentManifestForm
      value={state}
      onChange={setState}
      providers={[{ name: "openai" }]}
      models={models}
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

  it("opens scheduling errors and exposes an invalid continuous interval", () => {
    const state = emptyManifestForm();
    state.schedule = { mode: "continuous", check_interval_secs: "0" };

    render(
      <Harness
        initialState={state}
        invalidFields={new Set(["schedule.check_interval_secs"])}
      />,
    );

    const input = screen.getByRole("spinbutton", {
      name: "agents.form.check_interval_secs",
    });
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(input).toHaveAttribute("aria-required", "true");
    expect(input).toHaveAccessibleDescription("agents.detail.schedule_invalid_interval");
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

describe("AgentManifestForm — inference parameters", () => {
  /** The four knobs an agent could not reach before (#7781). */
  it("lets the agent set every sampling preference, not just temperature and max_tokens", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    for (const label of [
      "agents.form.temperature",
      "agents.form.top_p",
      "agents.form.frequency_penalty",
      "agents.form.presence_penalty",
    ]) {
      expect(screen.getByRole("spinbutton", { name: label })).toBeInTheDocument();
    }

    const topP = screen.getByRole("spinbutton", { name: "agents.form.top_p" });
    await user.type(topP, "0.85");
    expect(topP).toHaveValue(0.85);
  });

  it("starts every knob on inherit rather than on a number nobody chose", () => {
    render(<Harness />);
    expect(screen.getByRole("spinbutton", { name: "agents.form.temperature" })).toHaveValue(null);
    // The ladder's inherit rung is pressed, which is the same state made visible.
    const inheritRungs = screen.getAllByRole("button", { name: "agents.form.inherit_default" });
    expect(inheritRungs.length).toBeGreaterThan(0);
    for (const rung of inheritRungs) {
      expect(rung).toHaveAttribute("aria-pressed", "true");
    }
  });

  it("replaces the response-length slider with a ladder plus a custom entry", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    // Scoped to the response-length field: the form also renders the context
    // ladder, which legitimately offers 2M. An unscoped query would be asking
    // whether 2M appears anywhere on the page, which is a different question.
    const lengthField = screen.getByText("agents.form.max_tokens").closest("div") as HTMLElement;

    // The output ladder stops at 128K. 1M / 2M are context figures, and no
    // model emits a million tokens of reply.
    for (const label of ["1K", "4K", "8K", "16K", "32K", "64K", "128K"]) {
      expect(within(lengthField).getByRole("button", { name: label })).toBeInTheDocument();
    }
    expect(within(lengthField).queryByRole("button", { name: "2M" })).not.toBeInTheDocument();
    expect(within(lengthField).queryByRole("button", { name: "1M" })).not.toBeInTheDocument();

    await user.click(within(lengthField).getByRole("button", { name: "8K" }));
    expect(
      within(lengthField).getByRole("button", { name: "8K", pressed: true }),
    ).toBeInTheDocument();
  });

  it("offers the context ladder up to 2M, which the output ladder must not", () => {
    render(<Harness />);
    const contextField = screen
      .getByText("agents.form.context_window")
      .closest("div") as HTMLElement;
    expect(within(contextField).getByRole("button", { name: "2M" })).toBeInTheDocument();
    expect(within(contextField).getByRole("button", { name: "1M" })).toBeInTheDocument();
  });

  it("opens a custom field for a value that is not on the ladder", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    const customButtons = screen.getAllByRole("button", { name: "agents.form.custom" });
    await user.click(customButtons[0]);

    const field = screen.getByRole("spinbutton", {
      name: "agents.form.max_tokens — agents.form.custom",
    });
    await user.clear(field);
    await user.type(field, "50000");
    expect(field).toHaveValue(50000);
  });

  /**
   * Warn, do not clamp. A silent truncation leaves the operator debugging a
   * number they never chose — worse than an explicit provider error when the
   * catalog figure is the thing that is wrong.
   */
  it("flags an over-limit response length without changing it", async () => {
    const state = emptyManifestForm();
    state.model.provider = "openai";
    state.model.model = "gpt-4o";
    state.model.max_tokens = "65536";

    render(
      <Harness
        initialState={state}
        models={[
          {
            provider: "openai",
            id: "gpt-4o",
            context_window: 200_000,
            max_output_tokens: 16_384,
            limits_known: true,
          },
        ]}
      />,
    );

    expect(screen.getByText(/agents\.form\.over_limit_warning/)).toBeInTheDocument();
    // The value is untouched, and the field is not marked invalid.
    expect(screen.getByRole("button", { name: "agents.form.custom", pressed: true })).toBeInTheDocument();
  });

  /**
   * An inferred limit is a guess, not a ceiling. Warning against one is noise,
   * and noise is what makes operators stop reading warnings (#7780).
   */
  it("stays silent when the model's limits were never sourced", () => {
    const state = emptyManifestForm();
    state.model.provider = "openai";
    state.model.model = "gpt-4o";
    state.model.max_tokens = "65536";

    render(
      <Harness
        initialState={state}
        models={[
          {
            provider: "openai",
            id: "gpt-4o",
            context_window: 131_072,
            max_output_tokens: 16_384,
            limits_known: false,
          },
        ]}
      />,
    );

    expect(screen.queryByText(/agents\.form\.over_limit_warning/)).not.toBeInTheDocument();
  });

  it("hides ladder rungs above a limit the model actually declared", () => {
    const state = emptyManifestForm();
    state.model.provider = "openai";
    state.model.model = "gpt-4o";

    render(
      <Harness
        initialState={state}
        models={[
          {
            provider: "openai",
            id: "gpt-4o",
            context_window: 200_000,
            max_output_tokens: 16_384,
            limits_known: true,
          },
        ]}
      />,
    );

    const lengthField = screen.getByText("agents.form.max_tokens").closest("div") as HTMLElement;
    expect(within(lengthField).getByRole("button", { name: "16K" })).toBeInTheDocument();
    expect(within(lengthField).queryByRole("button", { name: "32K" })).not.toBeInTheDocument();
  });
});
