import { beforeAll, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import type { ModelItem, ProviderItem } from "../../api";
import i18n, { i18nReady } from "../../lib/i18n";
import { ModelPicker } from "./ModelPicker";

// The component renders translated strings, so the singleton has to be up
// before any assertion that reads one. Most ui tests skip this and assert only
// on roles; this one needs the Back control, whose name *is* the translation.
beforeAll(async () => {
  await i18nReady;
});

const model = (provider: string, id: string, display_name?: string): ModelItem =>
  ({ provider, id, display_name }) as ModelItem;

const provider = (id: string, extra: Partial<ProviderItem> = {}): ProviderItem =>
  ({ id, ...extra }) as ProviderItem;

/** Open the popover with a click on the trigger. */
function open(label = "Agent model") {
  fireEvent.click(screen.getByRole("button", { name: new RegExp(`^${label}:`) }));
}

describe("ModelPicker", () => {
  it("reports the provider alongside the model, because the id alone is not unique", () => {
    const onChange = vi.fn();
    render(
      <ModelPicker
        label="Agent model"
        value={{ provider: "openai", model: "gpt-4" }}
        onChange={onChange}
        models={[model("anthropic", "claude-sonnet-5"), model("openai", "gpt-5")]}
        providers={[provider("anthropic"), provider("openai")]}
      />,
    );

    open();
    fireEvent.click(screen.getByRole("button", { name: "openai" }));
    fireEvent.click(screen.getByRole("button", { name: "openai/gpt-5" }));

    expect(onChange).toHaveBeenCalledWith({ provider: "openai", model: "gpt-5" });
  });

  it("lists providers alphabetically however the caller ordered them", () => {
    render(
      <ModelPicker
        label="Agent model"
        value={{ provider: "openai", model: "gpt-4" }}
        onChange={() => {}}
        models={[model("openai", "gpt-4"), model("anthropic", "claude-sonnet-5"), model("gemini", "flash-2")]}
        providers={[provider("openai"), provider("gemini"), provider("anthropic")]}
      />,
    );

    open();
    const ids = ["anthropic", "gemini", "openai"];
    const rows = screen
      .getAllByRole("button")
      .filter((b) => ids.includes(b.getAttribute("aria-label") ?? ""))
      .map((b) => b.getAttribute("aria-label"));

    expect(rows).toEqual(["anthropic", "gemini", "openai"]);
  });

  it("does not mark a model active when only the id matches, not the provider", () => {
    // `claude-sonnet-5` served by two providers is two different choices. A
    // check on the id alone would light up the wrong row — and, because the
    // click handler returns early on an active row, would make the other
    // provider's copy unselectable.
    const onChange = vi.fn();
    render(
      <ModelPicker
        label="Agent model"
        value={{ provider: "anthropic", model: "claude-sonnet-5" }}
        onChange={onChange}
        models={[model("anthropic", "claude-sonnet-5"), model("openai", "claude-sonnet-5")]}
        providers={[provider("anthropic"), provider("openai")]}
      />,
    );

    open();

    fireEvent.click(screen.getByRole("button", { name: "anthropic" }));
    expect(screen.getByRole("button", { name: "anthropic/claude-sonnet-5" })).toHaveAttribute(
      "aria-current",
      "true",
    );

    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    fireEvent.click(screen.getByRole("button", { name: "openai" }));
    const other = screen.getByRole("button", { name: "openai/claude-sonnet-5" });

    expect(other).not.toHaveAttribute("aria-current");
    fireEvent.click(other);
    expect(onChange).toHaveBeenCalledWith({ provider: "openai", model: "claude-sonnet-5" });
  });

  it("narrows the model list to the search term, on id or display name", () => {
    render(
      <ModelPicker
        label="Agent model"
        value={{ provider: "openai", model: "gpt-4" }}
        onChange={() => {}}
        models={[
          model("openai", "gpt-4"),
          model("openai", "o3-mini", "o3 Mini (fast)"),
          model("anthropic", "claude-sonnet-5"),
        ]}
        providers={[provider("openai"), provider("anthropic")]}
      />,
    );

    open();
    fireEvent.click(screen.getByRole("button", { name: "openai" }));

    const search = screen.getByRole("textbox");
    fireEvent.change(search, { target: { value: "mini" } });

    expect(screen.getByRole("button", { name: "openai/o3-mini" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "openai/gpt-4" })).not.toBeInTheDocument();
  });

  it("returns to the provider list from Back, dropping the search with it", () => {
    render(
      <ModelPicker
        label="Agent model"
        value={{ provider: "openai", model: "gpt-4" }}
        onChange={() => {}}
        models={[model("openai", "gpt-4"), model("anthropic", "claude-sonnet-5")]}
        providers={[provider("openai"), provider("anthropic")]}
      />,
    );

    open();
    fireEvent.click(screen.getByRole("button", { name: "openai" }));
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "gpt" } });
    fireEvent.click(screen.getByRole("button", { name: i18n.t("common.back") }));

    // Both providers are reachable again — a search term left behind would
    // have filtered the top level down to whatever matched "gpt".
    expect(screen.getByRole("button", { name: "anthropic" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "openai" })).toBeInTheDocument();
  });

  it("closes on Escape with the search box focused, restoring focus to the trigger", () => {
    render(
      <ModelPicker
        label="Agent model"
        value={{ provider: "openai", model: "gpt-4" }}
        onChange={() => {}}
        models={[model("openai", "gpt-4")]}
        providers={[provider("openai")]}
      />,
    );

    const trigger = screen.getByRole("button", { name: /^Agent model:/ });
    expect(trigger).toHaveAttribute("aria-expanded", "false");

    open();
    expect(trigger).toHaveAttribute("aria-expanded", "true");

    const search = screen.getByRole("textbox");
    search.focus();
    expect(search).toHaveFocus();

    // The handler is local and calls `preventDefault()`, so a `Modal` the
    // picker is nested in does not also close on the same key — it bails on
    // `defaultPrevented`. `fireEvent` returns false when default is prevented.
    expect(fireEvent.keyDown(search, { key: "Escape" })).toBe(false);
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    // Without the restore, focus falls to `<body>` when the popover unmounts.
    expect(trigger).toHaveFocus();
  });

  it("will not drill into a provider the catalog cannot serve", () => {
    render(
      <ModelPicker
        label="Agent model"
        value={null}
        onChange={() => {}}
        models={[model("openai", "gpt-4")]}
        providers={[provider("openai"), provider("groq", { reachable: false })]}
      />,
    );

    open();
    const groq = screen.getByRole("button", { name: "groq" });
    expect(groq).toBeDisabled();

    fireEvent.click(groq);
    // Still on the provider level: no model rows, no Back control.
    expect(screen.queryByRole("button", { name: i18n.t("common.back") })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "openai" })).toBeInTheDocument();
  });

  it("keeps an unavailable provider reachable while it is the current one", () => {
    // The manifest form lists a provider the agent already uses even when it
    // is down or its key was rejected. Disabling that row would strand the
    // value that is in the manifest.
    render(
      <ModelPicker
        label="Agent model"
        value={{ provider: "groq", model: "llama-3" }}
        onChange={() => {}}
        models={[model("groq", "llama-3"), model("openai", "gpt-4")]}
        providers={[
          provider("openai"),
          provider("groq", { reachable: false }),
          provider("cohere", { reachable: false }),
        ]}
      />,
    );

    open();
    const groq = screen.getByRole("button", { name: "groq" });
    expect(groq).toBeEnabled();
    expect(groq).toHaveAttribute("aria-current", "true");

    // A *different* unavailable provider, not the current value, stays blocked.
    expect(screen.getByRole("button", { name: "cohere" })).toBeDisabled();
    // An available provider is enabled.
    expect(screen.getByRole("button", { name: "openai" })).toBeEnabled();
  });

  it("returns from the hand-entry panel on Back instead of stranding it", () => {
    render(
      <ModelPicker
        label="Agent model"
        allowCustom
        value={{ provider: "anthropic", model: "claude-sonnet-5" }}
        onChange={() => {}}
        models={[model("openai", "gpt-4"), model("anthropic", "claude-sonnet-5")]}
        providers={[provider("openai"), provider("anthropic")]}
      />,
    );

    open();
    fireEvent.click(screen.getByRole("button", { name: "openai" }));
    fireEvent.click(screen.getByRole("button", { name: i18n.t("model_param.custom") }));

    // The hand-entry panel, with the model empty because the current value
    // belongs to another provider.
    expect(screen.getByLabelText(i18n.t("agents.form.model_id"))).toHaveValue("");

    fireEvent.click(screen.getByRole("button", { name: i18n.t("common.back") }));

    // Back to the model list of the provider we drilled into — not the provider
    // level, and not a hand-entry panel left with a Confirm that cannot enable.
    expect(screen.getByRole("button", { name: "openai/gpt-4" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: i18n.t("common.confirm") })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: i18n.t("model_param.custom") })).toBeInTheDocument();
  });

  it("gives the hand-entry panel an explicit cancel", () => {
    render(
      <ModelPicker
        label="Agent model"
        allowCustom
        value={null}
        onChange={() => {}}
        models={[]}
        providers={[]}
      />,
    );

    open();
    fireEvent.click(screen.getByRole("button", { name: i18n.t("model_param.custom") }));
    expect(screen.getByLabelText(i18n.t("agents.form.provider"))).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: i18n.t("common.cancel") }));

    expect(screen.queryByLabelText(i18n.t("agents.form.provider"))).not.toBeInTheDocument();
    expect(screen.queryByLabelText(i18n.t("agents.form.model_id"))).not.toBeInTheDocument();
    // The provider list (empty here) and the Custom row are reachable again.
    expect(screen.getByRole("button", { name: i18n.t("model_param.custom") })).toBeInTheDocument();
  });

  it("does not prefill a model that belongs to a different provider", () => {
    const onChange = vi.fn();
    render(
      <ModelPicker
        label="Agent model"
        allowCustom
        value={{ provider: "anthropic", model: "claude-sonnet-5" }}
        onChange={onChange}
        models={[model("openai", "gpt-4"), model("anthropic", "claude-sonnet-5")]}
        providers={[provider("openai"), provider("anthropic")]}
      />,
    );

    open();
    fireEvent.click(screen.getByRole("button", { name: "openai" }));
    fireEvent.click(screen.getByRole("button", { name: i18n.t("model_param.custom") }));

    const modelInput = screen.getByLabelText(i18n.t("agents.form.model_id"));
    // Not `claude-sonnet-5`: that pair does not exist and prefilling it left
    // Confirm enabled for one wrong click.
    expect(modelInput).toHaveValue("");
    expect(screen.getByRole("button", { name: i18n.t("common.confirm") })).toBeDisabled();

    fireEvent.change(modelInput, { target: { value: "gpt-6" } });
    fireEvent.click(screen.getByRole("button", { name: i18n.t("common.confirm") }));
    expect(onChange).toHaveBeenCalledWith({ provider: "openai", model: "gpt-6" });
  });

  it("counts only the models it was handed, not the server's unfiltered total", () => {
    render(
      <ModelPicker
        label="Agent model"
        value={null}
        onChange={() => {}}
        models={[model("openai", "gpt-4")]}
        providers={[provider("openai"), provider("groq", { model_count: 12 })]}
      />,
    );

    open();
    // groq is listed but every one of its models was filtered out, so it must
    // read 0 rather than advertise 12 rows the drill-down cannot produce.
    expect(screen.getByRole("button", { name: "groq" })).toHaveTextContent("0");
    expect(screen.getByRole("button", { name: "groq" })).not.toHaveTextContent("12");
    expect(screen.getByRole("button", { name: "openai" })).toHaveTextContent("1");
  });

  it("keeps the current provider reachable when the providers prop omits it", () => {
    render(
      <ModelPicker
        label="Agent model"
        value={{ provider: "groq", model: "llama-3" }}
        onChange={() => {}}
        models={[model("groq", "llama-3")]}
        providers={[provider("openai")]}
      />,
    );

    open();
    const groq = screen.getByRole("button", { name: "groq" });
    expect(groq).toBeEnabled();
    expect(groq).toHaveAttribute("aria-current", "true");

    fireEvent.click(groq);
    expect(screen.getByRole("button", { name: "groq/llama-3" })).toBeInTheDocument();
  });

  it("freezes the model rows while a write is in flight", () => {
    const base = {
      label: "Agent model",
      value: { provider: "openai", model: "gpt-4" },
      onChange: () => {},
      models: [model("openai", "gpt-4"), model("openai", "gpt-5")],
      providers: [provider("openai")],
    };
    const { rerender } = render(<ModelPicker {...base} busy={false} />);

    open();
    fireEvent.click(screen.getByRole("button", { name: "openai" }));
    // Re-render with the write in flight, as happens after a commit that keeps
    // the popover open. `pointer-events-none` alone would not stop Tab+Enter.
    rerender(<ModelPicker {...base} busy />);

    expect(screen.getByRole("button", { name: "openai/gpt-5" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "openai/gpt-4" })).toBeDisabled();
  });

  it("keeps the open list during a background refetch instead of blanking it", () => {
    render(
      <ModelPicker
        label="Agent model"
        isFetching
        allowCustom
        value={{ provider: "openai", model: "gpt-4" }}
        onChange={() => {}}
        models={[model("openai", "gpt-4")]}
        providers={[provider("openai")]}
      />,
    );

    open();
    // The selection, the provider row and the Custom row survive a refetch;
    // only the empty state is suppressed while the spinner shows.
    expect(screen.getByRole("button", { name: "openai" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: i18n.t("model_param.custom") })).toBeInTheDocument();
    expect(screen.getByText(i18n.t("chat.loading_models"))).toBeInTheDocument();
  });

  it("shows the loading state, not 'No models found', on a first load", () => {
    render(
      <ModelPicker
        label="Agent model"
        isFetching
        value={null}
        onChange={() => {}}
        models={[]}
        providers={[]}
      />,
    );

    open();
    expect(screen.getByText(i18n.t("chat.loading_models"))).toBeInTheDocument();
    expect(screen.queryByText(i18n.t("chat.no_models_found"))).not.toBeInTheDocument();
  });

  it("exposes the popover as the dialog the trigger promises", () => {
    render(
      <ModelPicker label="Agent model" value={null} onChange={() => {}} models={[]} providers={[]} />,
    );

    const trigger = screen.getByRole("button", { name: /^Agent model:/ });
    expect(trigger).not.toHaveAttribute("aria-controls");

    open();
    const dialog = screen.getByRole("dialog", { name: "Agent model" });
    expect(trigger).toHaveAttribute("aria-controls", dialog.id);
  });

  // Fields like `[routing] simple_model` hold a bare model name, resolved
  // against the global catalog — a `provider/model` string would not resolve.
  describe("flat model shape", () => {
    const catalog = [
      model("openai", "gpt-4"),
      model("anthropic", "claude-sonnet-5"),
      model("openai", "o3-mini", "o3 Mini"),
    ];

    it("lists every model without a provider level", () => {
      render(
        <ModelPicker
          label="Simple model"
          variant="model"
          value={null}
          onChange={() => {}}
          models={catalog}
        />,
      );

      open("Simple model");
      // All three are reachable immediately — there is no provider step.
      expect(screen.getByRole("button", { name: "openai/gpt-4" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "anthropic/claude-sonnet-5" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "openai/o3-mini" })).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: i18n.t("common.back") })).not.toBeInTheDocument();
    });

    it("names the current model on the trigger without a dangling separator", () => {
      render(
        <ModelPicker
          label="Simple model"
          variant="model"
          // The form adapts a bare name into a pair with an empty provider.
          value={{ provider: "", model: "gpt-4" }}
          onChange={() => {}}
          models={catalog}
        />,
      );
      expect(screen.getByRole("button", { name: "Simple model: gpt-4" })).toBeInTheDocument();
    });

    it("marks the configured model current even though the pair's provider is empty", () => {
      render(
        <ModelPicker
          label="Simple model"
          variant="model"
          // The form adapts a bare name into a pair with an empty provider.
          value={{ provider: "", model: "gpt-4" }}
          onChange={() => {}}
          models={catalog}
        />,
      );

      open("Simple model");
      // The id matches; comparing `provider` too would leave it unmarked.
      expect(screen.getByRole("button", { name: "openai/gpt-4" })).toHaveAttribute(
        "aria-current",
        "true",
      );
      expect(screen.getByRole("button", { name: "anthropic/claude-sonnet-5" })).not.toHaveAttribute(
        "aria-current",
      );
    });

    it("cancels hand entry back to the flat list", () => {
      render(
        <ModelPicker
          label="Simple model"
          variant="model"
          allowCustom
          value={null}
          onChange={() => {}}
          models={catalog}
        />,
      );

      open("Simple model");
      fireEvent.click(screen.getByRole("button", { name: i18n.t("model_param.custom") }));
      fireEvent.click(screen.getByRole("button", { name: i18n.t("common.cancel") }));

      expect(screen.getByRole("button", { name: "openai/gpt-4" })).toBeInTheDocument();
      expect(screen.queryByLabelText(i18n.t("agents.form.model_id"))).not.toBeInTheDocument();
    });

    it("clears the current value through the None row when the caller offers one", () => {
      const onClear = vi.fn();
      render(
        <ModelPicker
          label="Simple model"
          variant="model"
          value={{ provider: "", model: "gpt-4" }}
          onChange={() => {}}
          onClear={onClear}
          models={catalog}
        />,
      );

      open("Simple model");
      // Only when the caller passes `onClear`: a picker that cannot clear must
      // not offer a row that does nothing.
      fireEvent.click(screen.getByRole("button", { name: i18n.t("common.none") }));
      expect(onClear).toHaveBeenCalledTimes(1);
    });

    it("offers no clear row without an onClear handler", () => {
      render(
        <ModelPicker
          label="Simple model"
          variant="model"
          value={{ provider: "", model: "gpt-4" }}
          onChange={() => {}}
          models={catalog}
        />,
      );

      open("Simple model");
      expect(
        screen.queryByRole("button", { name: i18n.t("common.none") }),
      ).not.toBeInTheDocument();
    });

    it("collapses an id served by several providers into one row", () => {
      render(
        <ModelPicker
          label="Simple model"
          variant="model"
          // The form adapts a bare name into a pair with an empty provider.
          value={{ provider: "", model: "gpt-4" }}
          onChange={() => {}}
          models={[
            model("openai", "gpt-4"),
            model("azure", "gpt-4"),
            model("anthropic", "claude-sonnet-5"),
          ]}
        />,
      );

      open("Simple model");
      // The flat shape stores the name alone and `find_model` takes the first
      // match, so the provider is not part of the choice: one row per id rather
      // than one per provider, all of them marked active.
      expect(screen.getByRole("button", { name: "openai/gpt-4" })).toHaveAttribute(
        "aria-current",
        "true",
      );
      expect(screen.queryByRole("button", { name: "azure/gpt-4" })).not.toBeInTheDocument();
    });

    it("reports the provider of the row that was picked", () => {
      const onChange = vi.fn();
      render(
        <ModelPicker
          label="Simple model"
          variant="model"
          value={null}
          onChange={onChange}
          models={catalog}
        />,
      );

      open("Simple model");
      fireEvent.click(screen.getByRole("button", { name: "anthropic/claude-sonnet-5" }));
      expect(onChange).toHaveBeenCalledWith({ provider: "anthropic", model: "claude-sonnet-5" });
    });

    it("narrows by provider name, which is the only way to search a flat catalog", () => {
      render(
        <ModelPicker
          label="Simple model"
          variant="model"
          value={null}
          onChange={() => {}}
          models={catalog}
        />,
      );

      open("Simple model");
      fireEvent.change(screen.getByRole("textbox"), { target: { value: "anthropic" } });

      expect(screen.getByRole("button", { name: "anthropic/claude-sonnet-5" })).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "openai/gpt-4" })).not.toBeInTheDocument();
    });

    it("takes a hand-entered name with no provider at all", () => {
      const onChange = vi.fn();
      render(
        <ModelPicker
          label="Simple model"
          variant="model"
          allowCustom
          value={null}
          onChange={onChange}
          models={catalog}
        />,
      );

      open("Simple model");
      fireEvent.click(screen.getByRole("button", { name: i18n.t("model_param.custom") }));

      // No provider field: these fields hold a name, so demanding a provider
      // would make a valid entry impossible to commit.
      expect(screen.queryByLabelText(i18n.t("agents.form.provider"))).not.toBeInTheDocument();
      fireEvent.change(screen.getByLabelText(i18n.t("agents.form.model_id")), {
        target: { value: "llama-3.3-70b" },
      });
      fireEvent.click(screen.getByRole("button", { name: i18n.t("common.confirm") }));

      expect(onChange).toHaveBeenCalledWith({ provider: "", model: "llama-3.3-70b" });
    });
  });

  // The catalog is built from live discovery. When discovery finds nothing for
  // a provider — or an operator wants a model the daemon has never seen — the
  // picker has to stay usable, which is the whole reason `allowCustom` exists.
  describe("hand entry", () => {
    const customRow = () => screen.queryByRole("button", { name: i18n.t("model_param.custom") });

    it("is absent unless the caller asks for it", () => {
      render(
        <ModelPicker
          label="Agent model"
          value={null}
          onChange={() => {}}
          models={[model("openai", "gpt-4")]}
          providers={[provider("openai")]}
        />,
      );
      open();
      expect(customRow()).not.toBeInTheDocument();
    });

    it("takes both halves of the pair when no provider is drilled into", () => {
      const onChange = vi.fn();
      render(
        <ModelPicker
          label="Agent model"
          allowCustom
          value={null}
          onChange={onChange}
          models={[]}
          providers={[]}
        />,
      );

      open();
      fireEvent.click(customRow()!);
      fireEvent.change(screen.getByLabelText(i18n.t("agents.form.provider")), {
        target: { value: "  mycorp  " },
      });
      fireEvent.change(screen.getByLabelText(i18n.t("agents.form.model_id")), {
        target: { value: "my-model-1" },
      });
      fireEvent.click(screen.getByRole("button", { name: i18n.t("common.confirm") }));

      // Trimmed: a trailing space from a paste is not part of the id.
      expect(onChange).toHaveBeenCalledWith({ provider: "mycorp", model: "my-model-1" });
    });

    it("takes only the model when it is already inside a provider", () => {
      const onChange = vi.fn();
      render(
        <ModelPicker
          label="Agent model"
          allowCustom
          value={null}
          onChange={onChange}
          models={[model("openai", "gpt-4")]}
          providers={[provider("openai")]}
        />,
      );

      open();
      fireEvent.click(screen.getByRole("button", { name: "openai" }));
      fireEvent.click(customRow()!);

      expect(screen.queryByLabelText(i18n.t("agents.form.provider"))).not.toBeInTheDocument();
      fireEvent.change(screen.getByLabelText(i18n.t("agents.form.model_id")), {
        target: { value: "gpt-6-unreleased" },
      });
      fireEvent.click(screen.getByRole("button", { name: i18n.t("common.confirm") }));

      expect(onChange).toHaveBeenCalledWith({ provider: "openai", model: "gpt-6-unreleased" });
    });

    it("will not commit a pair with a missing half", () => {
      const onChange = vi.fn();
      render(
        <ModelPicker
          label="Agent model"
          allowCustom
          value={null}
          onChange={onChange}
          models={[]}
          providers={[]}
        />,
      );

      open();
      fireEvent.click(customRow()!);
      const confirm = screen.getByRole("button", { name: i18n.t("common.confirm") });

      expect(confirm).toBeDisabled();
      fireEvent.change(screen.getByLabelText(i18n.t("agents.form.provider")), {
        target: { value: "mycorp" },
      });
      // Provider alone is not a model.
      expect(screen.getByRole("button", { name: i18n.t("common.confirm") })).toBeDisabled();

      fireEvent.change(screen.getByLabelText(i18n.t("agents.form.model_id")), {
        target: { value: "m" },
      });
      expect(screen.getByRole("button", { name: i18n.t("common.confirm") })).toBeEnabled();
      expect(onChange).not.toHaveBeenCalled();
    });
  });
});
