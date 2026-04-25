import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useSessionStream } from "./sessions";

// Minimal in-test EventSource fake. Native EventSource exists in jsdom only
// as a no-op stub; we drive our own so we can deterministically assert
// open / message / error transitions.
class FakeEventSource implements Partial<EventSource> {
  static instances: FakeEventSource[] = [];
  url: string;
  withCredentials: boolean;
  readyState: number = 0; // CONNECTING
  closed = false;
  private listeners = new Map<string, Set<EventListener>>();

  constructor(url: string, init?: EventSourceInit) {
    this.url = url;
    this.withCredentials = init?.withCredentials ?? false;
    FakeEventSource.instances.push(this);
  }
  addEventListener(type: string, fn: EventListener) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set());
    this.listeners.get(type)!.add(fn);
  }
  removeEventListener(type: string, fn: EventListener) {
    this.listeners.get(type)?.delete(fn);
  }
  close() {
    this.closed = true;
    this.readyState = 2;
  }
  // Test helpers
  emitOpen() {
    this.readyState = 1;
    for (const fn of this.listeners.get("open") ?? []) fn(new Event("open"));
  }
  emit(type: string, data: string) {
    const ev = new MessageEvent(type, { data });
    for (const fn of this.listeners.get(type) ?? []) fn(ev);
  }
  emitError(closed: boolean) {
    if (closed) this.readyState = 2;
    for (const fn of this.listeners.get("error") ?? []) fn(new Event("error"));
  }
}

beforeEach(() => {
  FakeEventSource.instances = [];
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("useSessionStream", () => {
  it("does nothing when ids are missing", () => {
    const { result } = renderHook(() =>
      useSessionStream(null, null, {
        eventSourceCtor: FakeEventSource as unknown as typeof EventSource,
      }),
    );
    expect(FakeEventSource.instances).toHaveLength(0);
    expect(result.current.isAttached).toBe(false);
    expect(result.current.events).toEqual([]);
    expect(result.current.lastError).toBeNull();
  });

  it("opens a stream when both ids are present and parses events", () => {
    const { result } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        eventSourceCtor: FakeEventSource as unknown as typeof EventSource,
      }),
    );
    expect(FakeEventSource.instances).toHaveLength(1);
    const es = FakeEventSource.instances[0];
    expect(es.url).toBe("/api/agents/agent-1/sessions/sess-1/stream");

    act(() => es.emitOpen());
    expect(result.current.isAttached).toBe(true);

    act(() => es.emit("chunk", JSON.stringify({ content: "hi" })));
    expect(result.current.events).toHaveLength(1);
    expect(result.current.events[0].type).toBe("chunk");
    expect(result.current.events[0].data).toEqual({ content: "hi" });

    act(() => es.emit("tool_use", JSON.stringify({ tool: "fs_read", input: { path: "/tmp" } })));
    expect(result.current.events).toHaveLength(2);
    expect(result.current.events[1].type).toBe("tool_use");
  });

  it("auto-detaches on `done` and surfaces the event", () => {
    const { result } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        eventSourceCtor: FakeEventSource as unknown as typeof EventSource,
      }),
    );
    const es = FakeEventSource.instances[0];
    act(() => es.emitOpen());
    expect(result.current.isAttached).toBe(true);

    act(() => es.emit("done", "{}"));
    expect(result.current.isAttached).toBe(false);
    expect(result.current.events.at(-1)?.type).toBe("done");
  });

  it("treats error-before-any-data as a silent no-op (404 / not-deployed)", () => {
    const { result } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        eventSourceCtor: FakeEventSource as unknown as typeof EventSource,
      }),
    );
    const es = FakeEventSource.instances[0];
    // Connection closed without a single event arriving — looks exactly
    // like a 404 from the not-yet-deployed route.
    act(() => es.emitError(true));
    expect(result.current.isAttached).toBe(false);
    expect(result.current.lastError).toBeNull();
    expect(result.current.events).toHaveLength(0);
  });

  it("surfaces mid-stream disconnects via lastError", () => {
    const { result } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        eventSourceCtor: FakeEventSource as unknown as typeof EventSource,
      }),
    );
    const es = FakeEventSource.instances[0];
    act(() => es.emitOpen());
    act(() => es.emit("chunk", JSON.stringify({ content: "partial" })));
    act(() => es.emitError(true));
    expect(result.current.lastError).toBe("session stream disconnected");
    expect(result.current.isAttached).toBe(false);
  });

  it("closes the stream on unmount", () => {
    const { unmount } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        eventSourceCtor: FakeEventSource as unknown as typeof EventSource,
      }),
    );
    const es = FakeEventSource.instances[0];
    expect(es.closed).toBe(false);
    unmount();
    expect(es.closed).toBe(true);
  });

  it("closes and reconnects when sessionId changes", () => {
    const { rerender } = renderHook(
      ({ s }: { s: string }) =>
        useSessionStream("agent-1", s, {
          eventSourceCtor: FakeEventSource as unknown as typeof EventSource,
        }),
      { initialProps: { s: "sess-1" } },
    );
    expect(FakeEventSource.instances).toHaveLength(1);
    const first = FakeEventSource.instances[0];

    rerender({ s: "sess-2" });
    expect(first.closed).toBe(true);
    expect(FakeEventSource.instances).toHaveLength(2);
    expect(FakeEventSource.instances[1].url).toContain("sess-2");
  });

  it("falls back to raw content when payload is non-JSON", () => {
    const { result } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        eventSourceCtor: FakeEventSource as unknown as typeof EventSource,
      }),
    );
    const es = FakeEventSource.instances[0];
    act(() => es.emitOpen());
    act(() => es.emit("chunk", "not-json"));
    expect(result.current.events[0].data).toEqual({ content: "not-json" });
  });

  it("caps the event buffer at 500 to bound memory", () => {
    const { result } = renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        eventSourceCtor: FakeEventSource as unknown as typeof EventSource,
      }),
    );
    const es = FakeEventSource.instances[0];
    act(() => es.emitOpen());
    act(() => {
      for (let i = 0; i < 600; i++) {
        es.emit("chunk", JSON.stringify({ content: `c${i}` }));
      }
    });
    expect(result.current.events.length).toBe(500);
    // Oldest event dropped — first kept content is c100.
    expect(
      (result.current.events[0].data as { content?: string }).content,
    ).toBe("c100");
  });

  it("uses a custom buildUrl when supplied", () => {
    renderHook(() =>
      useSessionStream("agent-1", "sess-1", {
        eventSourceCtor: FakeEventSource as unknown as typeof EventSource,
        buildUrl: (a, s) => `https://example.test/x/${a}/${s}`,
      }),
    );
    expect(FakeEventSource.instances[0].url).toBe(
      "https://example.test/x/agent-1/sess-1",
    );
  });
});
