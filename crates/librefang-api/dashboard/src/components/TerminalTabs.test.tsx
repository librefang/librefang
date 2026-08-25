import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { TerminalTabs } from "./TerminalTabs";
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
