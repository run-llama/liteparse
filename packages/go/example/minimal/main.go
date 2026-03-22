package main

import (
	"fmt"
	"log"

	liteparse "github.com/run-llama/liteparse/packages/go"
)

func main() {
	parser := liteparse.New("")

	result, err := parser.Parse("document.pdf", nil)
	if err != nil {
		log.Fatal(err)
	}

	fmt.Printf("Parsed %d page(s)\n", result.NumPages())
	fmt.Println(result.Text)
}
