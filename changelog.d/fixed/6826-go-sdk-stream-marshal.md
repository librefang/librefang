Report generated Go SDK stream body encoding failures. (@houko)
The streaming helper previously discarded `json.Marshal` errors and continued with an empty request body, hiding unsupported values from callers.
It now emits a status-`0` error event and closes the stream before constructing or sending an HTTP request.
