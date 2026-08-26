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

	// Check server health
	health, err := client.System.Health()
	if err != nil {
		return fmt.Errorf("check server health: %w", err)
	}
	fmt.Println("Server:", health)

	// List existing agents
	raw, err := client.Agents.ListAgents(nil)
	if err != nil {
		return fmt.Errorf("list agents: %w", err)
	}
	items, ok := librefang.ToMap(raw)["items"]
	if !ok {
		return fmt.Errorf("list agents response is missing items")
	}
	agents := librefang.ToSlice(items)
	fmt.Println("Agents:", len(agents))

	// Use existing agent or create a new one
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

	// Send a message
	fmt.Println("\n--- Sending message ---")
	reply, err := client.Agents.SendMessage(agentID, map[string]interface{}{
		"message": "Say hello in 5 words.",
	})
	if err != nil {
		return fmt.Errorf("send message: %w", err)
	}
	fmt.Println("Reply:", reply)

	return nil
}
