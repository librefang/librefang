package main

import (
	"fmt"
	"log"

	"github.com/librefang/librefang/sdk/go"
)

func main() {
	client := librefang.New("http://localhost:4545")

	// List existing agents
	agents, err := client.Agents.List()
	if err != nil {
		log.Fatal(err)
	}

	var agentID string
	if len(agents) > 0 {
		agentID = agents[0]["id"].(string)
		fmt.Println("Using existing agent:", agentID)
	} else {
		agent, err := client.Agents.Create(map[string]interface{}{
			"template": "assistant",
		})
		if err != nil {
			log.Fatal(err)
		}
		agentID = agent["id"].(string)
		fmt.Println("Created agent:", agentID)
	}

	// Stream the response
	fmt.Println("\n--- Streaming response ---")
	for event := range client.Agents.Stream(agentID, "Say hello in 3 words.") {
		if delta, ok := event["delta"].(string); ok {
			fmt.Print(delta)
		} else if eventType, ok := event["type"].(string); ok {
			if eventType == "done" {
				fmt.Println("\n--- Done ---")
			}
		}
	}
}
