import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Keep i18n predictable — return the default value when one is supplied,
// otherwise the key. Matches the pattern used by AgentSkillItem.test.tsx
// and PromptsExperimentsModal.test.tsx.
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, defaultOrOpts?: unknown) => {
      if (typeof defaultOrOpts === "string") return defaultOrOpts;
      if (
        defaultOrOpts &&
        typeof defaultOrOpts === "object" &&
        "defaultValue" in (defaultOrOpts as Record<string, unknown>)
      ) {
        return String(
          (defaultOrOpts as { defaultValue: string }).defaultValue,
        );
      }
      return key;
    },
  }),
}));

const navigateMock = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigateMock,
}));

vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: { addToast: () => void }) => unknown) =>
    selector({ addToast: vi.fn() }),
}));

// Pending approvals fixture — three items so we can verify wrap-around,
// Home/End jumps, and the order in which menuitems gain focus.
const FAKE_APPROVALS = [
  { id: "a1", tool_name: "shell.exec", status: "pending" as const },
  { id: "a2", tool_name: "fs.write", status: "pending" as const },
  { id: "a3", tool_name: "net.request", status: "pending" as const },
];

vi.mock("../lib/queries/approvals", () => ({
  useApprovalCount: () => ({ data: FAKE_APPROVALS.length, isError: false }),
  useApprovals: () => ({ data: FAKE_APPROVALS }),
  useTotpStatus: () => ({ data: { enforced: false } }),
}));

vi.mock("../lib/queries/skills", () => ({
  usePendingSkillCandidates: () => ({ data: [] }),
}));

vi.mock("../lib/mutations/approvals", () => ({
  useApproveApproval: () => ({ mutateAsync: vi.fn() }),
  useRejectApproval: () => ({ mutateAsync: vi.fn() }),
}));

import { NotificationCenter } from "./NotificationCenter";

beforeEach(() => {
  navigateMock.mockReset();
  document.body.focus();
});

function getTrigger(): HTMLButtonElement {
  return screen.getByRole("button", {
    name: /Notifications \(3\)/i,
  }) as HTMLButtonElement;
}

function getMenuItems(): HTMLElement[] {
  return Array.from(
    document.querySelectorAll<HTMLElement>("[data-notif-menuitem]"),
  );
}

describe("NotificationCenter keyboard navigation", () => {
  it("trigger exposes the WAI-ARIA Menu Button attributes", () => {
    render(<NotificationCenter />);
    const trigger = getTrigger();
    expect(trigger).toHaveAttribute("aria-haspopup", "menu");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(trigger).toHaveAttribute("aria-controls");
  });

  it("ArrowDown on the trigger opens the menu and focuses the first menuitem", async () => {
    const user = userEvent.setup();
    render(<NotificationCenter />);
    const trigger = getTrigger();
    trigger.focus();
    await user.keyboard("{ArrowDown}");

    expect(trigger).toHaveAttribute("aria-expanded", "true");
    const items = getMenuItems();
    expect(items.length).toBeGreaterThan(0);
    // First menuitem after open is the "View all" link (header).
    expect(document.activeElement).toBe(items[0]);
  });

  it("ArrowDown and ArrowUp inside the menu move focus with wrap-around", async () => {
    const user = userEvent.setup();
    render(<NotificationCenter />);
    getTrigger().focus();
    await user.keyboard("{ArrowDown}"); // open + focus first

    const items = getMenuItems();
    expect(items.length).toBe(4); // "View all" + 3 approval rows.

    await user.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(items[1]);
    await user.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(items[2]);

    // Up from middle.
    await user.keyboard("{ArrowUp}");
    expect(document.activeElement).toBe(items[1]);

    // Up from first wraps to last.
    await user.keyboard("{ArrowUp}");
    expect(document.activeElement).toBe(items[0]);
    await user.keyboard("{ArrowUp}");
    expect(document.activeElement).toBe(items[items.length - 1]);

    // Down from last wraps to first.
    await user.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(items[0]);
  });

  it("Home and End jump to the first and last menuitems", async () => {
    const user = userEvent.setup();
    render(<NotificationCenter />);
    getTrigger().focus();
    await user.keyboard("{ArrowDown}");

    const items = getMenuItems();
    await user.keyboard("{End}");
    expect(document.activeElement).toBe(items[items.length - 1]);
    await user.keyboard("{Home}");
    expect(document.activeElement).toBe(items[0]);
  });

  it("Escape closes the menu and returns focus to the trigger", async () => {
    const user = userEvent.setup();
    render(<NotificationCenter />);
    const trigger = getTrigger();
    trigger.focus();
    await user.keyboard("{ArrowDown}");
    expect(trigger).toHaveAttribute("aria-expanded", "true");

    await user.keyboard("{Escape}");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    // requestAnimationFrame in the production code defers focus restore;
    // flush it before asserting.
    await act(async () => {
      await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
    });
    expect(document.activeElement).toBe(trigger);
  });

  it("Tab closes the menu and returns focus to the trigger", async () => {
    const user = userEvent.setup();
    render(<NotificationCenter />);
    const trigger = getTrigger();
    trigger.focus();
    await user.keyboard("{ArrowDown}");
    expect(trigger).toHaveAttribute("aria-expanded", "true");

    await user.keyboard("{Tab}");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    await act(async () => {
      await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
    });
    expect(document.activeElement).toBe(trigger);
  });

  it("menu container has role=menu and is labelled by the trigger", async () => {
    const user = userEvent.setup();
    render(<NotificationCenter />);
    await user.click(getTrigger());

    const menu = document.querySelector<HTMLElement>('[role="menu"]');
    expect(menu).not.toBeNull();
    const labelledBy = menu?.getAttribute("aria-labelledby");
    expect(labelledBy).toBeTruthy();
    expect(document.getElementById(labelledBy!)).toBe(getTrigger());
  });

  it("roving tabindex: exactly one menuitem has tabIndex=0 at a time", async () => {
    const user = userEvent.setup();
    render(<NotificationCenter />);
    getTrigger().focus();
    await user.keyboard("{ArrowDown}");
    await user.keyboard("{ArrowDown}");

    const items = getMenuItems();
    const zeros = items.filter((el) => el.tabIndex === 0);
    expect(zeros.length).toBe(1);
    expect(zeros[0]).toBe(items[1]);
  });
});
