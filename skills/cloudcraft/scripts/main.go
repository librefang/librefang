package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"os"

	"github.com/DataDog/cloudcraft-go"
)

func main() {
	key := os.Getenv("CLOUDCRAFT_API_KEY")
	if key == "" {
		log.Fatal("CLOUDCRAFT_API_KEY environment variable is not set")
	}

	cfg := cloudcraft.NewConfig(key)
	client, err := cloudcraft.NewClient(cfg)
	if err != nil {
		log.Fatalf("failed to create Cloudcraft client: %v", err)
	}

	if len(os.Args) < 2 {
		fmt.Println("Usage: cloudcraft <command> [args]")
		fmt.Println("Commands: me, blueprints, aws-list, aws-snapshot")
		os.Exit(1)
	}

	ctx := context.Background()

	switch os.Args[1] {
	case "me":
		user, _, err := client.User.Me(ctx)
		if err != nil {
			log.Fatalf("failed to get user profile: %v", err)
		}
		printJSON(user)

	case "blueprints":
		blueprints, _, err := client.Blueprint.List(ctx)
		if err != nil {
			log.Fatalf("failed to list blueprints: %v", err)
		}
		printJSON(blueprints)

	case "aws-list":
		accounts, _, err := client.AWS.List(ctx)
		if err != nil {
			log.Fatalf("failed to list AWS accounts: %v", err)
		}
		printJSON(accounts)

	case "aws-snapshot":
		if len(os.Args) < 4 {
			log.Fatal("Usage: cloudcraft aws-snapshot <accountID> <region> [format]")
		}
		accountID := os.Args[2]
		region := os.Args[3]
		format := "png"
		if len(os.Args) > 4 {
			format = os.Args[4]
		}
		// Create snapshot
		data, _, err := client.AWS.Snapshot(ctx, accountID, region, format, nil)
		if err != nil {
			log.Fatalf("failed to create AWS snapshot: %v", err)
		}
		if format == "png" || format == "svg" {
			os.Stdout.Write(data)
		} else {
			fmt.Println(string(data))
		}

	default:
		fmt.Printf("Unknown command: %s\n", os.Args[1])
		os.Exit(1)
	}
}

func printJSON(v interface{}) {
	b, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		log.Fatalf("failed to marshal JSON: %v", err)
	}
	fmt.Println(string(b))
}
