Wrap generated Python SDK connection failures in `LibreFangError`. (@houko)
Both ordinary and streaming requests previously leaked `urllib.error.URLError` for failures such as DNS resolution errors, refused connections, and connection timeouts.
Callers can now handle HTTP and connection-level API failures through the SDK's documented error type, with connection failures represented by status `0` and an empty response body.
