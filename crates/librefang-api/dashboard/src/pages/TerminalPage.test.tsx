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
  },
}));

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    buildAuthenticatedWebSocketUrl: () => "ws://localhost/api/terminal/ws",
  };
});

vi.mock("../lib/queries/terminal", () => ({
  useTerminalHealth: (...args: unknown[]) => useTerminalHealthMock(...args),
}));

vi.mock("../lib/store", () => ({
  useUIStore: (selector: (state: {
    terminalEnabled: boolean | null;
    addToast: (message: string, type?: "success" | "error" | "info") => void;
  }) => unknown) =>
    useUIStoreMock(selector),
}));

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
