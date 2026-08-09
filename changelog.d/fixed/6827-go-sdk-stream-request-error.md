Handle generated Go SDK stream request-construction failures. (@houko)
The streaming helper previously ignored `http.NewRequest` errors and dereferenced a nil request, allowing malformed methods or URLs to panic its goroutine and terminate the process.
It now emits a status-`0` error event and closes the stream before accessing the invalid request.
