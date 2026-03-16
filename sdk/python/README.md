# LibreFang Python SDK

Official Python client and SDK for the LibreFang Agent OS.

## Installation

```bash
pip install librefang
```

## Two Packages

This package provides two different interfaces:

### 1. REST API Client (`librefang.client`)

Control LibreFang remotely via its REST API.

```python
from librefang import Client

client = Client("http://localhost:4545")

# Create an agent
agent = client.agents.create(template="assistant")
print(f"Agent created: {agent['id']}")

# Send a message
reply = client.agents.message(agent["id"], "Hello!")
print(reply)

# Stream a response
for event in client.agents.stream(agent["id"], "Tell me a story"):
    if event.get("type") == "text_delta":
        print(event["delta"], end="", flush=True)
```

### 2. Agent SDK (`librefang.sdk`)

Write Python agents that run inside LibreFang.

```python
from librefang import Agent

agent = Agent()

@agent.on_message
def handle(message: str, context: dict) -> str:
    return f"You said: {message}"

agent.run()
```

Or use the simple input/output functions:

```python
from librefang import read_input, respond

data = read_input()
result = f"Echo: {data['message']}"
respond(result)
```

## Examples

See the `examples/` directory for more examples:

### Client Examples
- `client_basic.py` - Basic REST API usage
- `client_streaming.py` - Streaming responses

### SDK Examples
- `echo_agent.py` - Simple echo agent

## Requirements

- Python 3.8+

## License

MIT
