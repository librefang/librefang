# LibreFang Go SDK

Official Go client for the LibreFang Agent OS REST API.

## Installation

```bash
go get github.com/librefang/librefang/sdk/go
```

## Usage

```go
package main

import (
	"fmt"
	"log"

	"github.com/librefang/librefang/sdk/go"
)

func main() {
	client := librefang.New("http://localhost:4545")

	// Create an agent
	agent, err := client.Agents.Create(map[string]interface{}{
		"template": "assistant",
	})
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println("Agent:", agent["id"])

	// Send a message
	reply, err := client.Agents.Message(agent["id"].(string), "Hello!")
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println("Reply:", reply)

	// Stream a response
	for event := range client.Agents.Stream(agent["id"].(string), "Tell me a joke") {
		if text, ok := event["delta"].(string); ok {
			fmt.Print(text)
		}
	}

	// Clean up
	client.Agents.Delete(agent["id"].(string))
}
```

## API Reference

### Client

- `librefang.New(baseURL)` - Create a new client

### Agent Operations

- `client.Agents.List()` - List all agents
- `client.Agents.Create(params)` - Create a new agent
- `client.Agents.Get(id)` - Get agent by ID
- `client.Agents.Delete(id)` - Delete an agent
- `client.Agents.Message(id, text)` - Send a message
- `client.Agents.Stream(id, text)` - Stream a response

### Other Resources

- `client.Sessions` - Session management
- `client.Workflows` - Workflow operations
- `client.Skills` - Skill management
- `client.Channels` - Channel configuration
- `client.Tools` - Tool listing
- `client.Models` - Model management
- `client.Providers` - Provider configuration
- `client.Memory` - Agent memory (KV store)
- `client.Triggers` - Trigger management
- `client.Schedules` - Schedule management

## License

MIT
