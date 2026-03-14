# Sidecar Channel Adapter — Python Example

This is an example sidecar channel adapter for LibreFang. It demonstrates how to
write a channel adapter in Python (or any language) that communicates with
LibreFang via JSON-RPC over stdin/stdout.

## How It Works

LibreFang spawns your adapter as a subprocess. Communication uses **newline-delimited
JSON** (one JSON object per line):

- **stdout** (adapter -> LibreFang): Events such as incoming messages
- **stdin** (LibreFang -> adapter): Commands such as "send a message"
- **stderr**: Forwarded to LibreFang's logs (use for debugging)

## Protocol

### Events (adapter -> LibreFang via stdout)

#### `ready` — Signal that the adapter is initialized and ready

```json
{"method": "ready"}
```

#### `message` — An incoming message from the platform

```json
{
  "method": "message",
  "params": {
    "user_id": "user-123",
    "user_name": "Alice",
    "text": "Hello!",
    "channel_id": "general",
    "platform": "my-platform"
  }
}
```

Fields `text`, `channel_id`, and `platform` are optional.

#### `error` — Report an error to LibreFang

```json
{"method": "error", "params": {"message": "Connection lost"}}
```

### Commands (LibreFang -> adapter via stdin)

#### `send` — Send a message to the platform

```json
{
  "method": "send",
  "params": {
    "channel_id": "general",
    "text": "Hello from LibreFang!"
  }
}
```

#### `shutdown` — Graceful shutdown request

```json
{"method": "shutdown"}
```

The adapter should clean up and exit.

## Configuration

Add to your `~/.librefang/config.toml`:

```toml
[[sidecar_channels]]
name = "echo-test"
command = "python3"
args = ["path/to/adapter.py"]
# Optional: extra environment variables
env = {}
# Optional: channel type identifier (defaults to the adapter name)
# channel_type = "custom-echo"
```

## Running the Example

```bash
# Test the adapter standalone
echo '{"method":"send","params":{"channel_id":"test","text":"Hello"}}' | python3 adapter.py

# Or configure it in LibreFang and start the daemon
```

## Writing Your Own Adapter

1. Read commands from stdin (one JSON per line)
2. Write events to stdout (one JSON per line, flush after each)
3. Use stderr for debug logging
4. Send `{"method": "ready"}` on stdout when initialized
5. Handle `{"method": "shutdown"}` by cleaning up and exiting
6. Handle `{"method": "send", "params": {...}}` by delivering the message

You can write adapters in any language: Python, Go, Node.js, Ruby, Bash, etc.
