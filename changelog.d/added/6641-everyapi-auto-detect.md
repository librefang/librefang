Auto-detect a locally installed and logged-in EveryAPI CLI and expose it as an LLM provider without ever copying its relay key into a LibreFang-owned file.
Credentials are resolved per request through EveryAPI's own credential-process command and refreshed once after an HTTP 401, so EveryAPI remains the authority for key selection, OAuth refresh, and region resolution.
Explicit provider keys, URLs, and user suppression still take precedence over auto-detection, and `librefang doctor` reports the detected wiring and any conflicting configuration (#6641) (@houko)
