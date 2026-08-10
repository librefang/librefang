Always close generated Python SDK streaming responses when iteration ends. (@houko)
The SSE generator previously leaked its HTTP response when it returned on `[DONE]`, the caller stopped iteration early, or a read or decode operation raised before the loop reached the trailing `close()` call.
Response cleanup now lives in `finally`, covering normal EOF, protocol completion, generator close, and exceptional exits while preserving the original event and error semantics.
