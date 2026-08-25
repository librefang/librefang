import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { stripMarkdown, useTtsManager } from "./tts";

const synthesizeSpeech = vi.hoisted(() => vi.fn());

vi.mock("../api", () => ({ synthesizeSpeech }));

type AudioListener = EventListenerOrEventListenerObject;

class MockAudio {
  static instances: MockAudio[] = [];
  static playImpl: () => Promise<void> = () => Promise.resolve();

  currentTime = 0;
  readonly listeners = new Map<string, Set<AudioListener>>();
  readonly pause = vi.fn();
  readonly play = vi.fn(() => MockAudio.playImpl());

  constructor(readonly src: string) {
    MockAudio.instances.push(this);
  }

  addEventListener(type: string, listener: AudioListener): void {
    const listeners = this.listeners.get(type) ?? new Set<AudioListener>();
    listeners.add(listener);
    this.listeners.set(type, listeners);
  }

  removeEventListener(type: string, listener: AudioListener): void {
    this.listeners.get(type)?.delete(listener);
  }

  emit(type: string): void {
    for (const listener of this.listeners.get(type) ?? []) {
      if (typeof listener === "function") listener(new Event(type));
      else listener.handleEvent(new Event(type));
    }
  }
}

const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
};

beforeEach(() => {
  synthesizeSpeech.mockReset();
  MockAudio.instances = [];
  MockAudio.playImpl = () => Promise.resolve();
  vi.stubGlobal("Audio", MockAudio);
  Object.defineProperty(URL, "revokeObjectURL", {
    configurable: true,
    value: vi.fn(),
  });
});

describe("stripMarkdown", () => {
  it("preserves currency ranges while removing command-based inline LaTeX", () => {
    expect(stripMarkdown("Costs $5 to $10 today.")).toBe("Costs $5 to $10 today.");
    expect(stripMarkdown("Before $\\frac{1}{2}$ after")).toBe("Before  after");
  });
});

describe("useTtsManager", () => {
  it("ignores a duplicate toggle while the same message is loading", async () => {
    const pending = deferred<{ url: string }>();
    synthesizeSpeech.mockReturnValueOnce(pending.promise);
    const { result } = renderHook(() => useTtsManager());

    let first!: Promise<void>;
    act(() => {
      first = result.current.toggle("a", "hello");
    });
    await waitFor(() => expect(result.current.status).toBe("loading"));
    await act(async () => {
      await result.current.toggle("a", "hello");
    });
    expect(synthesizeSpeech).toHaveBeenCalledTimes(1);

    act(() => result.current.stop());
    await act(async () => {
      pending.resolve({ url: "blob:cancelled" });
      await first;
    });
  });

  it("discards a superseded synthesis result without disturbing newer playback", async () => {
    const firstSynthesis = deferred<{ url: string }>();
    synthesizeSpeech
      .mockReturnValueOnce(firstSynthesis.promise)
      .mockResolvedValueOnce({ url: "blob:new" });
    const { result } = renderHook(() => useTtsManager());

    let first!: Promise<void>;
    act(() => {
      first = result.current.toggle("a", "first");
    });
    await waitFor(() => expect(result.current.status).toBe("loading"));
    await act(async () => {
      await result.current.toggle("b", "second");
    });
    expect(result.current.speakingMessageId).toBe("b");
    expect(result.current.status).toBe("playing");

    await act(async () => {
      firstSynthesis.resolve({ url: "blob:old" });
      await first;
    });
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:old");
    expect(MockAudio.instances.map((audio) => audio.src)).toEqual(["blob:new"]);
    expect(result.current.speakingMessageId).toBe("b");
    expect(result.current.status).toBe("playing");
  });

  it("ignores terminal events from a superseded audio element", async () => {
    synthesizeSpeech
      .mockResolvedValueOnce({ url: "blob:first" })
      .mockResolvedValueOnce({ url: "blob:second" });
    const { result } = renderHook(() => useTtsManager());

    await act(async () => result.current.toggle("a", "first"));
    const oldAudio = MockAudio.instances[0];
    await act(async () => result.current.toggle("b", "second"));

    act(() => oldAudio.emit("ended"));
    act(() => oldAudio.emit("error"));
    expect(result.current.speakingMessageId).toBe("b");
    expect(result.current.status).toBe("playing");
    expect(result.current.error).toBeNull();
  });

  it("detaches terminal listeners when audio playback rejects", async () => {
    synthesizeSpeech.mockResolvedValueOnce({ url: "blob:failed" });
    MockAudio.playImpl = () => Promise.reject(new Error("autoplay blocked"));
    const { result } = renderHook(() => useTtsManager());

    await act(async () => result.current.toggle("a", "hello"));
    const audio = MockAudio.instances[0];
    expect(audio.listeners.get("ended")?.size).toBe(0);
    expect(audio.listeners.get("error")?.size).toBe(0);
    expect(result.current.status).toBe("idle");
    expect(result.current.error).toBe("tts_error");
  });
});
