import { describe, it, expect } from "vitest";
import {
  buildMediaEndpointPayload,
  mediaEndpointDraftFrom,
  mediaEndpointHasVoiceAndFormat,
  selectMediaModelEndpoints,
} from "./mediaModelEndpoints";
import type { MediaModelEndpoint } from "../api";

// Shaped like the `media` / `tts` sections of `GET /api/config`
// (`redacted_config_json` in crates/librefang-api/src/routes/config/manage.rs).
const fullConfig: Record<string, unknown> = {
  media: {
    audio_provider: "local-whisper",
    image_provider: "local-llava",
    video_provider: null,
    custom_stt: {
      base_url: "http://localhost:8080/v1/audio/transcriptions",
      api_key_env: "MY_LOCAL_WHISPER_KEY",
      key_required: false,
      model: "large-v3",
    },
    custom_image: {
      base_url: "http://localhost:11434/v1/chat/completions",
      api_key_env: "",
      key_required: false,
      model: "llava",
    },
    custom_video: {
      base_url: "",
      api_key_env: "",
      key_required: false,
      model: null,
    },
  },
  tts: {
    provider: "local-piper",
    custom: {
      base_url: "http://localhost:5000/v1/audio/speech",
      api_key_env: "",
      key_required: false,
      model: "tts-1",
      voice: "en_US-lessac-medium",
      format: "mp3",
    },
  },
};

describe("selectMediaModelEndpoints", () => {
  it("projects all four modalities in a stable order", () => {
    const endpoints = selectMediaModelEndpoints(fullConfig);
    expect(endpoints.map((e) => e.kind)).toEqual(["stt", "tts", "image", "video"]);
  });

  it("maps each modality to the config path that actually holds it", () => {
    const endpoints = selectMediaModelEndpoints(fullConfig);
    expect(
      endpoints.map((e) => [e.kind, e.config_path, e.provider_path]),
    ).toEqual([
      ["stt", "media.custom_stt", "media.audio_provider"],
      // TTS is the odd one out: `[tts.custom]`, not `[media.*]`.
      ["tts", "tts.custom", "tts.provider"],
      ["image", "media.custom_image", "media.image_provider"],
      ["video", "media.custom_video", "media.video_provider"],
    ]);
  });

  it("reads the endpoint table and the provider selector that arms it", () => {
    const [stt] = selectMediaModelEndpoints(fullConfig);
    expect(stt.provider).toBe("local-whisper");
    expect(stt.config).toEqual({
      base_url: "http://localhost:8080/v1/audio/transcriptions",
      api_key_env: "MY_LOCAL_WHISPER_KEY",
      key_required: false,
      model: "large-v3",
    });
    expect(stt.configured).toBe(true);
  });

  it("carries voice and format for TTS only", () => {
    const endpoints = selectMediaModelEndpoints(fullConfig);
    const tts = endpoints.find((e) => e.kind === "tts")!;
    const stt = endpoints.find((e) => e.kind === "stt")!;
    expect(tts.config.voice).toBe("en_US-lessac-medium");
    expect(tts.config.format).toBe("mp3");
    expect(stt.config).not.toHaveProperty("voice");
    expect(stt.config).not.toHaveProperty("format");
    expect(mediaEndpointHasVoiceAndFormat("tts")).toBe(true);
    expect(mediaEndpointHasVoiceAndFormat("stt")).toBe(false);
  });

  it("marks an endpoint with no base URL unconfigured but still lists it", () => {
    const endpoints = selectMediaModelEndpoints(fullConfig);
    const video = endpoints.find((e) => e.kind === "video")!;
    expect(video.configured).toBe(false);
    expect(video.provider).toBe("");
    expect(video.config.model).toBeNull();
  });

  it("returns four unconfigured rows for a config with no media sections", () => {
    for (const input of [undefined, {}, { media: "not-a-table" }]) {
      const endpoints = selectMediaModelEndpoints(
        input as Record<string, unknown> | undefined,
      );
      expect(endpoints).toHaveLength(4);
      expect(endpoints.every((e) => !e.configured)).toBe(true);
    }
  });
});

describe("buildMediaEndpointPayload", () => {
  const stt = () => selectMediaModelEndpoints(fullConfig)[0];

  it("writes the whole table so an untouched key is never dropped", () => {
    const endpoint = stt();
    const payload = buildMediaEndpointPayload(endpoint, {
      base_url: "  http://whisper.internal/v1/audio/transcriptions  ",
      key_required: true,
      model: " medium.en ",
    });
    expect(payload).toEqual({
      base_url: "http://whisper.internal/v1/audio/transcriptions",
      api_key_env: "MY_LOCAL_WHISPER_KEY",
      key_required: true,
      model: "medium.en",
    });
  });

  it("preserves api_key_env from the config rather than from the draft", () => {
    const endpoint = stt();
    const payload = buildMediaEndpointPayload(endpoint, {
      base_url: "http://whisper.internal",
      key_required: false,
      model: "",
      // A draft can never carry an api_key_env; assert the payload is
      // unaffected even when one is smuggled in past the type.
      ...({ api_key_env: "ANTHROPIC_API_KEY" } as Record<string, string>),
    });
    expect(payload.api_key_env).toBe("MY_LOCAL_WHISPER_KEY");
  });

  it("omits an empty model instead of sending an empty string", () => {
    // `json_to_toml_edit_value` turns JSON null into "", which would
    // deserialize as Some("") rather than None on an Option<String>.
    const payload = buildMediaEndpointPayload(stt(), {
      base_url: "http://whisper.internal",
      key_required: false,
      model: "   ",
    });
    expect(payload).not.toHaveProperty("model");
  });

  it("sends voice and format for TTS and for nothing else", () => {
    const endpoints = selectMediaModelEndpoints(fullConfig);
    const tts = endpoints.find((e) => e.kind === "tts")!;
    const image = endpoints.find((e) => e.kind === "image")!;
    expect(
      buildMediaEndpointPayload(tts, {
        base_url: "http://piper.internal/v1/audio/speech",
        key_required: false,
        model: "tts-1",
        voice: "alloy",
        format: "wav",
      }),
    ).toEqual({
      base_url: "http://piper.internal/v1/audio/speech",
      api_key_env: "",
      key_required: false,
      model: "tts-1",
      voice: "alloy",
      format: "wav",
    });
    const imagePayload = buildMediaEndpointPayload(image, {
      base_url: "http://sd.internal/v1/chat/completions",
      key_required: false,
      model: "sdxl",
      voice: "alloy",
      format: "wav",
    });
    expect(imagePayload).not.toHaveProperty("voice");
    expect(imagePayload).not.toHaveProperty("format");
  });
});

describe("mediaEndpointDraftFrom", () => {
  it("round-trips a configured endpoint through the form and back", () => {
    const endpoint = selectMediaModelEndpoints(fullConfig)[0];
    const payload = buildMediaEndpointPayload(
      endpoint,
      mediaEndpointDraftFrom(endpoint),
    );
    expect(payload).toEqual(endpoint.config);
  });

  it("seeds an empty draft for an unconfigured endpoint", () => {
    const video = selectMediaModelEndpoints(fullConfig).find(
      (e) => e.kind === "video",
    ) as MediaModelEndpoint;
    expect(mediaEndpointDraftFrom(video)).toEqual({
      base_url: "",
      key_required: false,
      model: "",
    });
  });
});
