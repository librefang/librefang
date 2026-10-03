import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Bot, Clock, Database } from "lucide-react";
import { AgentTabBar } from "./AgentTabs";
import { CONFIG_GROUP_IDS } from "../pages/AgentsPage";

// The bar is the surface half of the group map: `AgentsPage` draws its config
// tabs from `CONFIG_GROUP_IDS`, so what has to hold is that the bar renders
// exactly the list it is handed — one button per id, in order, with the active
// one marked. A bar that dropped or deduplicated an entry would take a whole
// group off the surface while the map still listed it, which is the failure the
// static guards cannot see.
describe("AgentTabBar", () => {
  const tabs = [
    { id: "general", label: "General", Icon: Bot },
    { id: "memory", label: "Memory", Icon: Database },
    { id: "planning", label: "Planning", Icon: Clock },
  ] as const;

  it("renders one tab per definition, in order", () => {
    render(
      <AgentTabBar
        tabs={tabs}
        active="general"
        onSelect={() => {}}
        ariaLabel="Groups"
      />,
    );
    const rendered = screen.getAllByRole("tab");
    expect(rendered.map((el) => el.textContent)).toEqual([
      "General",
      "Memory",
      "Planning",
    ]);
  });

  it("marks only the active tab as selected", () => {
    render(
      <AgentTabBar tabs={tabs} active="memory" onSelect={() => {}} ariaLabel="Groups" />,
    );
    expect(screen.getByRole("tab", { name: "Memory" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tab", { name: "General" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
  });

  it("reports the clicked id", () => {
    const onSelect = vi.fn();
    render(
      <AgentTabBar tabs={tabs} active="general" onSelect={onSelect} ariaLabel="Groups" />,
    );
    fireEvent.click(screen.getByRole("tab", { name: "Planning" }));
    expect(onSelect).toHaveBeenCalledWith("planning");
  });

  // A rendering check, not a coverage one: this test builds its own list from
  // `CONFIG_GROUP_IDS`, so it cannot see what `AgentsPage` hands the bar — a
  // truncated list in the page would pass here. That join is guaranteed by
  // construction there (`configTabs` is `CONFIG_GROUP_IDS.map(...)`, so a group
  // cannot reach the map without a tab) and observed end to end by
  // `e2e/agent-view.spec.ts`, which asserts all eight groups by name. What this
  // test adds is that the bar draws every definition it *is* given: one dropped
  // or deduplicated would take a group off the surface while the map still
  // listed it.
  it("renders every config group id it is given", () => {
    const configTabs = CONFIG_GROUP_IDS.map((id) => ({
      id,
      label: id,
      Icon: Bot,
    }));
    render(
      <AgentTabBar
        tabs={configTabs}
        active="general"
        onSelect={() => {}}
        ariaLabel="Configuration groups"
      />,
    );
    expect(screen.getAllByRole("tab")).toHaveLength(CONFIG_GROUP_IDS.length);
  });
});
