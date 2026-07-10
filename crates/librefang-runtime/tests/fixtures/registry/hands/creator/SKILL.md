---
name: media-generation-skill
version: "1.0.0"
description: "Expert knowledge for AI media generation — image prompting, video workflows, music composition, and TTS best practices"
runtime: prompt_only
---

# Media Generation Expert Knowledge

## Tool Reference

### image_generate

Generate images from text prompts via OpenAI or MiniMax.

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `prompt` | string | yes | — | Text description of the desired image |
| `provider` | string | no | auto | `openai` or `minimax` |
| `model` | string | no | provider default | `gpt-image-1`, `dall-e-3`, `image-01` |
| `width` | int | no | 1024 | Image width in pixels |
| `height` | int | no | 1024 | Image height in pixels |
| `count` | int | no | 1 | Number of images (1-4) |
| `quality` | string | no | `auto` | `low`, `medium`, `high`, `auto` |
| `seed` | int | no | random | Reproducibility seed |

**Provider-specific notes:**

- **OpenAI gpt-image-1**: Best for photorealistic and creative images. Supports inpainting hints in prompt. Sizes: 1024x1024, 1792x1024, 1024x1792.
- **OpenAI dall-e-3**: Good quality, may revise your prompt (check `revised_prompt` in response). Only generates 1 image per call.
- **MiniMax image-01**: Fast generation, good for illustrations and concept art. Supports arbitrary aspect ratios.

**Result:** Returns `images` array with `url` fields pointing to `/api/uploads/{id}`.

---

### text_to_speech

Convert text to spoken audio.

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `text` | string | yes | — | Text to speak (max ~4096 chars per call) |
| `provider` | string | no | auto | `openai` or `minimax` |
| `model` | string | no | provider default | `tts-1`, `tts-1-hd`, `speech-2.8-hd` |
| `voice` | string | no | `alloy` | Voice selection (see table below) |
| `speed` | float | no | 1.0 | Playback speed (0.25 - 4.0) |
| `format` | string | no | `mp3` | `mp3`, `wav`, `flac`, `opus`, `aac` |

**OpenAI voices:**

| Voice | Character |
|-------|-----------|
| `alloy` | Neutral, balanced |
| `echo` | Male, warm |
| `fable` | Storytelling, expressive |
| `nova` | Female, friendly |
| `onyx` | Deep male, authoritative |
| `shimmer` | Warm female, gentle |

**MiniMax voices:**

| Voice | Character |
|-------|-----------|
| `English_Graceful_Lady` | Female, elegant |
| `English_Calm_Man` | Male, composed |
| `English_Energetic_Girl` | Female, upbeat |

**Tips:**
- For long content, split at paragraph boundaries to keep natural pacing
- `tts-1-hd` is higher quality but slower; use `tts-1` for drafts
- Speed 0.8-0.9 works well for narration; 1.1-1.2 for summaries

**Result:** Returns `url` to the audio file, `format`, `duration_ms`, `sample_rate`.

---

### video_generate

Submit an asynchronous video generation task. Video generation takes 1-3 minutes.

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `prompt` | string | yes | — | Scene description |
| `provider` | string | no | auto | Currently only `minimax` |
| `model` | string | no | `T2V-01` | Video model |
| `duration_secs` | int | no | 5 | Video duration (5-10 seconds) |
| `resolution` | string | no | `1080p` | `720p`, `1080p` |

**Prompt writing for video:**
- Be specific about the scene, subject, and action
- Describe camera movement explicitly: "slow pan left", "zoom in", "static shot"
- Keep it focused — one scene per generation works best
- Include lighting and atmosphere: "golden hour lighting", "neon-lit street at night"
- Avoid complex multi-character interactions (current models handle single subjects best)

**Good prompts:**
- "A golden retriever running through a wheat field at sunset, slow motion, cinematic"
- "Close-up of coffee being poured into a ceramic cup, steam rising, warm morning light"
- "Aerial drone shot flying over a tropical coastline, turquoise water, white sand beach"

**Bad prompts:**
- "A video" (too vague)
- "Two people having a conversation at a cafe while a dog runs by and a car crashes outside" (too complex)

**Result:** Returns `task_id` and `provider`. You MUST poll with `video_status`.

---

### video_status

Poll the status of a video generation task.

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `task_id` | string | yes | From video_generate response |
| `provider` | string | yes | Must match the provider from video_generate |

**Statuses:**

| Status | Meaning | Action |
|--------|---------|--------|
| `pending` | Queued, not started | Wait 10-15s, poll again |
| `processing` | Actively generating | Wait 15-20s, poll again |
| `completed` | Done | Result includes `file_url` |
| `failed` | Generation failed | Check error message, may retry with different prompt |

**Polling pattern:**
1. Call video_generate → get task_id
2. Wait 10 seconds
3. Call video_status with task_id + provider
4. If not completed, wait 15-20 seconds and poll again
5. Maximum ~10 polls (about 3 minutes total)
6. Always inform the user of current status

**Result (completed):** Returns `file_url`, `width`, `height`, `duration_secs`, `provider`, `model`.

---

### music_generate

Generate music from a text prompt and/or lyrics.

**Parameters:**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `prompt` | string | no* | — | Style/mood description |
| `lyrics` | string | no* | — | Song lyrics with structure |
| `provider` | string | no | auto | Currently only `minimax` |
| `model` | string | no | `music-2.5` | Music model |
| `instrumental` | bool | no | false | Generate without vocals |
| `format` | string | no | `mp3` | `mp3`, `wav`, `flac` |

*At least one of `prompt` or `lyrics` is required.

**Prompt writing for music:**

For instrumentals, describe:
- Genre: electronic, jazz, classical, hip-hop, rock, ambient, lo-fi
- Tempo: slow (60-80 BPM), medium (100-120 BPM), fast (130-160 BPM)
- Mood: uplifting, melancholic, energetic, relaxing, dramatic, mysterious
- Instruments: piano, synth, acoustic guitar, strings, drums, bass

For songs with vocals, provide lyrics with structure markers:

```
[Verse 1]
Walking down the empty street
Moonlight dancing at my feet

[Chorus]
This is where the night begins
Let the music pull us in

[Verse 2]
...
```

**Good prompts:**
- `prompt`: "Chill lo-fi hip-hop beat, vinyl crackle, mellow piano chords, 85 BPM, rainy day vibe"
- `prompt`: "Epic orchestral trailer music, building tension, brass and strings, 140 BPM"
- `prompt` + `lyrics`: "Indie folk acoustic ballad, fingerpicking guitar, gentle male vocals" with lyrics

**Result:** Returns `url` to audio file, `format`, `duration_ms`, `sample_rate`.

---

## Combined Workflow Recipes

### Podcast Intro
1. `music_generate` — instrumental jingle, 10-15 seconds, upbeat
2. `text_to_speech` — "Welcome to [show name]..." with energetic voice
3. Report both URLs to user

### Social Media Post
1. `image_generate` — eye-catching visual for the post
2. Suggest caption text based on the image
3. Optionally `text_to_speech` for accessibility audio version

### Video with Narration
1. `text_to_speech` — generate narration audio
2. `video_generate` — generate matching video clip
3. `video_status` — poll until complete
4. Report both URLs (user can combine with ffmpeg or editing tools)

### Album Art + Preview
1. `image_generate` — album cover artwork
2. `music_generate` — short preview track matching the artwork mood
3. Present together

### Audiobook Chapter
1. Split text into sections (~500 words each)
2. `text_to_speech` for each section with consistent voice
3. Report all audio URLs in order

---

## Error Handling

| Error | Cause | Fix |
|-------|-------|-----|
| `missing_key` | API key not configured | Ask user to set OPENAI_API_KEY or MINIMAX_API_KEY |
| `not_supported` | Provider doesn't support this modality | Switch to a provider that does |
| `content_filtered` | Safety filter rejected the prompt | Rephrase without prohibited content |
| `rate_limited` | Too many requests | Wait 30-60 seconds and retry |
| `invalid_request` | Bad parameters | Check parameter ranges (e.g., count 1-4, speed 0.25-4.0) |

---

## Provider Capability Matrix

| Capability | OpenAI | MiniMax |
|------------|--------|---------|
| Image generation | gpt-image-1, dall-e-3 | image-01 |
| Text-to-speech | tts-1, tts-1-hd | speech-2.8-hd |
| Video generation | — | T2V-01, video-01 |
| Music generation | — | music-2.5 |

**Auto-detection priority:** OpenAI > MiniMax (for capabilities both support).
If only MiniMax key is set, all 4 modalities are available through MiniMax.
