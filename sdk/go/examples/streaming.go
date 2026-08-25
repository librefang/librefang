//go:build ignore

package main

import (
	"fmt"
	"log"

	"github.com/librefang/librefang/sdk/go"
)

func main() {
	if err := run(); err != nil {
		log.Fatal(err)
	}
}

func run() (err error) {
	client := librefang.New("http://localhost:4545")

	raw, err := client.Agents.ListAgents(nil)
	if err != nil {
		return fmt.Errorf("list agents: %w", err)
	}
	items, ok := librefang.ToMap(raw)["items"]
	if !ok {
		return fmt.Errorf("list agents response is missing items")
	}
	agents := librefang.ToSlice(items)

	var agentID string
	if len(agents) > 0 {
		id, ok := agents[0]["id"].(string)
		if !ok || id == "" {
			return fmt.Errorf("existing agent entry is missing a valid id")
		}
		agentID = id
		fmt.Println("Using existing agent:", agentID)
	} else {
		agent, err := client.Agents.SpawnAgent(map[string]interface{}{
			"template": "assistant",
		})
		if err != nil {
			return fmt.Errorf("spawn agent: %w", err)
		}
		id, ok := librefang.ToMap(agent)["agent_id"].(string)
		if !ok || id == "" {
			return fmt.Errorf("spawned agent response is missing a valid agent_id")
		}
		agentID = id
		fmt.Println("Created agent:", agentID)
		defer func() {
			_, cleanupErr := client.Agents.KillAgent(
				agentID,
				map[string]string{"confirm": "true"},
			)
			if cleanupErr != nil {
				if err == nil {
					err = fmt.Errorf("delete created agent: %w", cleanupErr)
				} else {
					log.Printf("warning: failed to delete created agent: %v", cleanupErr)
				}
				return
			}
			fmt.Println("Agent deleted.")
		}()
	}

	fmt.Println("\n--- Streaming response ---")
	sawDone := false
	for event := range client.Agents.SendMessageStream(agentID, map[string]interface{}{
		"message": "Say hello in 3 words.",
	}) {
		if errMessage, ok := event["error"].(string); ok {
			return fmt.Errorf("stream failed: %s", errMessage)
		}
		if content, ok := event["content"].(string); ok {
			fmt.Print(content)
		}
		if done, ok := event["done"].(bool); ok && done {
			sawDone = true
			fmt.Println("\n--- Done ---")
		}
	}
	if !sawDone {
		return fmt.Errorf("stream ended before the done event")
	}
	return nil
}
