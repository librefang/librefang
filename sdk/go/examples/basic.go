package main

import (
	"fmt"
	"log"

	"github.com/librefang/librefang/sdk/go"
)

func main() {
	client := librefang.New("http://localhost:4545")

	// Check server health
	health, err := client.Health()
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println("Server:", health)

	// List existing agents
	agents, err := client.Agents.List()
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println("Agents:", len(agents))

	// Use existing agent or create a new one
	var agentID string
	var shouldDelete bool

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
		shouldDelete = true
		fmt.Println("Created agent:", agentID)
	}

	// Send a message
	fmt.Println("\n--- Sending message ---")
	reply, err := client.Agents.Message(agentID, "Say hello in 5 words.")
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println("Reply:", reply)

	// Clean up only if we created it
	if shouldDelete {
		client.Agents.Delete(agentID)
		fmt.Println("Agent deleted.")
	}
}
