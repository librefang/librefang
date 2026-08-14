Scrub internal media driver details from HTTP 500 responses while preserving actionable client errors and retaining the full internal cause in server logs.
Generated uploads now derive PNG, JPEG, GIF, or WebP metadata from their byte signatures instead of labeling every image as PNG, and unknown signatures remain inline rather than being persisted under a false type.
Video polling rejects unknown providers, upstream non-client statuses map to Bad Gateway, and transcription temporary files are removed even when the handler future is cancelled (#7085) (@xiaomo)
