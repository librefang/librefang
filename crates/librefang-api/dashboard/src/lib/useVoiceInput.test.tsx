import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { transcribeAudio } from "../api";
import { useVoiceInput } from "./useVoiceInput";

vi.mock("../api", () => ({ transcribeAudio: vi.fn() }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolver) => {
    resolve = resolver;
  });
  return { promise, resolve };
}

class FakeMediaRecorder {
  static instance: FakeMediaRecorder | undefined;
  static isTypeSupported = vi.fn(() => true);

  ondataavailable: ((event: { data: Blob }) => void) | null = null;
  onstop: (() => void | Promise<void>) | null = null;

  constructor() {
    FakeMediaRecorder.instance = this;
  }

  start = vi.fn();
  stop = vi.fn(() => {
    void this.onstop?.();
  });
}

describe("useVoiceInput", () => {
  const stopTrack = vi.fn();
  const stream = { getTracks: () => [{ stop: stopTrack }] } as unknown as MediaStream;
  const getUserMedia = vi.fn(async () => stream);

  beforeEach(() => {
    FakeMediaRecorder.instance = undefined;
    getUserMedia.mockClear();
    stopTrack.mockClear();
    vi.mocked(transcribeAudio).mockReset();
    vi.stubGlobal("MediaRecorder", FakeMediaRecorder);
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: { getUserMedia },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("uses the callback from the latest committed render", async () => {
    const firstCallback = vi.fn();
    const nextCallback = vi.fn();
    const transcription = deferred<{ text: string; provider: string; model: string }>();
    vi.mocked(transcribeAudio).mockReturnValue(transcription.promise);
    const { result, rerender } = renderHook(
      ({ callback }) => useVoiceInput(callback),
      { initialProps: { callback: firstCallback } },
    );

    await act(async () => {
      await result.current.startRecording();
    });
    act(() => FakeMediaRecorder.instance?.ondataavailable?.({ data: new Blob(["audio"]) }));
    FakeMediaRecorder.instance?.stop();
    await waitFor(() => expect(transcribeAudio).toHaveBeenCalledOnce());
    act(() => rerender({ callback: nextCallback }));
    await act(async () => {
      transcription.resolve({ text: "  hello  ", provider: "test", model: "test" });
      await transcription.promise;
    });

    expect(firstCallback).not.toHaveBeenCalled();
    expect(nextCallback).toHaveBeenCalledWith("hello");
  });

  it("does not publish a transcription after unmount", async () => {
    const callback = vi.fn();
    const transcription = deferred<{ text: string; provider: string; model: string }>();
    vi.mocked(transcribeAudio).mockReturnValue(transcription.promise);
    const { result, unmount } = renderHook(() => useVoiceInput(callback));

    await act(async () => {
      await result.current.startRecording();
    });
    act(() => FakeMediaRecorder.instance?.ondataavailable?.({ data: new Blob(["audio"]) }));
    FakeMediaRecorder.instance?.stop();
    await waitFor(() => expect(transcribeAudio).toHaveBeenCalledOnce());
    unmount();
    await act(async () => {
      transcription.resolve({ text: "too late", provider: "test", model: "test" });
      await transcription.promise;
    });

    expect(callback).not.toHaveBeenCalled();
  });

  it("discards a microphone stream granted after unmount", async () => {
    const permission = deferred<MediaStream>();
    getUserMedia.mockReturnValueOnce(permission.promise);
    const { result, unmount } = renderHook(() => useVoiceInput(vi.fn()));

    let startPromise!: Promise<void>;
    act(() => {
      startPromise = result.current.startRecording();
    });
    unmount();
    await act(async () => {
      permission.resolve(stream);
      await startPromise;
    });

    expect(stopTrack).toHaveBeenCalledOnce();
    expect(FakeMediaRecorder.instance).toBeUndefined();
  });
});
