//go:build ignore

package main

import (
	"fmt"

	"github.com/librefang/librefang/sdk/go"
)

func main() {
	client := librefang.New("http://localhost:4545")

	skills, _ := client.Skills.ListSkills()
	fmt.Printf("Skills: %d\n", len(librefang.ToSlice(skills)))

	models, _ := client.Models.ListAllModels()
	fmt.Printf("Models: %d\n", len(librefang.ToSlice(models)))

	providers, _ := client.Providers.ListProviders()
	fmt.Printf("Providers: %d\n", len(librefang.ToSlice(providers)))
}
