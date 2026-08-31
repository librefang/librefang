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
 * `enabled_key` is the modality's master switch, and it is a second way for a
 * complete endpoint to be inert — `TtsService::synthesize` returns early on
 * `!self.config.enabled` (`crates/librefang-runtime/src/tts.rs`) and
 * `describe_video` on `!self.config.video_description`
 * (`crates/librefang-runtime-media/src/media_understanding.rs`), and both
 * default to `false`. The tab surfaces the flag rather than writing it: arming
 * a global modality is not a side effect a "save this endpoint" button should
 * have.
 *
 * `model_override_key` is the `[media]` scalar that *wins* over the table's own
 * `model`: model resolution is `config.audio_model.or(custom_stt_model_ref(…))`,
 * so with `[media] audio_model` set, the `Model` field edited here has no
 * effect. TTS has no such scalar — `synthesize_custom` reads `cfg.model`
 * directly.
 *
 * The four shapes are NOT identical. `CustomSttConfig` / `CustomImageConfig` /
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
  /** Key of the modality's master on/off switch inside that section. */
  enabledKey: string;
  /** Key of the `[media]` scalar that overrides the table's `model`, if any. */
  modelOverrideKey: string | null;
  /** TTS is the only kind with voice / format. */
  hasVoiceAndFormat: boolean;
}[] = [
  {
    kind: "stt",
    section: "media",
    table: "custom_stt",
    providerKey: "audio_provider",
    enabledKey: "audio_transcription",
    modelOverrideKey: "audio_model",
    hasVoiceAndFormat: false,
  },
  {
    kind: "tts",
    section: "tts",
    table: "custom",
    providerKey: "provider",
    enabledKey: "enabled",
    modelOverrideKey: null,
    hasVoiceAndFormat: true,
  },
  {
    kind: "image",
    section: "media",
    table: "custom_image",
    providerKey: "image_provider",
    enabledKey: "image_description",
    modelOverrideKey: "image_model",
    hasVoiceAndFormat: false,
  },
  {
    kind: "video",
    section: "media",
    table: "custom_video",
    providerKey: "video_provider",
    enabledKey: "video_description",
    modelOverrideKey: "video_model",
    hasVoiceAndFormat: false,
  },
];

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function asString(value: unknown): string {
  return typeof value === "string" ? value : "";
}

/** A non-empty string, or `null` for absent / blank / wrong-typed. */
function asOptionalString(value: unknown): string | null {
  const s = asString(value).trim();
  return s === "" ? null : s;
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
 * "where do I point image description at my local llava" needs to see the
 * empty slot, which is the whole complaint in #8011.
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
      // Absent reads as enabled, the way an absent key reads as unrestricted
      // everywhere else in this codebase: only an explicit `false` from the
      // daemon is evidence the modality is off, and a partial payload must not
      // raise a false alarm.
      modality_enabled: section[descriptor.enabledKey] !== false,
      modality_enabled_path: `${descriptor.section}.${descriptor.enabledKey}`,
      model_override: descriptor.modelOverrideKey
        ? asOptionalString(section[descriptor.modelOverrideKey])
        : null,
      model_override_path: descriptor.modelOverrideKey
        ? `${descriptor.section}.${descriptor.modelOverrideKey}`
        : null,
    };
  });
}

/** One `POST /api/config/set` a media-endpoint save issues. */
export type MediaEndpointWrite = {
  path: string;
  value: string | boolean | null;
};

/**
 * Build the per-leaf `POST /api/config/set` writes one media-endpoint save
 * issues.
 *
 * **Per-leaf, not one wholesale table, and that is load-bearing twice over.**
 *
 * `config_set` assigns a depth-2 path wholesale —
 * `doc[parts[0]][parts[1]] = Item::Value(json_to_toml_edit_value(&value))` in
 * `crates/librefang-api/src/routes/config/manage.rs` — so posting
 * `media.custom_stt` as an object *replaces the whole table*. That leaves only
 * two options for a form that deliberately does not edit `api_key_env`: echo
 * the stored value back inside the payload, or drop the operator's env-var name
 * from `config.toml`. Both are wrong.
 *
 * - Echoing it back is refused as of the payload scrub (PR #8085): the scan
 *   walks the submitted JSON and answers `403` on a credential-shaped key at
 *   any depth, `api_key_env` included. A wholesale save would simply stop
 *   working.
 * - Omitting it deletes it, silently unauthenticating an endpoint that was
 *   working.
 *
 * Writing each non-secret leaf on its own path avoids the dilemma entirely:
 * `media.custom_stt.base_url` is two segments after the `media.` prefix, which
 * `is_writable_config_path` accepts (same shape as the `channels.telegram.enabled`
 * case its own test asserts writable), none of the leaf names trip
 * `SCRUB_SUFFIXES`, `api_key_env` is never named in a request at all, and the
 * key the form does not edit is simply left alone on disk. It is also the only
 * shape that keeps `manage.rs`'s advertised "preserves comments, whitespace and
 * key ordering" contract: the wholesale write turns a documented
 * `[media.custom_stt]` standard table into an inline table and drops the
 * commented-out `# api_key_env = …` guidance the struct docs tell operators to
 * put there.
 *
 * An empty optional field is sent as JSON `null`, which `config_set` handles by
 * *removing* the key (`is_remove = value.is_null()`) rather than writing the
 * empty string `json_to_toml_edit_value` would otherwise produce. That is what
 * makes clearing a field restore the default instead of persisting `""`:
 * `model` returns to `None` (or `"tts-1"` for TTS), and `voice` / `format`
 * return to `"alloy"` / `"mp3"` instead of being forwarded to the endpoint as
 * `"response_format": ""`.
 */
export function buildMediaEndpointWrites(
  endpoint: MediaModelEndpoint,
  draft: MediaModelEndpointDraft,
): MediaEndpointWrite[] {
  const leaf = (key: string) => `${endpoint.config_path}.${key}`;
  const optional = (key: string, raw: string | undefined): MediaEndpointWrite => {
    const value = (raw ?? "").trim();
    return { path: leaf(key), value: value === "" ? null : value };
  };

  const writes: MediaEndpointWrite[] = [
    { path: leaf("base_url"), value: draft.base_url.trim() },
    optional("model", draft.model),
  ];
  if (mediaEndpointHasVoiceAndFormat(endpoint.kind)) {
    writes.push(optional("voice", draft.voice), optional("format", draft.format));
  }
  writes.push({ path: leaf("key_required"), value: draft.key_required });
  return writes;
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
