import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AgentSkillItem } from "./AgentSkillItem";

// Issue #4925 — assignment UI used to show only the skill name; this
// component renders the description inline below the name when present
// and falls back to the previous "installed" hint when the global
// registry has no description (or it's empty). The test pins both
// branches so a regression toward "name-only" surfaces immediately.

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, defaultOrOpts?: unknown) => {
        if (
          defaultOrOpts &&
          typeof defaultOrOpts === "object" &&
          "defaultValue" in (defaultOrOpts as Record<string, unknown>)
        ) {
          return String(
            (defaultOrOpts as { defaultValue: string }).defaultValue,
          );
        }
        return typeof defaultOrOpts === "string" ? defaultOrOpts : key;
      },
    }),
  };
});

describe("AgentSkillItem", () => {
  it("shows the description inline below the name when present", () => {
    render(
      <AgentSkillItem
        name="web-search"
        description="Searches the public web and returns ranked snippets"
      />,
    );
    expect(screen.getByTestId("agent-skill-item-name").textContent).toBe(
      "web-search",
    );
    expect(
      screen.getByTestId("agent-skill-item-description").textContent,
    ).toBe("Searches the public web and returns ranked snippets");
  });

  it("falls back to the 'installed' hint when description is missing", () => {
    render(<AgentSkillItem name="writing-coach" />);
    expect(
      screen.getByTestId("agent-skill-item-description").textContent,
    ).toBe("installed");
  });

  it("falls back to the 'installed' hint when description is the empty string", () => {
    // Empty/whitespace descriptions used to render as a stray blank
    // second line because the original code unconditionally rendered
    // the subtitle div. The component now trims and treats empty as
    // "no description" so the row layout stays stable.
    render(<AgentSkillItem name="puppeteer" description="   " />);
    expect(
      screen.getByTestId("agent-skill-item-description").textContent,
    ).toBe("installed");
  });

  it("invokes onClick when the row is clicked", async () => {
    const onClick = vi.fn();
    render(
      <AgentSkillItem
        name="web-search"
        description="hi"
        onClick={onClick}
      />,
    );
    await userEvent.click(screen.getByTestId("agent-skill-item-action"));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  // Issue #4917 — inline assignment. The same row now renders a trailing
  // ✕ (remove) for assigned skills and a ＋ (add) for available ones.

  it("renders a remove button and fires onRemove without bubbling to the row", async () => {
    const onClick = vi.fn();
    const onRemove = vi.fn();
    render(
      <AgentSkillItem
        name="web-search"
        action="remove"
        onClick={onClick}
        onRemove={onRemove}
      />,
    );
    await userEvent.click(screen.getByTestId("agent-skill-item-remove"));
    expect(onRemove).toHaveBeenCalledTimes(1);
    // stopPropagation must keep the row's onClick from also firing.
    expect(onClick).not.toHaveBeenCalled();
    expect(screen.getByTestId("agent-skill-item-action"))
      .not.toContainElement(screen.getByTestId("agent-skill-item-remove"));
  });

  it("renders an add affordance and assigns via the row click", async () => {
    const onClick = vi.fn();
    render(
      <AgentSkillItem name="writing-coach" action="add" onClick={onClick} />,
    );
    expect(screen.getByTestId("agent-skill-item-add")).toBeTruthy();
    await userEvent.click(screen.getByTestId("agent-skill-item-action"));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("disables the remove button while busy", async () => {
    const onRemove = vi.fn();
    render(
      <AgentSkillItem
        name="web-search"
        action="remove"
        onRemove={onRemove}
        busy
      />,
    );
    const btn = screen.getByTestId("agent-skill-item-remove") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    await userEvent.click(btn);
    expect(onRemove).not.toHaveBeenCalled();
  });

  it("gives the interactive row a name from its visible content", () => {
    render(
      <AgentSkillItem
        name="web-search"
        description="Searches the public web"
        onClick={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", {
      name: "web-search Searches the public web",
    })).toBeInTheDocument();
  });

  it("blocks keyboard and pointer activation while busy", async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(<AgentSkillItem name="web-search" onClick={onClick} busy />);
    const action = screen.getByTestId("agent-skill-item-action");

    expect(action).toBeDisabled();
    action.focus();
    await user.keyboard("{Enter} ");
    await user.click(action);
    expect(onClick).not.toHaveBeenCalled();
  });

  // #7713 — a skill named in the manifest that the registry does not have.
  it("marks a pending skill and says why, while keeping it removable", () => {
    const onRemove = vi.fn();
    render(
      <AgentSkillItem
        name="ghost-skill"
        description="stale description from a previous install"
        action="remove"
        onRemove={onRemove}
        pending
      />,
    );

    expect(screen.getByTestId("agent-skill-item-pending")).toBeTruthy();
    expect(
      screen.getByTestId("agent-skill-item-description").textContent,
    ).toContain("not installed here");
    // The assignment is real — the row must still offer the remove affordance.
    expect(screen.getByTestId("agent-skill-item-remove")).toBeTruthy();
  });

  it("renders no pending marker for an installed skill", () => {
    render(<AgentSkillItem name="web-search" description="Searches the web" />);
    expect(screen.queryByTestId("agent-skill-item-pending")).toBeNull();
  });
});
