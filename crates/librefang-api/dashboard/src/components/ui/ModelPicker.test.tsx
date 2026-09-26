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

  it("closes on Escape even with the search box focused", () => {
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

    fireEvent.keyDown(document, { key: "Escape" });
    expect(trigger).toHaveAttribute("aria-expanded", "false");
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
        providers={[provider("openai"), provider("groq", { reachable: false })]}
      />,
    );

    open();
    const groq = screen.getByRole("button", { name: "groq" });
    expect(groq).toBeEnabled();
    expect(groq).toHaveAttribute("aria-current", "true");

    // The same provider stays blocked for a value that is not the current one.
    expect(screen.getByRole("button", { name: "openai" })).toBeEnabled();
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
