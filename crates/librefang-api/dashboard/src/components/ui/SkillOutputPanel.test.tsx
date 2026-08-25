import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useUIStore } from "../../lib/store";
import { SkillOutputPanel } from "./SkillOutputPanel";

vi.mock("react-i18next", async (importOriginal) => ({
  ...(await importOriginal<typeof import("react-i18next")>()),
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) =>
      options?.defaultValue ?? key,
  }),
}));

describe("SkillOutputPanel", () => {
  beforeEach(() => {
    act(() =>
      useUIStore.setState({
        skillOutputs: [
          {
            id: "output-1",
            skillName: "formatter",
            content: "Formatted output",
            timestamp: 0,
          },
        ],
        isMobileMenuOpen: false,
        isSidebarCollapsed: false,
      }),
    );
  });

  afterEach(() => {
    act(() => useUIStore.setState({ skillOutputs: [] }));
  });

  it("exposes the collapsible content state and relationship", async () => {
    const user = userEvent.setup();
    render(<SkillOutputPanel />);

    const toggle = screen.getByRole("button", { name: /Skill Outputs/ });
    const contentId = toggle.getAttribute("aria-controls");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(contentId).toBeTruthy();
    expect(document.getElementById(contentId!)).toBeInTheDocument();

    await user.click(toggle);

    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(document.getElementById(contentId!)).not.toBeInTheDocument();
  });

  it("gives the clear action a reliable accessible name", async () => {
    const user = userEvent.setup();
    render(<SkillOutputPanel />);

    await user.click(screen.getByRole("button", { name: "Clear" }));

    expect(screen.queryByText("Formatted output")).not.toBeInTheDocument();
  });

  it("names each dismiss action with its skill", async () => {
    const user = userEvent.setup();
    render(<SkillOutputPanel />);

    await user.click(
      screen.getByRole("button", { name: "Remove formatter" }),
    );

    expect(screen.queryByText("Formatted output")).not.toBeInTheDocument();
  });
});
