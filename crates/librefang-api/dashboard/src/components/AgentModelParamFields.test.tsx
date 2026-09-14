import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { AgentModelParamFields } from "./AgentModelParamFields";
import {
  buildModelConfigPatch,
  emptyModelNumerics,
  MODEL_NUMERIC_FIELDS,
  type ModelNumericField,
} from "../lib/agentModelPatch";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: unknown) =>
      opts && typeof opts === "object" && "limit" in (opts as Record<string, unknown>)
        ? `${key}:${(opts as { limit: string }).limit}`
        : key,
    i18n: { language: "en" },
  }),
}));

function renderFields(
  over: Partial<Parameters<typeof AgentModelParamFields>[0]> = {},
): { onChange: ReturnType<typeof vi.fn> } {
  const onChange = vi.fn();
  render(
    <AgentModelParamFields
      draft={emptyModelNumerics()}
      onChange={onChange}
      isHand={false}
      {...over}
    />,
  );
  return { onChange };
}

/** The rung buttons for one parameter, found through its own labelled group. */
function group(param: ModelNumericField): HTMLElement {
  return screen.getByRole("group", { name: `model_param.${param}` });
}

describe("AgentModelParamFields", () => {
  it("routes a rung click to the parameter it belongs to", () => {
    // The failure this guards is a copy-paste one: a handler that writes a
    // neighbouring key. Nothing else renders the drawer, so it would ship green.
    const { onChange } = renderFields();

    within(group("top_p")).getByRole("button", { name: "0.9" }).click();

    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith("top_p", "0.9");
  });

  it("carries that click through the patch builder as the value the operator picked", () => {
    // The other half of the wiring: what the click writes into the draft is
    // what reaches `PATCH /api/agents/{id}/config`.
    const { onChange } = renderFields();

    within(group("top_p")).getByRole("button", { name: "0.9" }).click();
    const [field, next] = onChange.mock.calls[0] as [ModelNumericField, string];
    const draft = {
      provider: "anthropic",
      model: "claude-sonnet",
      ...emptyModelNumerics(),
      [field]: next,
    };

    expect(
      buildModelConfigPatch(draft, { provider: "anthropic", model: "claude-sonnet" }).patch,
    ).toEqual({ top_p: 0.9 });
  });

  it("gives every parameter its own control", () => {
    renderFields();
    for (const param of MODEL_NUMERIC_FIELDS) {
      expect(group(param), param).toBeInTheDocument();
    }
  });

  it("offers a hand agent only the two parameters its write path stores", () => {
    // `patch_hand_agent_runtime_config` maps max_tokens and temperature into
    // `HandAgentRuntimeOverride` and silently drops the rest at 200 OK, so
    // rendering them would be an edit box for a value that cannot be saved.
    renderFields({ isHand: true });

    expect(group("temperature")).toBeInTheDocument();
    expect(group("max_tokens")).toBeInTheDocument();
    for (const param of [
      "top_p",
      "frequency_penalty",
      "presence_penalty",
      "context_window",
      "max_output_tokens",
    ] as const) {
      expect(
        screen.queryByRole("group", { name: `model_param.${param}` }),
        param,
      ).not.toBeInTheDocument();
    }
  });

  it("warns when max_tokens passes the model's declared output cap", () => {
    renderFields({
      draft: { ...emptyModelNumerics(), max_tokens: "99000" },
      limits: { maxOutputTokens: 8192 },
    });

    expect(screen.getByText("agents.form.over_limit_warning:8K")).toBeInTheDocument();
  });

  it("measures max_tokens against an operator-set output cap ahead of the catalog's", () => {
    // The operator is the one who knows their endpoint serves less than the
    // catalog claims, so their number outranks it — and 99000 is under it.
    renderFields({
      draft: { ...emptyModelNumerics(), max_tokens: "99000", max_output_tokens: "128000" },
      limits: { maxOutputTokens: 8192 },
    });

    expect(screen.queryByText(/over_limit_warning/)).not.toBeInTheDocument();
  });

  it("says nothing about a limit the catalog never measured", () => {
    // An unknown limit is not a ceiling (#7780); warning against an invented
    // one trains operators to ignore warnings.
    renderFields({ draft: { ...emptyModelNumerics(), max_tokens: "99000" } });

    expect(screen.queryByText(/over_limit_warning/)).not.toBeInTheDocument();
  });
});
