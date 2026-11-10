import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { TerminalTabs } from "./TerminalTabs";
import { TERMINAL_PANEL_ID, terminalTabId } from "./terminalTabIds";
import { useTerminalWindows } from "../lib/queries/terminal";
import {
  useCreateTerminalWindow,
  useDeleteTerminalWindow,
  useRenameTerminalWindow,
} from "../lib/mutations/terminal";
import { useUIStore } from "../lib/store";
import { safeStorageGet, safeStorageSet } from "../lib/safeStorage";
import type { TerminalWindow } from "../lib/http/client";

vi.mock("../lib/queries/terminal", () => ({ useTerminalWindows: vi.fn() }));
vi.mock("../lib/mutations/terminal", () => ({
  useCreateTerminalWindow: vi.fn(),
  useRenameTerminalWindow: vi.fn(),
  useDeleteTerminalWindow: vi.fn(),
}));
vi.mock("../lib/store", () => ({ useUIStore: vi.fn() }));
vi.mock("../lib/safeStorage", () => ({
  safeStorageGet: vi.fn(),
  safeStorageSet: vi.fn(),
}));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const windowsMock = useTerminalWindows as unknown as ReturnType<typeof vi.fn>;
const createMock = useCreateTerminalWindow as unknown as ReturnType<typeof vi.fn>;
const renameMock = useRenameTerminalWindow as unknown as ReturnType<typeof vi.fn>;
const deleteMock = useDeleteTerminalWindow as unknown as ReturnType<typeof vi.fn>;
const storeMock = useUIStore as unknown as ReturnType<typeof vi.fn>;
const storageGetMock = safeStorageGet as unknown as ReturnType<typeof vi.fn>;
const storageSetMock = safeStorageSet as unknown as ReturnType<typeof vi.fn>;

const terminalRef = { current: null };
const fitAddonRef = { current: null };

function terminalWindow(id: string, active = false): TerminalWindow {
  return { id, index: Number(id.charCodeAt(0)), name: id, active };
}

function openSocket() {
  return {
    readyState: WebSocket.OPEN,
    send: vi.fn(),
  } as unknown as WebSocket;
}

function renderTabs({
  windows,
  displayedActiveWindowId = "A",
  onSwitchWindow = vi.fn(),
  ws = openSocket(),
}: {
  windows: TerminalWindow[];
  displayedActiveWindowId?: string | null;
  onSwitchWindow?: (id: string | null) => void;
  ws?: WebSocket | null;
}) {
  windowsMock.mockReturnValue({ data: windows });
  return render(
    <TerminalTabs
      ws={ws}
      tmuxAvailable
      maxWindows={10}
      displayedActiveWindowId={displayedActiveWindowId}
      onSwitchWindow={onSwitchWindow}
      terminalRef={terminalRef}
      fitAddonRef={fitAddonRef}
      panelId={TERMINAL_PANEL_ID}
    />,
  );
}

describe("TerminalTabs", () => {
  beforeEach(() => {
    vi.stubGlobal("ResizeObserver", class {
      observe() {}
      disconnect() {}
    });
    storageGetMock.mockReturnValue(null);
    storageSetMock.mockReturnValue(true);
    createMock.mockReturnValue({ isPending: false, mutateAsync: vi.fn() });
    renameMock.mockReturnValue({ mutate: vi.fn() });
    deleteMock.mockReturnValue({ mutateAsync: vi.fn().mockResolvedValue(undefined) });
    storeMock.mockImplementation((selector: (state: { addToast: ReturnType<typeof vi.fn> }) => unknown) =>
      selector({ addToast: vi.fn() }),
    );
  });

  it("persists reconciled tab order after the state commit", async () => {
    renderTabs({ windows: [terminalWindow("A"), terminalWindow("B")] });

    await waitFor(() => expect(storageSetMock).toHaveBeenLastCalledWith(
      "terminal.tabOrder",
      JSON.stringify(["A", "B"]),
    ));
  });

  it("excludes every concurrent deletion target from the next active window", async () => {
    let resolveA!: () => void;
    let resolveB!: () => void;
    const mutateAsync = vi.fn((id: string) => new Promise<void>((resolve) => {
      if (id === "A") resolveA = resolve;
      if (id === "B") resolveB = resolve;
    }));
    deleteMock.mockReturnValue({ mutateAsync });
    const onSwitchWindow = vi.fn();
    const ws = openSocket();
    renderTabs({
      windows: [terminalWindow("A", true), terminalWindow("B"), terminalWindow("C")],
      onSwitchWindow,
      ws,
    });

    const closeButtons = screen.getAllByRole("button", { name: "terminal.tabs.close" });
    fireEvent.click(closeButtons[0]);
    fireEvent.click(closeButtons[1]);
    await act(async () => resolveA());

    expect(onSwitchWindow).toHaveBeenCalledWith("C");
    expect(ws.send).toHaveBeenCalledWith(JSON.stringify({
      type: "switch_window",
      window: "C",
    }));
    await act(async () => resolveB());
  });

  it("does not auto-select again when only the callback identity changes", async () => {
    const windows = [terminalWindow("A", true)];
    const first = vi.fn();
    const second = vi.fn();
    const view = renderTabs({
      windows,
      displayedActiveWindowId: null,
      onSwitchWindow: first,
    });
    await waitFor(() => expect(first).toHaveBeenCalledWith("A"));

    windowsMock.mockReturnValue({ data: windows });
    view.rerender(
      <TerminalTabs
        ws={openSocket()}
        tmuxAvailable
        maxWindows={10}
        displayedActiveWindowId={null}
        onSwitchWindow={second}
        terminalRef={terminalRef}
        fitAddonRef={fitAddonRef}
        panelId={TERMINAL_PANEL_ID}
      />,
    );
    expect(second).not.toHaveBeenCalled();
  });

  it("reports a disconnected tab switch instead of silently ignoring it", () => {
    const addToast = vi.fn();
    storeMock.mockImplementation((selector: (state: { addToast: typeof addToast }) => unknown) =>
      selector({ addToast }),
    );
    const onSwitchWindow = vi.fn();
    renderTabs({
      windows: [terminalWindow("A", true)],
      onSwitchWindow,
      ws: null,
    });

    const tab = screen.getByRole("tab", { name: "A" });
    expect(tab).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(tab);
    expect(addToast).toHaveBeenCalledWith(
      "terminal.tabs.switch_unavailable",
      "error",
    );
    expect(onSwitchWindow).not.toHaveBeenCalled();
  });
});

// Issue #8597 — the strip carries `role="tablist"` and `role="tab"` but none
// of the model behind them: no panel linkage, no roving tabindex, every tab a
// tab stop with no arrow keys, the new-tab button inside the tablist, and a
// nested close control that is a second tab stop.
describe("TerminalTabs tab semantics and keyboard model", () => {
  it("links every tab to the terminal panel and keeps the new-tab button outside the tablist", () => {
    renderTabs({ windows: [terminalWindow("A", true), terminalWindow("B")] });

    const tablist = screen.getByRole("tablist", { name: "nav.terminal" });
    const tabs = within(tablist).getAllByRole("tab");
    expect(tabs).toHaveLength(2);
    for (const tab of tabs) {
      expect(tab).toHaveAttribute("aria-controls", TERMINAL_PANEL_ID);
      expect(tab.id).not.toBe("");
    }
    expect(tabs[0]).toHaveAttribute("id", terminalTabId("A"));
    expect(tabs[1]).toHaveAttribute("id", terminalTabId("B"));

    // A tablist's children are tabs; the new-tab button beside it reads as
    // one otherwise.
    const newTab = screen.getByRole("button", { name: "terminal.tabs.new" });
    expect(newTab.closest('[role="tablist"]')).toBeNull();
  });

  it("roves the tabindex so only the active tab is in the tab order", () => {
    renderTabs({
      windows: [terminalWindow("A"), terminalWindow("B"), terminalWindow("C")],
      displayedActiveWindowId: "B",
    });

    const tabs = within(screen.getByRole("tablist")).getAllByRole("tab");
    const stops = tabs.filter((tab) => tab.tabIndex === 0);
    expect(stops).toHaveLength(1);
    expect(stops[0]).toHaveAttribute("aria-selected", "true");
    for (const tab of tabs) {
      if (tab !== stops[0]) expect(tab.tabIndex).toBe(-1);
    }
  });

  it("anchors the tab stop on the first tab when no window is selected yet", () => {
    renderTabs({
      windows: [terminalWindow("A"), terminalWindow("B")],
      displayedActiveWindowId: null,
    });

    const tabs = within(screen.getByRole("tablist")).getAllByRole("tab");
    expect(tabs[0].tabIndex).toBe(0);
    expect(tabs[1].tabIndex).toBe(-1);
  });

  it("switches windows with the arrow keys and moves focus to the target tab", () => {
    const ws = openSocket();
    const onSwitchWindow = vi.fn();
    renderTabs({
      windows: [terminalWindow("A", true), terminalWindow("B")],
      onSwitchWindow,
      ws,
    });

    const tabs = within(screen.getByRole("tablist")).getAllByRole("tab");
    tabs[0].focus();
    fireEvent.keyDown(tabs[0], { key: "ArrowRight" });

    expect(onSwitchWindow).toHaveBeenCalledWith("B");
    expect(ws.send).toHaveBeenCalledWith(
      JSON.stringify({ type: "switch_window", window: "B" }),
    );
    expect(document.activeElement).toBe(tabs[1]);
  });

  it("keeps the close control out of the tab order with a 24 px hit target", () => {
    renderTabs({ windows: [terminalWindow("A", true), terminalWindow("B")] });

    const closes = screen.getAllByRole("button", { name: "terminal.tabs.close" });
    expect(closes).toHaveLength(2);
    for (const close of closes) {
      expect(close.tabIndex).toBe(-1);
      // h-6/w-6 = 24 px, the WCAG 2.5.8 minimum; the icon stays 12 px.
      expect(close).toHaveClass("h-6", "w-6");
    }
  });

  it("closes the focused tab with Delete and hands focus to the neighbour", async () => {
    const mutateAsync = vi.fn().mockResolvedValue(undefined);
    deleteMock.mockReturnValue({ mutateAsync });
    renderTabs({ windows: [terminalWindow("A", true), terminalWindow("B")] });

    const tabs = within(screen.getByRole("tablist")).getAllByRole("tab");
    tabs[0].focus();
    fireEvent.keyDown(tabs[0], { key: "Delete" });

    expect(mutateAsync).toHaveBeenCalledWith("A");
    // Closing from the keyboard must not drop the user on <body>; the tab
    // that takes the closed tab's place receives focus.
    expect(document.activeElement).toBe(tabs[1]);
  });
});
