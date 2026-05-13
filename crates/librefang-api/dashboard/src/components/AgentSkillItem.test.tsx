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
    await userEvent.click(screen.getByTestId("agent-skill-item"));
    expect(onClick).toHaveBeenCalledTimes(1);
  });
});
