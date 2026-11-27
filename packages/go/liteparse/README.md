# LiteParse Go

Go support for LiteParse. This package wraps the `lit` CLI and parses its JSON output.

## Install

```sh
go get github.com/run-llama/liteparse/packages/go/liteparse
```

Requires the `lit` binary available on `PATH` (or set `LITEPARSE_CLI_PATH`). In a repo checkout, the package builds it automatically via `cargo`.

## Usage

```go
package main

import (
	"fmt"
	"log"

	"github.com/run-llama/liteparse/packages/go/liteparse"
)

func main() {
	parser := liteparse.New()
	res, err := parser.Parse("document.pdf")
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println(res.Text)
}
```