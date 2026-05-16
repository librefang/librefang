# librefang-runtime-media

Media generation drivers (TTS, image, video, music) for [LibreFang](https://github.com/librefang/librefang) (refs #3710 Phase 1).

Provider-agnostic abstraction mirroring `librefang-llm-drivers`:

- `MediaDriver` trait with per-modality methods; unsupported modalities default to a typed `NotSupported` error.
- `MediaDriverCache` for lazy-init, thread-safe driver caching with per-provider URL overrides.
- Provider impls: `elevenlabs`, `gemini`, `google_tts`, `minimax`, `openai`.
- `media_understanding` — speech-to-text and image/audio analysis routing.

## Where this fits

Extracted from `librefang-runtime` as part of the #3710 god-crate split
(renamed from the deleted `librefang-runtime-oauth`, whose OAuth code
collapsed back into the parent runtime crate). `librefang-runtime`
re-exports this crate at its historical path (`runtime::media`,
`runtime::media_understanding`), so downstream call sites do not need to
switch imports. Behind the parent crate's default-on `media` feature.

See the [workspace README](../../README.md) and `crates/librefang-runtime/README.md`.
