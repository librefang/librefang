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
  // `e2e/agent-view.spec.ts`, which asserts all nine groups by name. What this
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

// Issue #8597 — the bar announces `role="tablist"` but no roving tabindex,
// no arrow keys and no link to the panel it switches.
describe("AgentTabBar tab semantics and keyboard model", () => {
  const tabs = [
    { id: "general", label: "General", Icon: Bot },
    { id: "memory", label: "Memory", Icon: Database },
    { id: "planning", label: "Planning", Icon: Clock },
  ] as const;

  it("wires every tab to the panel it controls", () => {
    render(
      <AgentTabBar
        tabs={tabs}
        active="general"
        onSelect={() => {}}
        ariaLabel="Groups"
        panelId="agent-panel"
      />,
    );

    for (const tab of screen.getAllByRole("tab")) {
      expect(tab).toHaveAttribute("aria-controls", "agent-panel");
      expect(tab.id).not.toBe("");
    }
    expect(screen.getByRole("tab", { name: "Memory" })).toHaveAttribute(
      "id",
      "agent-panel-tab-memory",
    );
  });

  it("roves the tabindex so only the active tab is in the tab order", () => {
    render(
      <AgentTabBar
        tabs={tabs}
        active="memory"
        onSelect={() => {}}
        ariaLabel="Groups"
        panelId="agent-panel"
      />,
    );

    const rendered = screen.getAllByRole("tab");
    const stops = rendered.filter((tab) => tab.tabIndex === 0);
    expect(stops).toHaveLength(1);
    expect(stops[0]).toHaveTextContent("Memory");
    for (const tab of rendered) {
      if (tab !== stops[0]) expect(tab.tabIndex).toBe(-1);
    }
  });

  it("moves selection and focus with the arrow keys, wrapping at the edges", () => {
    const onSelect = vi.fn();
    render(
      <AgentTabBar
        tabs={tabs}
        active="general"
        onSelect={onSelect}
        ariaLabel="Groups"
        panelId="agent-panel"
      />,
    );

    const rendered = screen.getAllByRole("tab");
    rendered[0].focus();
    fireEvent.keyDown(rendered[0], { key: "ArrowRight" });
    expect(onSelect).toHaveBeenLastCalledWith("memory");
    expect(document.activeElement).toBe(rendered[1]);

    fireEvent.keyDown(rendered[0], { key: "ArrowLeft" });
    expect(onSelect).toHaveBeenLastCalledWith("planning");
    expect(document.activeElement).toBe(rendered[2]);
  });

  it("jumps to the edges with Home and End", () => {
    const onSelect = vi.fn();
    render(
      <AgentTabBar
        tabs={tabs}
        active="memory"
        onSelect={onSelect}
        ariaLabel="Groups"
        panelId="agent-panel"
      />,
    );

    const rendered = screen.getAllByRole("tab");
    rendered[1].focus();
    fireEvent.keyDown(rendered[1], { key: "End" });
    expect(onSelect).toHaveBeenLastCalledWith("planning");
    expect(document.activeElement).toBe(rendered[2]);

    fireEvent.keyDown(rendered[2], { key: "Home" });
    expect(onSelect).toHaveBeenLastCalledWith("general");
    expect(document.activeElement).toBe(rendered[0]);
  });
});
