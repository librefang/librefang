import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useSessionStream } from "./sessions";

// Minimal in-test WebSocket fake. jsdom does not implement WebSocket; we
// drive our own so we can deterministically assert open / message /
// close transitions and assert the bearer sub-protocol is carried
// correctly (audit `docs/issues/session-sse-withcredentials.md`).
//
// Intentionally NOT `implements WebSocket`: the real interface declares
// add/removeEventListener with overloaded signatures and a number of
// dom-only members (binaryType, bufferedAmount, extensions, …) that the
// hook never touches. The hook only depends on duck-typed access through
// the `as unknown as typeof WebSocket` cast at the call sites, so
// formal interface conformance buys us nothing and just fights the
// type checker.
class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  static CONNECTING = 0 as const;
  static OPEN = 1 as const;
  static CLOSING = 2 as const;
  static CLOSED = 3 as const;

  url: string;
  protocols: string[];
  readyState: number = FakeWebSocket.CONNECTING;
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;

  closeCode: number | undefined;
  closeReason: string | undefined;

  constructor(url: string, protocols?: string | string[]) {
    this.url = url;
    this.protocols = Array.isArray(protocols)
      ? protocols
      : protocols
      ? [protocols]
      : [];
    FakeWebSocket.instances.push(this);
  }

  close(code?: number, reason?: string) {
    if (
      this.readyState === FakeWebSocket.CLOSED ||
      this.readyState === FakeWebSocket.CLOSING
    ) {
      return;
    }
    this.closeCode = code;
    this.closeReason = reason;
    this.readyState = FakeWebSocket.CLOSED;
  }

  // Test helpers
  emitOpen() {
    this.readyState = FakeWebSocket.OPEN;
    this.onopen?.(new Event("open"));
  }
  emitMessage(data: string) {
    this.onmessage?.(new MessageEvent("message", { data }));
  }
  emitClose(code: number, reason = "") {
    this.readyState = FakeWebSocket.CLOSED;
    // Synthesise a CloseEvent-shaped object. jsdom does not expose the
    // CloseEvent constructor in all versions; a plain Event with the
    // extra fields satisfies the hook's structural use.
    const ev = new Event("close") as CloseEvent;
    Object.assign(ev, { code, reason, wasClean: code === 1000 });
    this.onclose?.(ev);
  }
}

const stubAuth = (path: string) => ({
  url: `ws://test${path}`,
  protocols: ["bearer.test-token"],
});

beforeEach(() => {
  FakeWebSocket.instances = [];
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("useSessionStream", () => {
  it("does nothing when ids are missing", () => {
    const { result } = renderHook(() =>
      useSessionStream(null, null, {
        webSocketCtor: FakeWebSocket as unknown as typeof WebSocket,
        buildWebSocket: stubAuth,
      }),
    );
    expect(FakeWebSocket.instances).toHaveLength(0);
    expect(result.current.isAttached).toBe(false);
    expect(result.current.events).toEqual([]);
    expect(result.current.lastError).toBeNull();
  });

  it("opens a WebSocket with the bearer sub-protocol when both ids are present", () => {
    renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        webSocketCtor: FakeWebSocket as unknown as typeof WebSocket,
        buildWebSocket: stubAuth,
      }),
    );
    expect(FakeWebSocket.instances).toHaveLength(1);
    const ws = FakeWebSocket.instances[0];
    // Regression: must NOT use HTTP / EventSource — the URL is a ws://
    // URL built by `buildAuthenticatedWebSocket`, and the bearer token
    // rides on `Sec-WebSocket-Protocol` rather than a cookie.
    expect(ws.url).toBe("ws://test/api/agents/agent-1/sessions/sess-1/stream");
    expect(ws.protocols).toEqual(["bearer.test-token"]);
  });

  it("parses typed envelopes and surfaces them via `events`", () => {
    const { result } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        webSocketCtor: FakeWebSocket as unknown as typeof WebSocket,
        buildWebSocket: stubAuth,
      }),
    );
    const ws = FakeWebSocket.instances[0];
    act(() => ws.emitOpen());
    expect(result.current.isAttached).toBe(true);

    act(() => ws.emitMessage(JSON.stringify({ type: "chunk", content: "hi" })));
    expect(result.current.events).toHaveLength(1);
    expect(result.current.events[0].type).toBe("chunk");
    expect(result.current.events[0].data).toEqual({ content: "hi" });

    act(() =>
      ws.emitMessage(
        JSON.stringify({ type: "tool_use", tool: "fs_read", input: { path: "/tmp" } }),
      ),
    );
    expect(result.current.events).toHaveLength(2);
    expect(result.current.events[1].type).toBe("tool_use");
    expect(
      (result.current.events[1].data as { tool?: string }).tool,
    ).toBe("fs_read");
  });

  it("auto-detaches on `done` and surfaces the event", () => {
    const { result } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        webSocketCtor: FakeWebSocket as unknown as typeof WebSocket,
        buildWebSocket: stubAuth,
      }),
    );
    const ws = FakeWebSocket.instances[0];
    act(() => ws.emitOpen());
    expect(result.current.isAttached).toBe(true);

    act(() => ws.emitMessage(JSON.stringify({ type: "done" })));
    expect(result.current.isAttached).toBe(false);
    expect(
      result.current.events[result.current.events.length - 1]?.type,
    ).toBe("done");
  });

  it("treats first-attempt close-without-data as a silent no-op (route not deployed)", () => {
    const { result } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        webSocketCtor: FakeWebSocket as unknown as typeof WebSocket,
        buildWebSocket: stubAuth,
      }),
    );
    const ws = FakeWebSocket.instances[0];
    // Server closes without ever opening / sending data — looks exactly
    // like a 404 against the not-yet-deployed handler.
    act(() => ws.emitClose(1006));
    expect(result.current.isAttached).toBe(false);
    expect(result.current.lastError).toBeNull();
    expect(result.current.events).toHaveLength(0);
    // Must NOT schedule a reconnect for the first-attempt no-payload case.
    expect(FakeWebSocket.instances).toHaveLength(1);
  });

  it("surfaces an auth-required error and stops reconnecting on 4401", () => {
    const { result } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        webSocketCtor: FakeWebSocket as unknown as typeof WebSocket,
        buildWebSocket: stubAuth,
      }),
    );
    const ws = FakeWebSocket.instances[0];
    act(() => ws.emitClose(4401, "Unauthorized"));
    expect(result.current.lastError).toBe(
      "session stream authentication required",
    );
    expect(result.current.isAttached).toBe(false);
    // No reconnect attempt — auth errors require user action.
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(FakeWebSocket.instances).toHaveLength(1);
  });

  it("reconnects with backoff after a mid-stream disconnect", () => {
    const { result } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        webSocketCtor: FakeWebSocket as unknown as typeof WebSocket,
        buildWebSocket: stubAuth,
      }),
    );
    const ws = FakeWebSocket.instances[0];
    act(() => ws.emitOpen());
    act(() => ws.emitMessage(JSON.stringify({ type: "chunk", content: "partial" })));
    act(() => ws.emitClose(1006));
    expect(result.current.isAttached).toBe(false);
    // First backoff is 1s (2^0 * 1000ms).
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(FakeWebSocket.instances).toHaveLength(2);
    expect(FakeWebSocket.instances[1].url).toBe(ws.url);
  });

  it("gives up after WS_MAX_RETRIES and surfaces `lastError`", () => {
    const { result } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        webSocketCtor: FakeWebSocket as unknown as typeof WebSocket,
        buildWebSocket: stubAuth,
      }),
    );
    // First connection: open + receive so the first-attempt swallow
    // doesn't fire; then close abnormally repeatedly.
    act(() => FakeWebSocket.instances[0].emitOpen());
    act(() =>
      FakeWebSocket.instances[0].emitMessage(
        JSON.stringify({ type: "chunk", content: "ok" }),
      ),
    );
    act(() => FakeWebSocket.instances[0].emitClose(1006));
    // Drive reconnect attempts to exhaustion (5 retries → 5 new
    // instances after the first).
    for (let i = 0; i < 5; i++) {
      act(() => {
        vi.advanceTimersByTime(20_000);
      });
      const latest = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
      act(() => latest.emitClose(1006));
    }
    expect(result.current.lastError).toBe("session stream disconnected");
  });

  it("falls back to chunk { content } when a frame is non-JSON", () => {
    const { result } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        webSocketCtor: FakeWebSocket as unknown as typeof WebSocket,
        buildWebSocket: stubAuth,
      }),
    );
    const ws = FakeWebSocket.instances[0];
    act(() => ws.emitOpen());
    act(() => ws.emitMessage("not-json"));
    expect(result.current.events).toHaveLength(1);
    expect(result.current.events[0].type).toBe("chunk");
    expect(result.current.events[0].data).toEqual({ content: "not-json" });
  });

  it("closes the WebSocket on unmount", () => {
    const { unmount } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        webSocketCtor: FakeWebSocket as unknown as typeof WebSocket,
        buildWebSocket: stubAuth,
      }),
    );
    const ws = FakeWebSocket.instances[0];
    expect(ws.readyState).not.toBe(FakeWebSocket.CLOSED);
    unmount();
    expect(ws.readyState).toBe(FakeWebSocket.CLOSED);
    expect(ws.closeCode).toBe(1000);
  });

  it("closes and reopens when sessionId changes", () => {
    const { rerender } = renderHook(
      ({ s }: { s: string }) =>
        useSessionStream("agent-1", s, {
          webSocketCtor: FakeWebSocket as unknown as typeof WebSocket,
          buildWebSocket: stubAuth,
        }),
      { initialProps: { s: "sess-1" } },
    );
    expect(FakeWebSocket.instances).toHaveLength(1);
    const first = FakeWebSocket.instances[0];

    rerender({ s: "sess-2" });
    expect(first.readyState).toBe(FakeWebSocket.CLOSED);
    expect(FakeWebSocket.instances).toHaveLength(2);
    expect(FakeWebSocket.instances[1].url).toContain("sess-2");
  });

  it("caps the event buffer at 500 to bound memory", () => {
    const { result } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        webSocketCtor: FakeWebSocket as unknown as typeof WebSocket,
        buildWebSocket: stubAuth,
      }),
    );
    const ws = FakeWebSocket.instances[0];
    act(() => ws.emitOpen());
    act(() => {
      for (let i = 0; i < 600; i++) {
        ws.emitMessage(JSON.stringify({ type: "chunk", content: `c${i}` }));
      }
    });
    expect(result.current.events.length).toBe(500);
    // Oldest event dropped — first kept content is c100.
    expect(
      (result.current.events[0].data as { content?: string }).content,
    ).toBe("c100");
  });

  it("uses a custom buildPath when supplied", () => {
    renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        webSocketCtor: FakeWebSocket as unknown as typeof WebSocket,
        buildWebSocket: stubAuth,
        buildPath: (a, s) => `/custom/${a}/${s}`,
      }),
    );
    expect(FakeWebSocket.instances[0].url).toBe("ws://test/custom/agent-1/sess-1");
  });
});
