# @librefang/sdk

Official JavaScript/TypeScript client for the LibreFang Agent OS REST API.

## Installation

```bash
npm install @librefang/sdk
```

## Usage

```javascript
const { LibreFang } = require("@librefang/sdk");

const client = new LibreFang("http://localhost:4545");
```

### Create an Agent

```javascript
const agent = await client.agents.create({ template: "assistant" });
console.log("Agent created:", agent.id);
```

### Send a Message

```javascript
const reply = await client.agents.message(agent.id, "Hello!");
console.log(reply);
```

### Streaming Response

```javascript
for await (const event of client.agents.stream(agent.id, "Tell me a story")) {
  if (event.type === "text_delta") {
    process.stdout.write(event.delta);
  }
}
```

### List Agents

```javascript
const agents = await client.agents.list();
console.log(agents);
```

### More Examples

See the `examples/` directory for more examples:
- `basic.js` - Basic usage
- `streaming.js` - Streaming responses

## API Reference

### Constructor

```javascript
new LibreFang(baseUrl, options)
```

- `baseUrl` - LibreFang server URL (e.g., "http://localhost:4545")
- `options` - Optional configuration
  - `headers` - Extra headers for every request

### Resources

The client provides the following resources:

- `client.agents` - Agent management
- `client.sessions` - Session management
- `client.workflows` - Workflow management
- `client.skills` - Skill management
- `client.channels` - Channel management
- `client.tools` - Tool listing
- `client.models` - Model management
- `client.providers` - Provider management
- `client.memory` - Memory/KV storage
- `client.triggers` - Trigger management
- `client.schedules` - Schedule management

### Server Methods

- `client.health()` - Basic health check
- `client.healthDetail()` - Detailed health
- `client.status()` - Server status
- `client.version()` - Server version
- `client.metrics()` - Prometheus metrics
- `client.usage()` - Usage statistics
- `client.config()` - Server configuration

## TypeScript

This SDK includes TypeScript definitions (`index.d.ts`). Simply import and use:

```typescript
import { LibreFang } from "@librefang/sdk";

const client = new LibreFang("http://localhost:4545");
const agents = await client.agents.list();
```

## License

MIT
