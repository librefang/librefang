import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, it, expect, vi } from "vitest";
import {
  AgentManifestForm,
  type ManifestCatalogEntry,
  type ManifestSectionId,
  MANIFEST_SECTION_IDS,
  sectionForInvalidField,
} from "./AgentManifestForm";
import {
  emptyManifestExtras,
  emptyManifestForm,
  parseManifestToml,
  serializeManifestForm,
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
  routerProfileCatalog,
  routerProfilesEnabled,
  initialState,
  invalidFields = new Set(),
  models = [{ provider: "openai", id: "gpt-4o" }],
  providers = [{ name: "openai" }],
  nameField,
  sections,
  onState,
}: {
  skillCatalog?: ManifestCatalogEntry[];
  toolCatalog?: ManifestCatalogEntry[];
  mcpCatalog?: ManifestCatalogEntry[];
  routerProfileCatalog?: ManifestCatalogEntry[];
  routerProfilesEnabled?: boolean;
  initialState?: ManifestFormState;
  invalidFields?: Set<string>;
  models?: HarnessModel[];
  providers?: { name: string }[];
  nameField?: "editable" | "readonly" | "hidden";
  sections?: ManifestSectionId[];
  /** Receives every state the form produces, so a test can read what would be saved. */
  onState?: (next: ManifestFormState) => void;
}) {
  const [state, setState] = useState<ManifestFormState>(() => initialState ?? emptyManifestForm());
  return (
    <AgentManifestForm
      value={state}
      onChange={(next) => {
        setState(next);
        onState?.(next);
      }}
      providers={providers}
      models={models}
      invalidFields={invalidFields}
      extras={emptyManifestExtras()}
      skillCatalog={skillCatalog}
      toolCatalog={toolCatalog}
      mcpCatalog={mcpCatalog}
      routerProfileCatalog={routerProfileCatalog}
      routerProfilesEnabled={routerProfilesEnabled}
      nameField={nameField}
      sections={sections}
    />
  );
}

describe("AgentManifestForm — complexity routing tiers", () => {
  const MODELS = [
    { provider: "openai", id: "gpt-4o" },
    { provider: "anthropic", id: "claude-sonnet-5" },
  ];

  async function openRouting(user: ReturnType<typeof userEvent.setup>) {
    await user.click(screen.getByText("agents.form.routing"));
    await user.click(screen.getByLabelText("agents.form.routing_enabled"));
  }

  // The tier fields hold a bare model name — the daemon resolves it against the
  // global catalog via `ModelCatalog::find_model`, so a `provider/model` string
  // would not resolve. The picker speaks in pairs, and the adapter between the
  // two is exactly where a provider could leak into the stored value.
  it("stores the model name alone when a tier is picked", async () => {
    const user = userEvent.setup();
    render(<Harness models={MODELS} />);
    await openRouting(user);

    await user.click(screen.getByRole("button", { name: "agents.form.simple_model: None" }));
    await user.click(screen.getByRole("button", { name: "anthropic/claude-sonnet-5" }));

    // The trigger reads back from the form state, so this fails if either the
    // provider leaked in or the name never reached `simple_model`.
    expect(
      screen.getByRole("button", { name: "agents.form.simple_model: claude-sonnet-5" }),
    ).toBeInTheDocument();
  });

  it("offers every model in one flat list, with no provider step", async () => {
    const user = userEvent.setup();
    render(<Harness models={MODELS} />);
    await openRouting(user);

    await user.click(screen.getByRole("button", { name: "agents.form.medium_model: None" }));

    // Both providers' models are reachable without drilling in, which is the
    // point of the flat shape for a field that cannot hold a provider.
    expect(screen.getByRole("button", { name: "openai/gpt-4o" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "anthropic/claude-sonnet-5" })).toBeInTheDocument();
  });

  it("accepts a model the catalog has never seen", async () => {
    const user = userEvent.setup();
    render(<Harness models={MODELS} />);
    await openRouting(user);

    await user.click(screen.getByRole("button", { name: "agents.form.complex_model: None" }));
    await user.click(screen.getByRole("button", { name: "Custom" }));
    // No provider field in this shape: requiring one would make a valid entry
    // impossible to commit.
    await user.type(screen.getByLabelText("Model"), "llama-3.3-70b");
    await user.click(screen.getByRole("button", { name: "Confirm" }));

    expect(
      screen.getByRole("button", { name: "agents.form.complex_model: llama-3.3-70b" }),
    ).toBeInTheDocument();
  });
});

describe("AgentManifestForm — provider selection", () => {
  // The caller passes only providers that can serve a request, so an agent
  // assigned to one whose key was rejected (or whose local service is down) is
  // not in that list. The control has to offer it anyway: an operator who
  // cannot see the provider their agent runs on cannot change the model
  // without first moving the agent somewhere it is not.
  //
  // This is deliberately asserted here rather than left to the picker's own
  // suite. The picker only knows the list it is handed; adding the current
  // provider back is `providerOptions`, which is this component's job.
  async function openModelPicker() {
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /^agents\.form\.model:/ }));
    return user;
  }

  it("offers the provider the agent already uses even when it is not selectable anew", async () => {
    const state = emptyManifestForm();
    state.model = { ...state.model, provider: "deepseek", model: "deepseek-chat" };

    render(<Harness initialState={state} providers={[{ name: "openai" }]} />);
    await openModelPicker();

    expect(screen.getByRole("button", { name: "deepseek" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "openai" })).toBeInTheDocument();
  });

  it("does not list a current provider that is already offered, twice", async () => {
    const state = emptyManifestForm();
    state.model = { ...state.model, provider: "openai", model: "gpt-4o" };

    render(<Harness initialState={state} providers={[{ name: "openai" }]} />);
    await openModelPicker();

    expect(screen.getAllByRole("button", { name: "openai" })).toHaveLength(1);
  });
});

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

  it("opens the (defaultOpen=false) Shared Folders section and reddens its title on a validation error (#8013)", () => {
    const state = emptyManifestForm();
    state.workspaces.push({ _uid: "w1", name: "shared", path: "../escape", mode: "rw" });

    render(
      <Harness initialState={state} invalidFields={new Set(["workspaces.w1.path"])} />,
    );

    const pathInput = screen.getByPlaceholderText("agents.form.folder_path");
    expect(pathInput).toHaveAttribute("aria-invalid", "true");
    expect(pathInput.closest("details")).toHaveAttribute("open");
    expect(pathInput.closest("details")?.querySelector("summary")).toHaveAttribute(
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
  it("lets the agent set every sampling preference on the shared ladder", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    for (const label of [
      "model_param.temperature",
      "model_param.top_p",
      "model_param.frequency_penalty",
      "model_param.presence_penalty",
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }

    // The control is shared; the rungs are not. A sampling parameter's ladder
    // carries its own values, so a token count appearing here would mean the
    // shared object had been handed the wrong ladder.
    const tempField = screen.getByText("model_param.temperature").closest("div") as HTMLElement;
    for (const rung of ["0", "0.2", "0.5", "0.7", "1", "1.5", "2"]) {
      expect(within(tempField).getByRole("button", { name: rung })).toBeInTheDocument();
    }
    expect(within(tempField).queryByRole("button", { name: "8K" })).not.toBeInTheDocument();

    const topPField = screen.getByText("model_param.top_p").closest("div") as HTMLElement;
    await user.click(within(topPField).getByRole("button", { name: "0.9" }));
    expect(
      within(topPField).getByRole("button", { name: "0.9", pressed: true }),
    ).toBeInTheDocument();
  });

  it("starts every knob on inherit rather than on a number nobody chose", () => {
    render(<Harness />);
    // Every parameter the form sets, token counts and sampling alike, lands on
    // the inherit rung — the agent states no opinion until someone gives it one.
    for (const param of [
      "context_window",
      "max_tokens",
      "temperature",
      "top_p",
      "frequency_penalty",
      "presence_penalty",
    ]) {
      const field = screen.getByText(`model_param.${param}`).closest("div") as HTMLElement;
      expect(within(field).getByRole("button", { name: "model_param.inherit" })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
    }
  });

  it("replaces the response-length slider with a ladder plus a custom entry", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    // Scoped to the response-length field: the form also renders the context
    // ladder, which legitimately offers 2M. An unscoped query would be asking
    // whether 2M appears anywhere on the page, which is a different question.
    const lengthField = screen.getByText("model_param.max_tokens").closest("div") as HTMLElement;

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
      .getByText("model_param.context_window")
      .closest("div") as HTMLElement;
    expect(within(contextField).getByRole("button", { name: "2M" })).toBeInTheDocument();
    expect(within(contextField).getByRole("button", { name: "1M" })).toBeInTheDocument();
  });

  it("opens a custom field for a value that is not on the ladder", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    // Scoped to the response-length field: the form now renders seven ladders,
    // so an unscoped "first custom button" is whichever one the layout happens
    // to put first.
    const lengthField = screen.getByText("model_param.max_tokens").closest("div") as HTMLElement;
    await user.click(within(lengthField).getByRole("button", { name: "model_param.custom" }));

    const field = within(lengthField).getByRole("spinbutton", {
      name: "model_param.max_tokens — model_param.custom",
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
    expect(screen.getByRole("button", { name: "model_param.custom", pressed: true })).toBeInTheDocument();
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

    const lengthField = screen.getByText("model_param.max_tokens").closest("div") as HTMLElement;
    expect(within(lengthField).getByRole("button", { name: "16K" })).toBeInTheDocument();
    expect(within(lengthField).queryByRole("button", { name: "32K" })).not.toBeInTheDocument();
  });
});

// #8028: the agent-type editor drives its own Name input (create) or pins
// identity to a URL segment (edit), and either way this form's own Name
// field must not offer a second, disagreeing way to set it.
describe("AgentManifestForm — nameField", () => {
  it("renders an editable Name field by default", () => {
    render(<Harness />);
    expect(screen.getByRole("textbox", { name: "agents.form.name" })).toBeEnabled();
  });

  it("hides the Name field entirely when nameField is 'hidden'", () => {
    render(<Harness nameField="hidden" />);
    expect(screen.queryByRole("textbox", { name: "agents.form.name" })).not.toBeInTheDocument();
  });

  it("renders the Name field disabled when nameField is 'readonly', pre-filled from the manifest", () => {
    const state = emptyManifestForm();
    state.name = "existing-type";
    render(<Harness initialState={state} nameField="readonly" />);

    const input = screen.getByRole("textbox", { name: "agents.form.name" });
    expect(input).toBeDisabled();
    expect(input).toHaveValue("existing-type");
  });
});

// The agent drawer hosts this form *inside* its tabs, so the same editor
// backs "Conversation", "Routing", "Tools" … rather than living in a second
// "Edit full configuration" drawer. That only works if a caller can name the
// sections it wants, and if naming a subset actually drops the rest — an
// ignored `sections` prop would render the whole manifest on every tab and
// look, to a reader, exactly like the feature working.
describe("AgentManifestForm — section addressing", () => {
  const renderedSections = (container: HTMLElement): string[] =>
    Array.from(container.querySelectorAll("[data-section]")).map(
      (el) => el.getAttribute("data-section") ?? "",
    );

  it("renders every section when the caller passes no list", () => {
    const { container } = render(<Harness />);
    expect(renderedSections(container)).toEqual([...MANIFEST_SECTION_IDS]);
  });

  it("renders only the sections the caller asked for", () => {
    const { container } = render(<Harness sections={["routing"]} />);
    expect(renderedSections(container)).toEqual(["routing"]);
  });

  it("renders one section per tab set, in the order asked", () => {
    // The Routing tab. Each of these used to sit elsewhere: the tiers were
    // two drawers deep and the fallback chain was its own collapsed block in
    // the other surface.
    const routingTab: ManifestSectionId[] = [
      "model",
      "fallback_models",
      "thinking",
      "routing",
    ];
    const { container } = render(<Harness sections={routingTab} />);
    expect(renderedSections(container)).toEqual(routingTab);
  });

  it("renders nothing, not everything, for an empty list", () => {
    // The failure mode worth guarding: an empty array is falsy-ish in the
    // places a caller might spread it, and falling back to "all sections"
    // would put the entire manifest on a tab that asked for none of it.
    const { container } = render(<Harness sections={[]} />);
    expect(renderedSections(container)).toEqual([]);
  });
});

// A validation message that names a field on a tab the operator is not
// looking at is indistinguishable from no message at all, and with the
// sections split across tabs that became possible for the first time.
// `sectionForInvalidField` is what lets the caller jump to the right tab, and
// it is only correct while it covers everything the validator can report.
describe("AgentManifestForm — validation paths are routable", () => {
  it("maps every field path validateManifestForm can report to a section", () => {
    const source = readFileSync(
      join(__dirname, "..", "lib", "agentManifest.ts"),
      "utf8",
    );
    // Every `errors.push("…")` in the validator. Template literals included:
    // `workspaces.${ws._uid}.name` is captured with its placeholder intact,
    // which still matches the `workspaces.` prefix.
    const reported = [...source.matchAll(/errors\.push\(\s*["`]([^"`]+)["`]/g)].map(
      (m) => m[1],
    );

    expect(reported.length).toBeGreaterThan(0);

    const unrouted = reported.filter((path) => sectionForInvalidField(path) === undefined);
    expect(
      unrouted,
      `validateManifestForm reports field paths that no section claims, so an ` +
        `operator who trips one would be told to fix a field the editor cannot ` +
        `navigate to. Add the prefix to FIELD_PREFIX_TO_SECTION.\n\n` +
        `Unrouted: ${unrouted.join(", ")}`,
    ).toEqual([]);
  });
});

describe("AgentManifestForm — quantity ladders", () => {
  // The ladders replaced bare number boxes, and a number box carries
  // constraints the ladder's custom rung has to keep carrying.
  //
  // The dollar fields are the ones where dropping them is invisible: an unset
  // `step` defaults to 1, so `min={0}` alone makes 0.50 a step mismatch and the
  // browser marks the input invalid — a legitimate half-dollar cap that the
  // form would refuse to submit, with the reason visible only to the browser.
  it("keeps the custom cost box able to accept cents", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    const group = screen.getByRole("group", { name: "agents.form.cost_per_hour" });
    await user.click(within(group).getByRole("button", { name: "model_param.custom" }));

    // The custom box is a sibling of the `role="group"` div, not a child of
    // it, so it is reached by its own label rather than through `within`.
    const input = screen.getByRole("spinbutton", {
      name: "agents.form.cost_per_hour — model_param.custom",
    });
    expect(input).toHaveAttribute("step", "0.01");
    expect(input).toHaveAttribute("min", "0");
  });

  // Each converted field must still offer the inherit state, which is what an
  // empty value means and what the manifest writes when the agent has no
  // opinion. A ladder without it would force every agent to state a number.
  it("offers the inherit rung on a converted quantity", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    // Two gates: the fields live in a collapsed section, and they only render
    // once autonomy is on — a disabled feature's tuning knobs are not shown.
    await user.click(screen.getByText("agents.form.autonomous"));
    await user.click(screen.getByLabelText("agents.form.autonomous_enabled"));

    const group = screen.getByRole("group", { name: "agents.form.max_iterations" });
    expect(
      within(group).getByRole("button", { name: "model_param.inherit" }),
    ).toBeInTheDocument();
  });
});

// Swapping a control is only safe if the thing it writes is unchanged. The
// provider+model `<select>` pair stored `[model] provider` and `[model] model`
// as two keys; the picker hands back a pair. That they agree is not something
// the DOM can show — the trigger renders the pair either way — so this asserts
// what would actually be saved, and then reads it back.
//
// This is deliberately a write-then-read rather than a check of the captured
// state alone: a control that writes the right value to the wrong path passes
// any assertion made against the control's own output.
describe("AgentManifestForm — the model picker persists the pair it replaced", () => {
  it("writes provider and model where the selects wrote them, and reads them back", async () => {
    const user = userEvent.setup();
    let latest: ManifestFormState | null = null;
    render(<Harness onState={(next) => { latest = next; }} />);

    await user.click(screen.getByRole("button", { name: /^agents\.form\.model:/ }));
    await user.click(screen.getByRole("button", { name: "openai" }));
    await user.click(screen.getByRole("button", { name: "openai/gpt-4o" }));

    expect(latest).not.toBeNull();
    const form = latest as unknown as ManifestFormState;
    expect(form.model.provider).toBe("openai");
    expect(form.model.model).toBe("gpt-4o");

    // What the daemon reads.
    const toml = serializeManifestForm(form);
    expect(toml).toContain("provider = \"openai\"");
    expect(toml).toContain("model = \"gpt-4o\"");

    // And what comes back when the same manifest is opened again.
    const parsed = parseManifestToml(toml);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.model.provider).toBe("openai");
    expect(parsed.form.model.model).toBe("gpt-4o");
  });
});

// A field marked invalid without a reason tells the operator that something is
// wrong and not what — which is the half of the message that does not help, and
// it is what makes the tab jump land on something that still does not explain
// itself. The cron and JSON-schema paths already carried `error=`; these are the
// ones that did not.
describe("AgentManifestForm — a marked field says why", () => {
  it("explains a missing name, and associates it with the input", () => {
    const state = emptyManifestForm();
    state.name = "";

    render(<Harness initialState={state} invalidFields={new Set(["name"])} />);

    const input = screen.getByRole("textbox", { name: "agents.form.name" });
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(input).toHaveAccessibleDescription("agents.form.name_required");
  });

  it("names the range when a sampling parameter is out of it", () => {
    const state = emptyManifestForm();
    state.model.temperature = "9";

    render(
      <Harness initialState={state} invalidFields={new Set(["model.temperature"])} />,
    );

    // The message states the bounds rather than only marking the control, and
    // the bounds come from the same table the ladder uses, so the two cannot
    // disagree about what is allowed.
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent(/between/);
  });
});

// A component declared inside a render body is a new function on every render,
// and React compares element types by reference — so it unmounts and remounts
// its whole subtree on every keystroke.
//
// The two cases below are a pair on purpose: the module-scope `Section` and the
// section wrapper must both survive a re-render, so a regression in either is
// visible. A test on one alone cannot tell a wrapper-wide defect from a
// field-specific one — and that is precisely how this one read, because the
// seven sections using the module-scope `Section` never had it.
//
// What it cost: a controlled input inside one of the twelve wrapper sections
// kept only the first character typed into it (the element the second keystroke
// was headed for had already been destroyed), and an open `<select>` was
// dismissed under the operator by any re-render.
describe("AgentManifestForm — a re-render must not remount the fields", () => {
  it("keeps every character typed into a section field", async () => {
    const user = userEvent.setup();
    let latest: ManifestFormState | null = null;
    render(
      <Harness
        sections={["proactive_memory"]}
        onState={(n) => {
          latest = n;
        }}
      />,
    );

    await user.click(screen.getByText("config.sec_proactive_memory"));
    await user.type(screen.getByRole("textbox"), "abc");

    // `"a"` is what a remount produces: the remaining keystrokes go to a node
    // that is no longer in the document.
    expect(
      (latest as unknown as ManifestFormState | null)?.proactive_memory.extraction_model,
    ).toBe("abc");
  });

  it("keeps a select's DOM node across a re-render, so an open dropdown is not dismissed", async () => {
    const user = userEvent.setup();
    render(<Harness sections={["proactive_memory"]} onState={() => {}} />);

    await user.click(screen.getByText("config.sec_proactive_memory"));
    const before = screen.getAllByRole("combobox")[0];

    await user.type(screen.getByRole("textbox"), "x");

    expect(screen.getAllByRole("combobox")[0]).toBe(before);
  });
});

describe("AgentManifestForm — the counters report before the server does", () => {
  it("names the rule on a counter that is not a whole number", () => {
    const state = emptyManifestForm();
    state.max_history_messages = "1.5";

    render(
      <Harness
        initialState={state}
        invalidFields={new Set(["max_history_messages"])}
        sections={["lifecycle"]}
      />,
    );

    // A red label with no text is the shape review already rejected once: it
    // tells the operator a field is wrong and not what would make it right.
    expect(screen.getByRole("alert")).toHaveTextContent(
      "agents.form.whole_number_required",
    );
  });
});

// The user's brief for the unified editor: "everything in the same place, with
// an ADVANCED button to give each section more depth, and in BASIC mode each
// section shows what matters most." The basic/advanced split is per section:
// the fields an operator sets first render always, and everything beyond them
// folds behind the section's own Advanced disclosure — the same details/sum-
// mary mechanism CollapsibleSection uses for the section itself.
/**
 * The Advanced disclosure inside one section, addressed by section id.
 * File scope: more describes than the fold's own need to reach it.
 */
function advancedGroup(sectionId: string): HTMLDetailsElement | null {
  const section = document.querySelector(`[data-section="${sectionId}"]`);
  if (!section) return null;
  const summary = Array.from(section.querySelectorAll("summary")).find(
    // The harness's i18n stub echoes the key; the advanced group's summary
    // is the only one this component renders itself.
    (s) => s.textContent === "agents.form.advanced",
  );
  return summary ? (summary.closest("details") as HTMLDetailsElement) : null;
}

describe("AgentManifestForm — basic and advanced per section", () => {
  it("keeps the model section basic: the sampling knobs fold behind Advanced", () => {
    render(<Harness />);
    const group = advancedGroup("model");
    expect(group, "the model section has an Advanced disclosure").toBeTruthy();
    // BASIC: closed. The temperature field exists in the DOM (a closed
    // details still contains its children) but the disclosure says so.
    expect(group).not.toHaveAttribute("open");
    expect(group!.querySelector("input")).toBeTruthy();
  });

  it("opens the advanced group when a validation error lands inside it", () => {
    // A hidden error reads as no error — the same rule CollapsibleSection
    // applies to a folded section, one level down.
    render(<Harness invalidFields={new Set(["model.temperature"])} />);
    expect(advancedGroup("model")).toHaveAttribute("open");
  });

  it("opens and closes on its own summary, like any details", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const group = advancedGroup("model");
    expect(group).not.toHaveAttribute("open");
    await user.click(group!.querySelector("summary")!);
    expect(group).toHaveAttribute("open");
    await user.click(group!.querySelector("summary")!);
    expect(group).not.toHaveAttribute("open");
  });

  it("folds the identity extras behind Advanced too", () => {
    render(<Harness />);
    const group = advancedGroup("identity");
    expect(group).toBeTruthy();
    expect(group).not.toHaveAttribute("open");
    // Name stays outside the fold: it is the field the section is about.
    expect(
      group!.querySelector('input[aria-label="agents.form.name"]'),
    ).toBeNull();
  });

  it("hides nothing when the caller never asked for a split section", () => {
    // A section list without `model` renders no model section at all; the
    // split must not break a caller that hosts other sections.
    render(<Harness sections={["identity"]} />);
    expect(advancedGroup("identity")).toBeTruthy();
    expect(advancedGroup("model")).toBeNull();
  });
});


// ALTO 1, remedied: the routing panel was the only surface with the
// server-backed profile catalog, and the unified editor's allowed_profiles
// was a blind tag box — a capability loss, not a unification. The catalog
// comes in as a prop like every other catalog the form merges, and a name
// the catalog does not know is still typeable.
describe("AgentManifestForm — the router's profile picker", () => {
  const PROFILES: ManifestCatalogEntry[] = [
    { name: "coding", description: "openai/gpt-4o · cheap" },
    { name: "research", description: "anthropic/claude-sonnet-5 · expensive" },
  ];

  async function openRouterFields(user: ReturnType<typeof userEvent.setup>) {
    const group = advancedGroup("model");
    if (!group) throw new Error("model advanced group not found");
    await user.click(group.querySelector("summary")!);
  }

  it("offers the server-backed catalog for allowed_profiles", async () => {
    const user = userEvent.setup();
    render(<Harness routerProfileCatalog={PROFILES} />);
    await openRouterFields(user);
    const group = advancedGroup("model")!;

    const input = within(group).getByPlaceholderText("Search model profiles…");
    await user.click(input);
    // Scoped to the section: the routing tab's tool-profile select offers a
    // `coding` option of its own, and a closed details still queries in jsdom.
    expect(await within(group).findByText("coding")).toBeInTheDocument();
    expect(within(group).getByText("research")).toBeInTheDocument();

    await user.click(within(group).getByText("coding"));
    expect(screen.getByRole("button", { name: "Remove coding" })).toBeInTheDocument();
  });

  it("still accepts a profile the catalog does not know", async () => {
    const user = userEvent.setup();
    render(
      <Harness
        routerProfileCatalog={PROFILES}
        initialState={(() => {
          const s = emptyManifestForm();
          s.model.router_allowed_profiles = ["hand-written-profile"];
          return s;
        })()}
      />,
    );
    await openRouterFields(user);

    // The union merge keeps a hand-typed profile selectable, the way the
    // skill finder keeps an uninstalled skill.
    const group = advancedGroup("model")!;
    expect(screen.getByRole("button", { name: "Remove hand-written-profile" })).toBeInTheDocument();
    // With chips present the finder's placeholder is the add-more one (the
    // stub resolves this key's defaultValue).
    const input = within(group).getByPlaceholderText("Add more…");
    await user.click(input);
    expect(await within(group).findByText("hand-written-profile")).toBeInTheDocument();
  });

  it("keeps the plain tag box when the caller carries no catalog", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await openRouterFields(user);

    // No catalog prop, no picker — the TagInput the field had before, so a
    // caller without the profiles query loses nothing it ever had. (The
    // harness's i18n stub echoes keys, so the tag box carries the key.)
    expect(screen.queryByPlaceholderText("Search model profiles…")).not.toBeInTheDocument();
    expect(
      screen.getByPlaceholderText("agents.form.router_allowed_profiles_placeholder"),
    ).toBeInTheDocument();
  });

  it("says when the router is off kernel-wide, instead of offering choices silently", async () => {
    const user = userEvent.setup();
    render(<Harness routerProfileCatalog={PROFILES} routerProfilesEnabled={false} />);
    await openRouterFields(user);

    expect(screen.getByText("agents.form.router_kernel_off")).toBeInTheDocument();
  });
});

// The skill workshop's FormSection sat nested inside compaction's — a rebase
// artifact, not a design. The drawer's tabs made it worse than cosmetic: the
// memory tab hosts compaction without workshop and the skills tab hosts
// workshop without compaction, so the shows guard zeroed the outer section
// on both tabs and the workshop rendered NOWHERE. Unnested, each section
// appears on the tab that hosts it and nowhere else.
describe("AgentManifestForm — the workshop is not nested inside compaction", () => {
  it("renders skill_workshop on the skills tab, which hosts it", () => {
    render(<Harness sections={["skills", "skill_workshop"]} />);

    const section = document.querySelector('[data-section="skill_workshop"]');
    expect(section, "the skills tab hosts skill_workshop, so it renders").toBeTruthy();
    expect(
      within(section as HTMLElement).getByText("agents.form.skill_workshop_enabled"),
    ).toBeInTheDocument();
    // And compaction stays off the tab that does not host it.
    expect(document.querySelector('[data-section="compaction"]')).toBeNull();
  });

  it("renders compaction on the memory tab without carrying the workshop in it", () => {
    render(<Harness sections={["proactive_memory", "auto_dream", "compaction"]} />);

    const section = document.querySelector('[data-section="compaction"]');
    expect(section).toBeTruthy();
    expect(
      within(section as HTMLElement).queryByText("agents.form.skill_workshop_enabled"),
    ).toBeNull();
    expect(document.querySelector('[data-section="skill_workshop"]')).toBeNull();
  });
});

// The inherit-the-deployment-default capability, which the unified editor lost
// when the drawer's model editor went: it used to send `provider = "default"`
// and the daemon resolves that (and an empty provider) to whatever the kernel
// boots with (`kernel/llm_drivers.rs:178`). Without a way back, an agent pinned
// to one model could never be unpinned — the picker cannot emit an empty
// commit, so the only route was hand-editing `agent.toml`.
//
// Driven through the serializer as well as the control: `provider = "default"`
// is what the daemon reads, and a control that set the state without the
// serializer carrying it would be a capability that looks restored and is not.
describe("AgentManifestForm — inheriting the deployment default model", () => {
  const pinned = () => {
    const form = emptyManifestForm();
    form.name = "pinned-agent";
    form.model.provider = "anthropic";
    form.model.model = "claude-sonnet-5";
    return form;
  };

  // Matched by its English label, not the key: this file's i18n mock resolves
  // `defaultValue`, which is what the control ships to an operator.
  const resetControl = () =>
    screen.getByRole("button", { name: /use global default/i });

  it("offers a way back to the global default from a pinned model", () => {
    render(<Harness initialState={pinned()} />);
    expect(resetControl()).toBeInTheDocument();
  });

  it("resets the pair to the sentinel the daemon resolves", async () => {
    const user = userEvent.setup();
    let latest: ManifestFormState | null = null;
    render(<Harness initialState={pinned()} onState={(next) => { latest = next; }} />);

    await user.click(resetControl());

    expect(latest).not.toBeNull();
    expect(latest!.model.provider).toBe("default");
    expect(latest!.model.model).toBe("default");

    // And the file the daemon reads carries it.
    const toml = serializeManifestForm(latest!, emptyManifestExtras());
    expect(toml).toContain('provider = "default"');
    expect(toml).toContain('model = "default"');
  });
});

// A validation error inside a folded section has to open that section. The
// inner `AdvancedFields` opens its own fold and `Field` marks the control, but
// the section's outer `<details>` is the one the operator has to see through:
// with it closed the marked field is `aria-invalid=1` and invisible, which is
// the same failure the group jump in AgentsPage exists to prevent one level up.
//
// Every section that can host one of the validator's paths is listed, not just
// the five that were broken — the three that worked would have caught a
// regression here, and they are the shape the others had to copy.
describe("AgentManifestForm — a validation error opens its folded section", () => {
  const cases: Array<[string, string[]]> = [
    ["scheduling", ["schedule.cron"]],
    ["response_format", ["response_format.schema"]],
    ["autonomous", ["autonomous.heartbeat_timeout_secs"]],
    ["compaction", ["compaction.max_retries"]],
    ["compaction", ["compaction.max_loop_steps_before_aggregate"]],
    ["compaction", ["compaction.strip_reasoning_after_turns"]],
    ["lifecycle", ["max_history_messages"]],
    ["lifecycle", ["max_concurrent_invocations"]],
    ["auto_dream", ["auto_dream_min_sessions"]],
    ["skill_workshop", ["skill_workshop.max_pending_age_days"]],
  ];

  it.each(cases)("opens %s for %s", (section, path) => {
    // `new Set(path)`, not `new Set([path])`: the case's second element is
    // already the list of paths, and wrapping it again puts an array in the
    // set, which `has()` then answers false for — every section reads closed
    // and the test looks like it is finding a real defect.
    render(<Harness invalidFields={new Set(path)} />);

    const details = document.querySelector(`[data-section="${section}"]`);
    expect(details, `no section rendered for ${section}`).toBeTruthy();
    expect((details as HTMLDetailsElement).open).toBe(true);
  });

  // `shared_folders` keys its errors by row, so the section opens when any of
  // its rendered workspaces is the one complained about.
  it("opens shared_folders for the row the error names", () => {
    const form = emptyManifestForm();
    form.name = "an-agent";
    form.workspaces = [{ _uid: "u1", name: "docs", path: "", mode: "rw" }];
    render(<Harness initialState={form} invalidFields={new Set(["workspaces.u1.path"])} />);

    const details = document.querySelector('[data-section="shared_folders"]');
    expect(details).toBeTruthy();
    // `.open` is the element's own view of itself, which is what decides
    // whether the row is on screen. `toHaveAttribute("open")` answers the same
    // question here — React writes the attribute and the DOM reflects it — so
    // either spelling is fine; this one is the property the browser reads.
    expect((details as HTMLDetailsElement).open).toBe(true);
  });

  // And it does not open a section the error is not about: an always-open
  // section would defeat the point of folding the rest.
  it("leaves a section the error does not name closed", () => {
    render(<Harness invalidFields={new Set(["compaction.max_retries"])} />);

    expect(
      (document.querySelector('[data-section="skill_workshop"]') as HTMLDetailsElement).open,
    ).toBe(false);
  });
});
