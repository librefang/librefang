import { describe, it, expect } from "vitest";
import {
  buildMediaEndpointWrites,
  mediaEndpointDraftFrom,
  mediaEndpointHasVoiceAndFormat,
  selectMediaModelEndpoints,
} from "./mediaModelEndpoints";
import type { MediaModelEndpoint } from "../api";

// Shaped like the `media` / `tts` sections of `GET /api/config`
// (`redacted_config_json` in crates/librefang-api/src/routes/config/manage.rs).
const fullConfig: Record<string, unknown> = {
  media: {
    audio_transcription: true,
    image_description: true,
    video_description: false,
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
    enabled: false,
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

describe("buildMediaEndpointWrites", () => {
  const stt = () => selectMediaModelEndpoints(fullConfig)[0];

  it("never names api_key_env in any request it issues", () => {
    // The payload scrub (#8085) answers 403 on a credential-shaped key at any
    // depth of a `POST /api/config/set` body, and a wholesale table write would
    // otherwise have to either echo `api_key_env` back or drop it from the
    // file. Per-leaf writes do neither.
    for (const endpoint of selectMediaModelEndpoints(fullConfig)) {
      const writes = buildMediaEndpointWrites(endpoint, {
        base_url: "http://endpoint.internal",
        key_required: true,
        model: "m",
        voice: "alloy",
        format: "mp3",
      });
      expect(writes.every((w) => !w.path.includes("api_key_env"))).toBe(true);
      expect(writes.every((w) => typeof w.value !== "object" || w.value === null)).toBe(true);
    }
  });

  it("writes one leaf per non-secret field under the endpoint table", () => {
    expect(
      buildMediaEndpointWrites(stt(), {
        base_url: "  http://whisper.internal/v1/audio/transcriptions  ",
        key_required: true,
        model: " medium.en ",
      }),
    ).toEqual([
      { path: "media.custom_stt.base_url", value: "http://whisper.internal/v1/audio/transcriptions" },
      { path: "media.custom_stt.model", value: "medium.en" },
      { path: "media.custom_stt.key_required", value: true },
    ]);
  });

  it("ignores an api_key_env smuggled past the draft type", () => {
    const writes = buildMediaEndpointWrites(stt(), {
      base_url: "http://whisper.internal",
      key_required: false,
      model: "",
      ...({ api_key_env: "ANTHROPIC_API_KEY" } as Record<string, string>),
    });
    expect(writes.map((w) => w.path)).toEqual([
      "media.custom_stt.base_url",
      "media.custom_stt.model",
      "media.custom_stt.key_required",
    ]);
  });

  it("clears an empty optional field with null so the default comes back", () => {
    // `config_set` removes the key on a null value; the empty string
    // `json_to_toml_edit_value` would otherwise produce deserializes as
    // Some("") on an Option<String>, and as a literal "" on the non-Option
    // TTS voice / format, which the runtime forwards as `"response_format": ""`.
    const [, tts] = selectMediaModelEndpoints(fullConfig);
    expect(
      buildMediaEndpointWrites(tts, {
        base_url: "http://piper.internal/v1/audio/speech",
        key_required: false,
        model: "   ",
        voice: "",
        format: "  ",
      }),
    ).toEqual([
      { path: "tts.custom.base_url", value: "http://piper.internal/v1/audio/speech" },
      { path: "tts.custom.model", value: null },
      { path: "tts.custom.voice", value: null },
      { path: "tts.custom.format", value: null },
      { path: "tts.custom.key_required", value: false },
    ]);
  });

  it("writes voice and format for TTS and for nothing else", () => {
    const endpoints = selectMediaModelEndpoints(fullConfig);
    const tts = endpoints.find((e) => e.kind === "tts")!;
    const image = endpoints.find((e) => e.kind === "image")!;
    expect(
      buildMediaEndpointWrites(tts, {
        base_url: "http://piper.internal/v1/audio/speech",
        key_required: false,
        model: "tts-1",
        voice: "alloy",
        format: "wav",
      }).map((w) => w.path),
    ).toEqual([
      "tts.custom.base_url",
      "tts.custom.model",
      "tts.custom.voice",
      "tts.custom.format",
      "tts.custom.key_required",
    ]);
    const imagePaths = buildMediaEndpointWrites(image, {
      base_url: "http://llava.internal/v1/chat/completions",
      key_required: false,
      model: "llava",
      voice: "alloy",
      format: "wav",
    }).map((w) => w.path);
    expect(imagePaths).not.toContain("media.custom_image.voice");
    expect(imagePaths).not.toContain("media.custom_image.format");
  });

  it("keeps every leaf inside the write allowlist's accepted depth", () => {
    // `is_writable_config_path` accepts one or two segments after a writable
    // section prefix, and refuses anything ending in a SCRUB_SUFFIXES entry.
    const scrubbed = [".api_key", ".token", ".secret", ".password", ".bypass", ".admin", ".owner", "_env", ".client_id", ".client_secret"];
    for (const endpoint of selectMediaModelEndpoints(fullConfig)) {
      for (const { path } of buildMediaEndpointWrites(endpoint, {
        base_url: "http://endpoint.internal",
        key_required: false,
        model: "m",
        voice: "alloy",
        format: "mp3",
      })) {
        expect(path.split(".")).toHaveLength(3);
        expect(scrubbed.some((suffix) => path.endsWith(suffix))).toBe(false);
      }
    }
  });
});

describe("modality and override warnings", () => {
  it("reports the master switch that makes a complete endpoint inert", () => {
    const endpoints = selectMediaModelEndpoints(fullConfig);
    expect(
      endpoints.map((e) => [e.kind, e.modality_enabled, e.modality_enabled_path]),
    ).toEqual([
      ["stt", true, "media.audio_transcription"],
      // `TtsConfig::default().enabled` is false, so a filled-in `[tts.custom]`
      // synthesises nothing until the flag is flipped.
      ["tts", false, "tts.enabled"],
      ["image", true, "media.image_description"],
      ["video", false, "media.video_description"],
    ]);
  });

  it("treats an absent switch as enabled rather than raising a false alarm", () => {
    for (const e of selectMediaModelEndpoints({})) {
      expect(e.modality_enabled).toBe(true);
    }
  });

  it("reports the [media] scalar that overrides the table's model", () => {
    const withOverride = selectMediaModelEndpoints({
      ...fullConfig,
      media: { ...(fullConfig.media as Record<string, unknown>), audio_model: "whisper-1" },
    });
    const stt = withOverride.find((e) => e.kind === "stt")!;
    expect(stt.model_override).toBe("whisper-1");
    expect(stt.model_override_path).toBe("media.audio_model");
    // Unset in the base fixture, and TTS has no such scalar at all.
    const base = selectMediaModelEndpoints(fullConfig);
    expect(base.find((e) => e.kind === "stt")!.model_override).toBeNull();
    expect(base.find((e) => e.kind === "tts")!.model_override_path).toBeNull();
  });
});

describe("mediaEndpointDraftFrom", () => {
  it("round-trips a configured endpoint through the form and back", () => {
    const endpoint = selectMediaModelEndpoints(fullConfig)[0];
    const writes = buildMediaEndpointWrites(
      endpoint,
      mediaEndpointDraftFrom(endpoint),
    );
    expect(writes).toEqual([
      { path: "media.custom_stt.base_url", value: endpoint.config.base_url },
      { path: "media.custom_stt.model", value: endpoint.config.model },
      { path: "media.custom_stt.key_required", value: endpoint.config.key_required },
    ]);
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
