# LibreFang Python SDK

Official Python client and SDK for the LibreFang Agent OS.

## Installation

```bash
pip install librefang-sdk
```

## Public APIs

This package provides two different interfaces:

### 1. REST API Client

Control LibreFang remotely via its REST API.

```python
from librefang import Client

client = Client("http://localhost:4545")

# Create an agent
agent = client.agents.spawn_agent(template="assistant")
agent_id = agent["agent_id"]
print(f"Agent created: {agent_id}")

# Send a message
reply = client.agents.send_message(agent_id, message="Hello!")
print(reply)

# Stream a response
for event in client.agents.send_message_stream(agent_id, message="Tell me a story"):
    if event.get("content"):
        print(event["content"], end="", flush=True)
```

### 2. Agent SDK

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

- Python 3.10+

## License

MIT
