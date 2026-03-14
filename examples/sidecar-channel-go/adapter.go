// Example sidecar channel adapter for LibreFang (Go)
//
// Build:
//
//	go build -o adapter adapter.go
//
// Usage in config.toml:
//
//	[[sidecar_channels]]
//	name = "go-echo"
//	command = "./examples/sidecar-channel-go/adapter"
package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
)

type Event struct {
	Method string      `json:"method"`
	Params interface{} `json:"params,omitempty"`
}

type Command struct {
	Method string          `json:"method"`
	Params json.RawMessage `json:"params,omitempty"`
}

type SendParams struct {
	ChannelID string `json:"channel_id"`
	Text      string `json:"text"`
}

type MessageParams struct {
	UserID    string `json:"user_id"`
	UserName  string `json:"user_name"`
	Text      string `json:"text"`
	ChannelID string `json:"channel_id"`
}

func sendEvent(method string, params interface{}) {
	evt := Event{Method: method, Params: params}
	data, _ := json.Marshal(evt)
	fmt.Println(string(data))
}

func main() {
	// Signal readiness
	sendEvent("ready", nil)

	// Read commands from stdin
	scanner := bufio.NewScanner(os.Stdin)
	for scanner.Scan() {
		line := scanner.Text()
		if line == "" {
			continue
		}

		var cmd Command
		if err := json.Unmarshal([]byte(line), &cmd); err != nil {
			sendEvent("error", map[string]string{"message": fmt.Sprintf("Invalid JSON: %v", err)})
			continue
		}

		switch cmd.Method {
		case "send":
			var params SendParams
			json.Unmarshal(cmd.Params, &params)
			sendEvent("message", MessageParams{
				UserID:    "echo-user",
				UserName:  "Echo Bot (Go)",
				Text:      fmt.Sprintf("Echo: %s", params.Text),
				ChannelID: params.ChannelID,
			})
		case "shutdown":
			os.Exit(0)
		}
	}
}
