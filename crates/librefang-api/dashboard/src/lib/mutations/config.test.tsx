import { beforeEach, describe, it, expect, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useSaveMediaModelEndpoint } from "./config";
import { configKeys } from "../queries/keys";
import { createQueryClientWrapper } from "../test/query-client";
import { setConfigValue } from "../http/client";
import type { MediaModelEndpoint } from "../../api";

vi.mock("../http/client", async () => {
  const actual = await vi.importActual<typeof import("../http/client")>(
    "../http/client",
  );
  return {
    ...actual,
    setConfigValue: vi.fn().mockResolvedValue({ status: "ok" }),
  };
});

const setConfigValueMock = vi.mocked(setConfigValue);

function sttEndpoint(
  overrides: Partial<MediaModelEndpoint> = {},
): MediaModelEndpoint {
  return {
    kind: "stt",
    config_path: "media.custom_stt",
    provider_path: "media.audio_provider",
    provider: "local-whisper",
    config: {
      base_url: "http://localhost:8080/v1/audio/transcriptions",
      api_key_env: "MY_LOCAL_WHISPER_KEY",
      key_required: false,
      model: "large-v3",
    },
    configured: true,
    ...overrides,
  };
}

describe("useSaveMediaModelEndpoint", () => {
  beforeEach(() => {
    setConfigValueMock.mockClear();
    setConfigValueMock.mockResolvedValue({ status: "ok" });
  });

  it("posts the endpoint table wholesale at its two-segment config path", async () => {
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useSaveMediaModelEndpoint(), { wrapper });

    await result.current.mutateAsync({
      endpoint: sttEndpoint(),
      draft: {
        base_url: "http://whisper.internal/v1/audio/transcriptions",
        key_required: true,
        model: "medium.en",
      },
      provider: "local-whisper",
    });

    // The table goes over as one object rather than field-by-field, so an
    // untouched key like `api_key_env` is never dropped by the write.
    expect(setConfigValueMock).toHaveBeenCalledTimes(1);
    expect(setConfigValueMock).toHaveBeenCalledWith("media.custom_stt", {
      base_url: "http://whisper.internal/v1/audio/transcriptions",
      api_key_env: "MY_LOCAL_WHISPER_KEY",
      key_required: true,
      model: "medium.en",
    });
  });

  it("writes the provider selector after the table, and only when it changed", async () => {
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useSaveMediaModelEndpoint(), { wrapper });

    const saved = await result.current.mutateAsync({
      endpoint: sttEndpoint({ provider: "" }),
      draft: { base_url: "http://whisper.internal", key_required: false, model: "" },
      provider: " local-whisper ",
    });

    expect(saved.writes.map((w) => w.path)).toEqual([
      "media.custom_stt",
      "media.audio_provider",
    ]);
    expect(setConfigValueMock).toHaveBeenNthCalledWith(
      2,
      "media.audio_provider",
      "local-whisper",
    );
  });

  it("clears the provider selector with null so the key is removed", async () => {
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useSaveMediaModelEndpoint(), { wrapper });

    await result.current.mutateAsync({
      endpoint: sttEndpoint(),
      draft: { base_url: "", key_required: false, model: "" },
      provider: "",
    });

    expect(setConfigValueMock).toHaveBeenNthCalledWith(
      2,
      "media.audio_provider",
      null,
    );
  });

  it("posts the TTS table at tts.custom with voice and format", async () => {
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useSaveMediaModelEndpoint(), { wrapper });

    await result.current.mutateAsync({
      endpoint: sttEndpoint({
        kind: "tts",
        config_path: "tts.custom",
        provider_path: "tts.provider",
        provider: "local-piper",
        config: {
          base_url: "http://localhost:5000/v1/audio/speech",
          api_key_env: "",
          key_required: false,
          model: "tts-1",
          voice: "alloy",
          format: "mp3",
        },
      }),
      draft: {
        base_url: "http://piper.internal/v1/audio/speech",
        key_required: false,
        model: "tts-1",
        voice: "en_US-lessac-medium",
        format: "wav",
      },
      provider: "local-piper",
    });

    expect(setConfigValueMock).toHaveBeenCalledWith("tts.custom", {
      base_url: "http://piper.internal/v1/audio/speech",
      api_key_env: "",
      key_required: false,
      model: "tts-1",
      voice: "en_US-lessac-medium",
      format: "wav",
    });
  });

  it("invalidates the whole config domain, which is what holds the tables", async () => {
    const { queryClient, wrapper } = createQueryClientWrapper();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useSaveMediaModelEndpoint(), { wrapper });

    await result.current.mutateAsync({
      endpoint: sttEndpoint(),
      draft: { base_url: "http://whisper.internal", key_required: false, model: "" },
      provider: "local-whisper",
    });

    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: configKeys.all });
    });
  });

  it("surfaces a rejected write instead of reporting success", async () => {
    setConfigValueMock.mockRejectedValueOnce(
      new Error("config path 'media.custom_stt' is not user-tunable"),
    );
    const { wrapper } = createQueryClientWrapper();
    const { result } = renderHook(() => useSaveMediaModelEndpoint(), { wrapper });

    await expect(
      result.current.mutateAsync({
        endpoint: sttEndpoint(),
        draft: { base_url: "http://whisper.internal", key_required: false, model: "" },
        provider: "local-whisper",
      }),
    ).rejects.toThrow("not user-tunable");
  });
});
