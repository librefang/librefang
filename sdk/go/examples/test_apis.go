package main

import (
	"fmt"

	"github.com/librefang/sdk-go"
)

func main() {
	client := librefang.New("http://localhost:4545")

	skills, _ := client.Skills.List()
	fmt.Printf("Skills: %d\n", len(skills))

	models, _ := client.Models.List()
	fmt.Printf("Models: %d\n", len(models))

	providers, _ := client.Providers.List()
	fmt.Printf("Providers: %d\n", len(providers))
}
