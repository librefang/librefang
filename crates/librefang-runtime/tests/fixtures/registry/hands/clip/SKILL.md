---
name: clip-hand-skill
version: "2.0.0"
description: "Expert knowledge for AI video clipping — yt-dlp downloading, whisper transcription, SRT generation, and ffmpeg processing"
runtime: prompt_only
---

# Video Clipping Expert Knowledge

## Cross-Platform Notes

All tools (ffmpeg, ffprobe, yt-dlp, whisper) use **identical CLI flags** on Windows, macOS, and Linux. The differences are only in shell syntax:

| Feature | macOS / Linux | Windows (cmd.exe) |
|---------|---------------|-------------------|
| Suppress stderr | `2>/dev/null` | `2>NUL` |
| Filter output | `\| grep pattern` | `\| findstr pattern` |
| Delete files | `rm file1 file2` | `del file1 file2` |
| Null output device | `-f null -` | `-f null -` (same) |
| ffmpeg subtitle paths | `subtitles=clip.srt` | `subtitles=clip.srt` (relative OK, absolute needs `C\\:/path`) |

IMPORTANT: ffmpeg filter paths (`-vf "subtitles=..."`) always need forward slashes. On Windows with absolute paths, escape the colon: `subtitles=C\\:/Users/me/clip.srt`

Prefer using `file_write` tool for creating SRT/text files instead of shell echo/heredoc.

---

## yt-dlp Reference

### Download with Format Selection
```
# Best video up to 1080p + best audio, merged
yt-dlp -f "bv[height<=1080]+ba/b[height<=1080]" --restrict-filenames -o "source.%(ext)s" "URL"

# 720p max (smaller, faster)
yt-dlp -f "bv[height<=720]+ba/b[height<=720]" --restrict-filenames -o "source.%(ext)s" "URL"

# Audio only (for transcription-only workflows)
yt-dlp -x --audio-format wav --restrict-filenames -o "audio.%(ext)s" "URL"
```

### Metadata Inspection
```
# Get full metadata as JSON (duration, title, chapters, available subs)
yt-dlp --dump-json "URL"

# Key fields: duration, title, description, chapters, subtitles, automatic_captions
```

### YouTube Auto-Subtitles
```
# Download auto-generated subtitles in json3 format (word-level timing)
yt-dlp --write-auto-subs --sub-lang en --sub-format json3 --skip-download --restrict-filenames -o "source" "URL"

# Download manual subtitles if available
yt-dlp --write-subs --sub-lang en --sub-format srt --skip-download --restrict-filenames -o "source" "URL"

# List available subtitle languages
yt-dlp --list-subs "URL"
```

### Useful Flags
- `--restrict-filenames` — safe ASCII filenames (no spaces/special chars) — important on all platforms
- `--no-playlist` — download single video even if URL is in a playlist
- `-o "template.%(ext)s"` — output template (%(ext)s auto-detects format)
- `--cookies-from-browser chrome` — use browser cookies for age-restricted content
- `--extract-audio` / `-x` — extract audio only
- `--audio-format wav` — convert audio to wav (for whisper)

---

## Whisper Transcription Reference

### Audio Extraction for Whisper
```
# Extract mono 16kHz WAV (whisper's preferred input format)
ffmpeg -i source.mp4 -vn -ar 16000 -ac 1 -y audio.wav
```

### Basic Transcription
```
# Standard transcription with word-level timestamps
whisper audio.wav --model small --output_format json --word_timestamps true --language en

# Faster alternative (same flags, 4x speed)
whisper-ctranslate2 audio.wav --model small --output_format json --word_timestamps true --language en
```

### Model Sizes
| Model | VRAM | Speed | Quality | Use When |
|-------|------|-------|---------|----------|
| tiny | ~1GB | Fastest | Rough | Quick previews, testing pipeline |
| base | ~1GB | Fast | OK | Short clips, clear speech |
| small | ~2GB | Good | Good | **Default — best balance** |
| medium | ~5GB | Slow | Better | Important content, accented speech |
| large-v3 | ~10GB | Slowest | Best | Final production, multiple languages |

Note: On macOS Apple Silicon, consider `mlx-whisper` as a faster native alternative.

### JSON Output Structure
```json
{
  "text": "full transcript text...",
  "segments": [
    {
      "id": 0,
      "start": 0.0,
      "end": 4.52,
      "text": " Hello everyone, welcome back.",
      "words": [
        {"word": " Hello", "start": 0.0, "end": 0.32, "probability": 0.95},
        {"word": " everyone,", "start": 0.32, "end": 0.78, "probability": 0.91},
        {"word": " welcome", "start": 0.78, "end": 1.14, "probability": 0.98},
        {"word": " back.", "start": 1.14, "end": 1.52, "probability": 0.97}
      ]
    }
  ]
}
```
- `segments[].words[]` gives word-level timing when `--word_timestamps true`
- `probability` indicates confidence (< 0.5 = likely wrong)

---

## YouTube json3 Subtitle Parsing

### Format Structure
```json
{
  "events": [
    {
      "tStartMs": 1230,
      "dDurationMs": 5000,
      "segs": [
        {"utf8": "hello ", "tOffsetMs": 0},
        {"utf8": "world ", "tOffsetMs": 200},
        {"utf8": "how ", "tOffsetMs": 450},
        {"utf8": "are you", "tOffsetMs": 700}
      ]
    }
  ]
}
```

### Extracting Word Timing
For each event and each segment within it:
- `word_start_ms = event.tStartMs + seg.tOffsetMs`
- `word_start_secs = word_start_ms / 1000.0`
- `word_text = seg.utf8.trim()`

Events without `segs` are line breaks or formatting — skip them.
Events with `segs` containing only `"\n"` are newlines — skip them.

---

## SRT Generation from Transcript

### SRT Format
```
1
00:00:00,000 --> 00:00:02,500
First line of caption text

2
00:00:02,500 --> 00:00:05,100
Second line of caption text
```

### Rules for Building Good SRT
- Group words into subtitle lines of ~8-12 words (2-3 seconds per line)
- Break at natural pause points (periods, commas, clause boundaries)
- Keep lines under 42 characters for readability on mobile
- Adjust timestamps relative to clip start (subtract clip start time from all timestamps)
- Timestamp format: `HH:MM:SS,mmm` (comma separator, not dot)
- Each entry: index line, timestamp line, text line(s), blank line
- Use `file_write` tool to create the SRT file — works identically on all platforms

### Styled Captions with ASS Format
For animated/styled captions, use ASS subtitle format instead of SRT:
```
ffmpeg -i clip.mp4 -vf "subtitles=clip.ass:force_style='FontSize=22,FontName=Arial,Bold=1,PrimaryColour=&H00FFFFFF,OutlineColour=&H00000000,Outline=2,Shadow=1,Alignment=2,MarginV=40'" -c:a copy output.mp4
```

Key ASS style properties:
- `PrimaryColour=&H00FFFFFF` — white text (AABBGGRR format)
- `OutlineColour=&H00000000` — black outline
- `Outline=2` — outline thickness
- `Alignment=2` — bottom center
- `MarginV=40` — margin from bottom edge
- `FontSize=22` — good size for 1080x1920 vertical

---

## FFmpeg Video Processing

### Scene Detection
```
ffmpeg -i input.mp4 -filter:v "select='gt(scene,0.3)',showinfo" -f null - 2>&1
```
- Threshold 0.1 = very sensitive, 0.5 = only major cuts
- Parse `pts_time:` from showinfo output for timestamps
- On macOS/Linux pipe through `grep showinfo`, on Windows pipe through `findstr showinfo`

### Silence Detection
```
ffmpeg -i input.mp4 -af "silencedetect=noise=-30dB:d=1.5" -f null - 2>&1
```
- `d=1.5` = minimum 1.5 seconds of silence
- Look for `silence_start` and `silence_end` in output

### Clip Extraction
```
# Re-encoded (accurate cuts)
ffmpeg -ss 00:01:30 -to 00:02:15 -i input.mp4 -c:v libx264 -c:a aac -preset fast -crf 23 -movflags +faststart -y clip.mp4

# Lossless copy (fast but may have keyframe alignment issues)
ffmpeg -ss 00:01:30 -to 00:02:15 -i input.mp4 -c copy -y clip.mp4
```
- `-ss` before `-i` = fast seek (recommended for extraction)
- `-to` = end timestamp, `-t` = duration

### Vertical Video (9:16 for Shorts/Reels/TikTok)
```
# Center crop (when source is 16:9)
ffmpeg -i input.mp4 -vf "crop=ih*9/16:ih:(iw-ih*9/16)/2:0,scale=1080:1920" -c:a copy output.mp4

# Scale with letterbox padding (preserves full frame)
ffmpeg -i input.mp4 -vf "scale=1080:1920:force_original_aspect_ratio=decrease,pad=1080:1920:(ow-iw)/2:(oh-ih)/2:black" -c:a copy output.mp4
```

### Caption Burn-in
```
# SRT subtitles with styling (use relative path or forward-slash absolute path)
ffmpeg -i input.mp4 -vf "subtitles=subs.srt:force_style='FontSize=22,FontName=Arial,PrimaryColour=&H00FFFFFF,OutlineColour=&H00000000,Outline=2,Alignment=2,MarginV=40'" -c:a copy output.mp4

# Simple text overlay
ffmpeg -i input.mp4 -vf "drawtext=text='Caption':fontsize=48:fontcolor=white:borderw=3:bordercolor=black:x=(w-text_w)/2:y=h-th-40" output.mp4
```
Windows path escaping: `subtitles=C\\:/Users/me/subs.srt` (double-backslash before colon)

### Thumbnail Generation
```
# At specific time (2 seconds in)
ffmpeg -i input.mp4 -ss 2 -frames:v 1 -q:v 2 -y thumb.jpg

# Best keyframe
ffmpeg -i input.mp4 -vf "select='eq(pict_type,I)',scale=1280:720" -frames:v 1 thumb.jpg

# Contact sheet
ffmpeg -i input.mp4 -vf "fps=1/10,scale=320:-1,tile=4x4" contact.jpg
```

### Video Analysis
```
# Full metadata (JSON)
ffprobe -v quiet -print_format json -show_format -show_streams input.mp4

# Duration only
ffprobe -v error -show_entries format=duration -of csv=p=0 input.mp4

# Resolution
ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0 input.mp4
```

## API-Based STT Reference

### Groq Whisper API
Fastest cloud STT — uses whisper-large-v3 on Groq hardware. Free tier available.
```
curl -s -X POST "https://api.groq.com/openai/v1/audio/transcriptions" \
  -H "Authorization: Bearer $GROQ_API_KEY" \
  -H "Content-Type: multipart/form-data" \
  -F "file=@audio.wav" \
  -F "model=whisper-large-v3" \
  -F "response_format=verbose_json" \
  -F "timestamp_granularities[]=word" \
  -o transcript_raw.json
```
Response: `{"text": "...", "words": [{"word": "hello", "start": 0.0, "end": 0.32}]}`
- Max file size: 25MB. For longer audio, split with ffmpeg first.
- `timestamp_granularities[]=word` is required for word-level timing.

### OpenAI Whisper API
```
curl -s -X POST "https://api.openai.com/v1/audio/transcriptions" \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H "Content-Type: multipart/form-data" \
  -F "file=@audio.wav" \
  -F "model=whisper-1" \
  -F "response_format=verbose_json" \
  -F "timestamp_granularities[]=word" \
  -o transcript_raw.json
```
Response format same as Groq. Max 25MB.

### Deepgram Nova-2
```
curl -s -X POST "https://api.deepgram.com/v1/listen?model=nova-2&smart_format=true&utterances=true&punctuate=true" \
  -H "Authorization: Token $DEEPGRAM_API_KEY" \
  -H "Content-Type: audio/wav" \
  --data-binary @audio.wav \
  -o transcript_raw.json
```
Response: `{"results": {"channels": [{"alternatives": [{"words": [{"word": "hello", "start": 0.0, "end": 0.32, "confidence": 0.99}]}]}]}}`
- Supports streaming, but for clips use batch mode.
- `smart_format=true` adds punctuation and casing.

---

## TTS Reference

### Edge TTS (free, no API key needed)
```
# List available voices
edge-tts --list-voices

# Generate speech
edge-tts --text "Your caption text here" --voice en-US-AriaNeural --write-media tts_output.mp3

# Other good voices: en-US-GuyNeural, en-GB-SoniaNeural, en-AU-NatashaNeural
```
Install: `pip install edge-tts`

### OpenAI TTS
```
curl -s -X POST "https://api.openai.com/v1/audio/speech" \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"tts-1","input":"Your text here","voice":"alloy"}' \
  --output tts_output.mp3
```
Voices: `alloy`, `echo`, `fable`, `onyx`, `nova`, `shimmer`
Models: `tts-1` (fast), `tts-1-hd` (quality)

### ElevenLabs
```
curl -s -X POST "https://api.elevenlabs.io/v1/text-to-speech/21m00Tcm4TlvDq8ikWAM" \
  -H "xi-api-key: $ELEVENLABS_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"text":"Your text here","model_id":"eleven_monolingual_v1"}' \
  --output tts_output.mp3
```
Voice ID `21m00Tcm4TlvDq8ikWAM` = Rachel (default). List voices: `GET /v1/voices`

### Audio Merging (TTS + Original)
```
# Mix TTS over original audio (original at 30% volume, TTS at 100%)
ffmpeg -i clip.mp4 -i tts.mp3 \
  -filter_complex "[0:a]volume=0.3[orig];[1:a]volume=1.0[tts];[orig][tts]amix=inputs=2:duration=first[out]" \
  -map 0:v -map "[out]" -c:v copy -c:a aac -y clip_voiced.mp4

# Replace audio entirely (no original audio)
ffmpeg -i clip.mp4 -i tts.mp3 -map 0:v -map 1:a -c:v copy -c:a aac -shortest -y clip_voiced.mp4
```

---

## Quality & Performance Tips

- Use `-preset ultrafast` for quick previews, `-preset slow` for final output
- Use `-crf 23` for good quality (18=high, 28=low, lower=bigger files)
- Add `-movflags +faststart` for web-friendly MP4
- Use `-threads 0` to auto-detect CPU cores
- Always use `-y` to overwrite without asking

---

## Telegram Bot API Reference

### sendVideo — Upload and send a video to a chat/channel
```
curl -s -X POST "https://api.telegram.org/bot<BOT_TOKEN>/sendVideo" \
  -F "chat_id=<CHAT_ID>" \
  -F "video=@clip_N_final.mp4" \
  -F "caption=Clip title here" \
  -F "parse_mode=HTML" \
  -F "supports_streaming=true"
```

### Parameters
| Parameter | Required | Description |
|-----------|----------|-------------|
| `chat_id` | Yes | Channel (`-100XXXXXXXXXX` or `@channelname`), group, or user numeric ID |
| `video` | Yes | `@filepath` for upload (max 50MB) or a Telegram `file_id` for re-send |
| `caption` | No | Text caption, up to 1024 characters |
| `parse_mode` | No | `HTML` or `MarkdownV2` for styled captions |
| `supports_streaming` | No | `true` enables progressive playback |

### Success Response
```json
{"ok": true, "result": {"message_id": 1234, "video": {"file_id": "BAACAgI...", "file_size": 5242880}}}
```

### Error Response
```json
{"ok": false, "error_code": 400, "description": "Bad Request: chat not found"}
```

### Common Errors
| Error Code | Description | Fix |
|------------|-------------|-----|
| 400 | Chat not found | Verify chat_id; bot must be added to the channel/group |
| 401 | Unauthorized | Bot token is invalid or revoked — regenerate via @BotFather |
| 413 | Request entity too large | File exceeds 50MB — re-encode: `ffmpeg -i input.mp4 -fs 49M -c:v libx264 -crf 28 -preset fast -c:a aac -y output.mp4` |
| 429 | Too many requests | Rate limited — wait the `retry_after` seconds from the response |

### File Size Limit
Telegram allows up to **50MB** for video uploads via Bot API. If a clip exceeds this:
```
ffmpeg -i clip_N_final.mp4 -fs 49M -c:v libx264 -crf 28 -preset fast -c:a aac -movflags +faststart -y clip_N_tg.mp4
```

---

## WhatsApp Business Cloud API Reference

### Two-Step Flow: Upload Media → Send Message

WhatsApp Cloud API requires uploading the video first to get a `media_id`, then sending a message referencing that ID.

### Step 1 — Upload Media
```
curl -s -X POST "https://graph.facebook.com/v21.0/<PHONE_NUMBER_ID>/media" \
  -H "Authorization: Bearer <ACCESS_TOKEN>" \
  -F "file=@clip_N_final.mp4" \
  -F "type=video/mp4" \
  -F "messaging_product=whatsapp"
```

Success response:
```json
{"id": "1234567890"}
```

### Step 2 — Send Video Message
```
curl -s -X POST "https://graph.facebook.com/v21.0/<PHONE_NUMBER_ID>/messages" \
  -H "Authorization: Bearer <ACCESS_TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{
    "messaging_product": "whatsapp",
    "to": "<RECIPIENT_PHONE>",
    "type": "video",
    "video": {
      "id": "<MEDIA_ID>",
      "caption": "Clip title here"
    }
  }'
```

Success response:
```json
{"messaging_product": "whatsapp", "contacts": [{"wa_id": "14155551234"}], "messages": [{"id": "wamid.HBgL..."}]}
```

### File Size Limit
WhatsApp allows up to **16MB** for video uploads. If a clip exceeds this:
```
ffmpeg -i clip_N_final.mp4 -fs 15M -c:v libx264 -crf 30 -preset fast -c:a aac -movflags +faststart -y clip_N_wa.mp4
```

### 24-Hour Messaging Window
WhatsApp requires the recipient to have messaged you within the last 24 hours (for non-template messages). If you get a "template required" error, either:
- Ask the recipient to send any message to the business number first
- Use a pre-approved message template instead of a free-form video message

### Common Errors
| Error Code | Description | Fix |
|------------|-------------|-----|
| 100 | Invalid parameter | Check phone_number_id and recipient format (no + prefix, no spaces) |
| 190 | Invalid/expired access token | Regenerate token in Meta Business Settings; temporary tokens expire in 24h |
| 131030 | Recipient not in allowed list | In test mode, add recipient to allowed numbers in Meta Developer Portal |
| 131047 | Re-engagement message / template required | Recipient hasn't messaged within 24h — use a template or ask them to message first |
| 131053 | Media upload failed | File too large or unsupported format — re-encode as MP4 under 16MB |
