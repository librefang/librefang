A streaming turn killed by a provider failure no longer vanishes from the chat without a trace.
Until now the operator's message was the last thing in the session: the turn returned an error to its caller, the history recorded nothing, and neither the open chat nor a reload explained where the answer went — the shape observed live when the provider circuit breaker opened mid-stream.
The session now keeps a short note saying the provider failed and that no response was produced.
The note is deliberately opaque and carries none of the driver's error text, which goes to the daemon log instead, because a provider error's `Display` routinely drags along the endpoint URL, the model id and the upstream response body.
It is stored with the `system` role rather than the assistant's, so the next turn reads it as a fact about the daemon instead of as something the agent itself said. (#7989) (@DaBlitzStein)
