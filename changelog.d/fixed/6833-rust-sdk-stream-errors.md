Surface generated Rust SDK stream transport failures. (@houko)
The SSE reader previously stopped silently when a response body chunk returned an error, making truncated connections indistinguishable from clean stream completion.
It now emits a status-`0` `stream error` event before closing the channel, while preserving any valid events received before the failure.
