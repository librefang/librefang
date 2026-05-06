import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalPage } from "./TerminalPage";

const invalidateQueries = vi.fn();
const navigateMock = vi.fn();
const useTerminalHealthMock = vi.fn();
const terminalTabsPropsMock = vi.fn();
const useUIStoreMock = vi.fn();

type MockSocketHandler = ((event?: unknown) => void) | null;

class MockWebSocket {
  static instances: MockWebSocket[] = [];
  static OPEN = 1;
  static CLOSED = 3;

  readyState = MockWebSocket.OPEN;
  sentMessages: string[] = [];
  onopen: MockSocketHandler = null;
  onmessage: MockSocketHandler = null;
  onerror: MockSocketHandler = null;
  onclose: MockSocketHandler = null;

  constructor(public url: string) {
    MockWebSocket.instances.push(this);
  }

  send(data: string) {
    this.sentMessages.push(data);
  }

  close() {
    this.readyState = MockWebSocket.CLOSED;
  }

  emitOpen() {
    this.onopen?.();
  }

  emitMessage(data: unknown) {
    this.onmessage?.({ data: JSON.stringify(data) });
  }
}

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) => {
        if (key === "terminal.subtitle_error" && opts?.error) {
          return `error: ${String(opts.error)}`;
        }
        return key;
      },
    }),
  };
});

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigateMock,
}));

vi.mock("@tanstack/react-query", async () => {
  const actual = await vi.importActual<typeof import("@tanstack/react-query")>("@tanstack/react-query");
  return {
    ...actual,
    useQueryClient: () => ({ invalidateQueries }),
  };
});

// jsdom has no canvas, so the real `Terminal` blows up. Stub the
// surface area `TerminalPage` actually touches; keep it minimal so a
// future caller that reaches for a new method gets a loud failure
// instead of a silent no-op.
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    options: Record<string, unknown> = {};
    loadAddon() {}
    open() {}
    write() {}
    clear() {}
    onData() {}
    onResize() {}
    attachCustomKeyEventHandler() {}
    dispose() {}
  },
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit() {}
  },
}));

vi.mock("@xterm/addon-search", () => ({
  SearchAddon: class {
    findNext() {}
    findPrevious() {}
    dispose() {}
  },
}));

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    buildAuthenticatedWebSocket: () => ({
      url: "ws://localhost/api/terminal/ws",
      protocols: [],
    }),
  };
});

vi.mock("../lib/queries/terminal", () => ({
  useTerminalHealth: (...args: unknown[]) => useTerminalHealthMock(...args),
}));

vi.mock("../lib/store", () => {
  const useUIStore = (selector: (state: {
    terminalEnabled: boolean | null;
    addToast: (message: string, type?: "success" | "error" | "info") => void;
  }) => unknown) => useUIStoreMock(selector);
  // The reconnect path reads `useUIStore.getState().toasts` to grab the
  // most-recent toast id; expose a stub so the test that exercises a
  // reconnect doesn't crash on a missing static method.
  (useUIStore as unknown as { getState: () => { toasts: unknown[] } }).getState =
    () => ({ toasts: [] });
  return { useUIStore };
});

vi.mock("../components/TerminalTabs", () => ({
  TerminalTabs: (props: {
    displayedActiveWindowId: string | null;
    onSwitchWindow: (id: string) => void;
  }) => {
    terminalTabsPropsMock(props);
    return (
      <div>
        <div data-testid="displayed-window">{props.displayedActiveWindowId ?? "none"}</div>
        <button onClick={() => props.onSwitchWindow("window-2")}>switch-window</button>
      </div>
    );
  },
}));

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });

  return render(
    <QueryClientProvider client={queryClient}>
      <TerminalPage />
    </QueryClientProvider>
  );
}

describe("TerminalPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket);
    useUIStoreMock.mockImplementation(
      (selector: (state: {
        terminalEnabled: boolean | null;
        addToast: (message: string, type?: "success" | "error" | "info") => void;
      }) => unknown) =>
        selector({
          terminalEnabled: true,
          addToast: vi.fn(),
        })
    );
    useTerminalHealthMock.mockReturnValue({
      data: { tmux: true, max_windows: 16, os: "linux" },
      isError: false,
    });
  });

  it("clears optimistic window after websocket error message", async () => {
    const user = userEvent.setup();
    renderPage();

    const ws = MockWebSocket.instances[0];
    act(() => {
      ws.emitOpen();
      ws.emitMessage({ type: "active_window", window_id: "window-1" });
    });

    await screen.findByText("window-1");
    await user.click(screen.getByRole("button", { name: "switch-window" }));

    expect(screen.getByTestId("displayed-window")).toHaveTextContent("window-2");

    act(() => {
      ws.emitMessage({ type: "error", content: "switch failed" });
    });

    await waitFor(() => {
      expect(screen.getByTestId("displayed-window")).toHaveTextContent("window-1");
    });
  });

  it("invalidates only terminal windows after active_window message", async () => {
    renderPage();

    const ws = MockWebSocket.instances[0];
    act(() => {
      ws.emitOpen();
      ws.emitMessage({ type: "active_window", window_id: "window-1" });
    });

    await waitFor(() => {
      expect(invalidateQueries).toHaveBeenCalledWith({ queryKey: ["terminal", "windows"] });
    });
    expect(invalidateQueries).not.toHaveBeenCalledWith({ queryKey: ["terminal"] });
    expect(invalidateQueries).not.toHaveBeenCalledWith({ queryKey: ["terminal", "health"] });
  });

  it("stops auto-reconnect after consecutive fast-failed launches (#4675)", async () => {
    // Two back-to-back connections that get `started` and then `exit` with
    // a non-zero code inside the FAST_EXIT_WINDOW_MS slot must trip the
    // give-up path: no third socket, error banner shows the giveup string.
    vi.useFakeTimers();
    try {
      renderPage();

      const fastFail = async (ws: MockWebSocket) => {
        await act(async () => {
          ws.emitOpen();
          ws.emitMessage({ type: "started", shell: "tmux", pid: 1234 });
          ws.emitMessage({ type: "exit", code: 1 });
          // The reconnect path requires either wsRef===null or
          // readyState===CLOSED; mirror what a real WS close would do.
          ws.readyState = MockWebSocket.CLOSED;
          ws.onclose?.({ code: 1006, reason: "" } as unknown as CloseEvent);
        });
      };

      // First fast-fail — handler classifies as transient, schedules a
      // reconnect with the 2 s base delay; counter goes 0 → 1.
      await fastFail(MockWebSocket.instances[0]);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(5_000);
      });
      expect(MockWebSocket.instances.length).toBe(2);

      // Second fast-fail — counter goes 1 → 2, hits MAX_CONSECUTIVE_FAST_FAILS.
      await fastFail(MockWebSocket.instances[1]);
      await act(async () => {
        // Long enough to cover any delayed retry that should NOT fire.
        await vi.advanceTimersByTimeAsync(10_000);
      });
      expect(MockWebSocket.instances.length).toBe(2);
      expect(screen.getByText("terminal.fast_exit_giveup")).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not resend null desired window on reconnect after disconnect before active window", async () => {
    const user = userEvent.setup();
    renderPage();

    const firstSocket = MockWebSocket.instances[0];
    act(() => {
      firstSocket.emitOpen();
    });

    await screen.findByRole("button", { name: "terminal.disconnect" });

    await user.click(screen.getByRole("button", { name: "terminal.disconnect" }));

    await user.click(screen.getByRole("button", { name: "terminal.connect" }));

    const secondSocket = MockWebSocket.instances[1];
    act(() => {
      secondSocket.emitOpen();
    });

    const switchMessages = secondSocket.sentMessages
      .map((msg) => JSON.parse(msg) as { type: string; window?: string })
      .filter((msg) => msg.type === "switch_window");

    expect(switchMessages).toEqual([]);
  });
});
