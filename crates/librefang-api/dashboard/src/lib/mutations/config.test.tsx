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
    modality_enabled: true,
    modality_enabled_path: "media.audio_transcription",
    model_override: null,
    model_override_path: "media.audio_model",
    ...overrides,
  };
}

describe("useSaveMediaModelEndpoint", () => {
  beforeEach(() => {
    setConfigValueMock.mockClear();
    setConfigValueMock.mockResolvedValue({ status: "ok" });
  });

  it("posts one leaf at a time and never mentions api_key_env", async () => {
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

    // Per-leaf, not one wholesale table: a depth-2 table write would have to
    // either echo `api_key_env` back — refused with 403 by the payload scrub in
    // #8085 — or omit it and delete the operator's env-var name from
    // config.toml, since the handler replaces the table it assigns.
    expect(setConfigValueMock.mock.calls).toEqual([
      ["media.custom_stt.base_url", "http://whisper.internal/v1/audio/transcriptions"],
      ["media.custom_stt.model", "medium.en"],
      ["media.custom_stt.key_required", true],
    ]);
    expect(
      setConfigValueMock.mock.calls.some(([path]) => String(path).includes("api_key_env")),
    ).toBe(false);
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
      "media.custom_stt.base_url",
      "media.custom_stt.model",
      "media.custom_stt.key_required",
      "media.audio_provider",
    ]);
    // The selector goes last, so a provider name is never pointed at a table
    // that has not been written yet.
    expect(setConfigValueMock).toHaveBeenLastCalledWith(
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

    expect(setConfigValueMock).toHaveBeenLastCalledWith(
      "media.audio_provider",
      null,
    );
  });

  it("posts the TTS leaves under tts.custom, voice and format included", async () => {
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

    expect(setConfigValueMock.mock.calls).toEqual([
      ["tts.custom.base_url", "http://piper.internal/v1/audio/speech"],
      ["tts.custom.model", "tts-1"],
      ["tts.custom.voice", "en_US-lessac-medium"],
      ["tts.custom.format", "wav"],
      ["tts.custom.key_required", false],
    ]);
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
      new Error("config path 'media.custom_stt.base_url' is not user-tunable"),
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
