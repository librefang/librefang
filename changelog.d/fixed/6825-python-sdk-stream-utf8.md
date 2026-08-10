Preserve split UTF-8 characters in generated Python SDK streams. (@houko)
The SSE reader previously decoded each 4096-byte network chunk independently, so a multibyte character split across reads raised `UnicodeDecodeError` and aborted the stream.
Streaming now buffers raw bytes and decodes only complete SSE lines, making text decoding independent of transport chunk boundaries.
