//go:build ignore

package main

import (
	"fmt"
	"log"

	"github.com/librefang/librefang/sdk/go"
)

func main() {
	client := librefang.New("http://localhost:4545")

	raw, err := client.Agents.ListAgents(nil)
	if err != nil {
		log.Fatal(err)
	}
	agents := librefang.ToSlice(raw)

	var agentID string
	if len(agents) > 0 {
		id, ok := agents[0]["id"].(string)
		if !ok || id == "" {
			log.Fatal("existing agent entry is missing a valid id")
		}
		agentID = id
		fmt.Println("Using existing agent:", agentID)
	} else {
		agent, err := client.Agents.SpawnAgent(map[string]interface{}{
			"template": "assistant",
		})
		if err != nil {
			log.Fatal(err)
		}
		id, ok := librefang.ToMap(agent)["id"].(string)
		if !ok || id == "" {
			log.Fatal("spawned agent response is missing a valid id")
		}
		agentID = id
		fmt.Println("Created agent:", agentID)
	}

	fmt.Println("\n--- Streaming response ---")
	for event := range client.Agents.SendMessageStream(agentID, map[string]interface{}{
		"message": "Say hello in 3 words.",
	}) {
		if errMessage, ok := event["error"].(string); ok {
			log.Fatal("stream failed: ", errMessage)
		}
		if delta, ok := event["delta"].(string); ok {
			fmt.Print(delta)
		}
		if eventType, ok := event["type"].(string); ok {
			if eventType == "done" {
				fmt.Println("\n--- Done ---")
			}
		}
	}
}
