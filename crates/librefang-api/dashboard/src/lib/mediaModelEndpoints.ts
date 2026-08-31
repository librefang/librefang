import type {
  MediaModelEndpoint,
  MediaModelEndpointConfig,
  MediaModelEndpointDraft,
  MediaModelKind,
} from "../api";

/**
 * Where each custom media endpoint lives in `config.toml` (refs #8038, #8011).
 *
 * Deliberately a projection, not a migration: the tables stay exactly where
 * they are — three under `[media]`, one under `[tts]` — and the Models tab
 * reads and writes them through the config API that already serves them
 * (`GET /api/config`, `POST /api/config/set`).
 *
 * `provider_path` is the scalar that decides whether the table is consulted at
 * all: the runtime only falls through to `custom_*` when the selected provider
 * name is not one of the built-ins, so an endpoint edited without setting it
 * would be inert.
 *
 * The two shapes are NOT identical. `CustomSttConfig` / `CustomImageConfig` /
 * `CustomVideoConfig` carry `base_url`, `api_key_env`, `key_required` and an
 * `Option<String>` model; `CustomTtsConfig` carries the same four with a
 * non-optional `model` plus `voice` and `format`.
 */
export const MEDIA_ENDPOINT_DESCRIPTORS: readonly {
  kind: MediaModelKind;
  /** Section of the redacted `GET /api/config` payload holding the table. */
  section: "media" | "tts";
  /** Key of the table inside that section. */
  table: string;
  /** Key of the provider selector inside that section. */
  providerKey: string;
  /** TTS is the only kind with voice / format. */
  hasVoiceAndFormat: boolean;
}[] = [
  { kind: "stt", section: "media", table: "custom_stt", providerKey: "audio_provider", hasVoiceAndFormat: false },
  { kind: "tts", section: "tts", table: "custom", providerKey: "provider", hasVoiceAndFormat: true },
  { kind: "image", section: "media", table: "custom_image", providerKey: "image_provider", hasVoiceAndFormat: false },
  { kind: "video", section: "media", table: "custom_video", providerKey: "video_provider", hasVoiceAndFormat: false },
];

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function asString(value: unknown): string {
  return typeof value === "string" ? value : "";
}

/** Whether this kind carries `voice` / `format` (TTS only). */
export function mediaEndpointHasVoiceAndFormat(kind: MediaModelKind): boolean {
  return MEDIA_ENDPOINT_DESCRIPTORS.some(
    (d) => d.kind === kind && d.hasVoiceAndFormat,
  );
}

/**
 * Project the redacted `GET /api/config` payload into one row per media
 * endpoint, in a fixed order so the Models tab does not reshuffle between
 * refetches.
 *
 * Always returns all four rows, configured or not: an operator looking for
 * "where do I point image generation at my local Stable Diffusion" needs to
 * see the empty slot, which is the whole complaint in #8011.
 */
export function selectMediaModelEndpoints(
  config: Record<string, unknown> | undefined,
): MediaModelEndpoint[] {
  const root = asRecord(config);
  return MEDIA_ENDPOINT_DESCRIPTORS.map((descriptor) => {
    const section = asRecord(root[descriptor.section]);
    const table = asRecord(section[descriptor.table]);
    const endpointConfig: MediaModelEndpointConfig = {
      base_url: asString(table.base_url),
      api_key_env: asString(table.api_key_env),
      key_required: table.key_required === true,
      model: typeof table.model === "string" ? table.model : null,
      ...(descriptor.hasVoiceAndFormat
        ? { voice: asString(table.voice), format: asString(table.format) }
        : {}),
    };
    return {
      kind: descriptor.kind,
      config_path: `${descriptor.section}.${descriptor.table}`,
      provider_path: `${descriptor.section}.${descriptor.providerKey}`,
      provider: asString(section[descriptor.providerKey]),
      config: endpointConfig,
      configured: (endpointConfig.base_url ?? "").trim().length > 0,
    };
  });
}

/**
 * Build the wholesale table `POST /api/config/set` writes back.
 *
 * The write allowlist accepts `media.` / `tts.` paths one or two segments deep
 * (`is_writable_config_path` in `crates/librefang-api/src/routes/config/mod.rs`),
 * so `media.custom_stt.base_url` is three segments and rejected — the table has
 * to go over as one object. That means every key the operator is *not* editing
 * must be carried across explicitly or `toml_edit` drops it when it replaces the
 * inline table.
 *
 * `api_key_env` is echoed back exactly as read rather than taken from the draft:
 * the dashboard shows the env-var name but does not offer to repoint it, which
 * keeps this write from becoming the env-var-redirect the leaf-level
 * `SCRUB_SUFFIXES` `_env` rule exists to block.
 *
 * An empty `model` is omitted rather than sent as `""`, because
 * `json_to_toml_edit_value` maps JSON null to an empty string and
 * `Option<String>` would then deserialize as `Some("")` instead of `None`.
 */
export function buildMediaEndpointPayload(
  endpoint: MediaModelEndpoint,
  draft: MediaModelEndpointDraft,
): MediaModelEndpointConfig {
  const payload: MediaModelEndpointConfig = {
    base_url: draft.base_url.trim(),
    // Preserved from the current config, never from operator input.
    api_key_env: endpoint.config.api_key_env ?? "",
    key_required: draft.key_required,
  };
  const model = draft.model.trim();
  if (model) payload.model = model;
  if (mediaEndpointHasVoiceAndFormat(endpoint.kind)) {
    payload.voice = (draft.voice ?? "").trim();
    payload.format = (draft.format ?? "").trim();
  }
  return payload;
}

/** Seed an edit form from the endpoint the server currently reports. */
export function mediaEndpointDraftFrom(
  endpoint: MediaModelEndpoint,
): MediaModelEndpointDraft {
  return {
    base_url: endpoint.config.base_url ?? "",
    key_required: endpoint.config.key_required === true,
    model: endpoint.config.model ?? "",
    ...(mediaEndpointHasVoiceAndFormat(endpoint.kind)
      ? {
          voice: endpoint.config.voice ?? "",
          format: endpoint.config.format ?? "",
        }
      : {}),
  };
}
