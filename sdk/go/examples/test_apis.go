//go:build ignore

package main

import (
	"fmt"
	"log"

	"github.com/librefang/librefang/sdk/go"
)

func main() {
	client := librefang.New("http://localhost:4545")

	if skills, err := client.Skills.ListSkills(); err != nil {
		log.Printf("ListSkills failed: %v", err)
	} else {
		fmt.Printf("Skills: %d\n", len(librefang.ToSlice(skills)))
	}

	if models, err := client.Models.ListAllModels(); err != nil {
		log.Printf("ListAllModels failed: %v", err)
	} else {
		fmt.Printf("Models: %d\n", len(librefang.ToSlice(models)))
	}

	if providers, err := client.Models.ListProviders(); err != nil {
		log.Printf("ListProviders failed: %v", err)
	} else {
		fmt.Printf("Providers: %d\n", len(librefang.ToSlice(providers)))
	}
}
