# LiteParse Go

Go wrapper for [LiteParse](https://github.com/run-llama/liteparse), exposing the local `liteparse` CLI as a Go package.

## Installation

```bash
go get github.com/run-llama/liteparse/packages/go
```

Prerequisite: install the LiteParse CLI locally.

```bash
npm install -g @llamaindex/liteparse
# or
npx liteparse --version
```

## Quick Start

```go
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

	fmt.Println(result.Text)
	fmt.Println("pages:", result.NumPages())
}
```

Runnable example:

```bash
cd example/minimal
go run .
```

## Configuration

```go
opts := liteparse.DefaultParseOptions()
opts.OutputFormat = liteparse.OutputFormatText
opts.OCREnabled = liteparse.Bool(false)
opts.MaxPages = 10
opts.DPI = 150
opts.PreserveVerySmallText = liteparse.Bool(true)

result, err := parser.Parse("document.pdf", &opts)
if err != nil {
	log.Fatal(err)
}

fmt.Println(result.OutputFormat)
```

## Batch Processing

```go
opts := liteparse.DefaultBatchParseOptions()
opts.Recursive = liteparse.Bool(true)
opts.ExtensionFilter = ".pdf"

result, err := parser.BatchParse("./documents", "./output", &opts)
if err != nil {
	log.Fatal(err)
}

fmt.Println(result.OutputDir)
```

## Screenshots

```go
opts := liteparse.DefaultScreenshotOptions()
opts.TargetPages = "1-3"
opts.LoadBytes = liteparse.Bool(true)

shots, err := parser.Screenshot("document.pdf", "./screenshots", &opts)
if err != nil {
	log.Fatal(err)
}

fmt.Println(shots.Len())
```

## API Surface

- `New(cliPath string) *LiteParse`
- `(*LiteParse).Parse(filePath string, opts *ParseOptions) (*ParseResult, error)`
- `(*LiteParse).BatchParse(inputDir, outputDir string, opts *BatchParseOptions) (*BatchResult, error)`
- `(*LiteParse).Screenshot(filePath, outputDir string, opts *ScreenshotOptions) (*ScreenshotBatchResult, error)`

The wrapper auto-detects `liteparse`, then `lit`, then falls back to `npx liteparse`.
