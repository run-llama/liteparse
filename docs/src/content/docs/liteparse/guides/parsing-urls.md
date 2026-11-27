---
title: Parsing URLs
description: Parse remote documents by reading URLs.
sidebar:
  order: 9
---

To parse remote files, LiteParse supports both CLI and library usage for reading bytes and streams. The CLI can download them with any tool you like and pipe the bytes to `lit parse` using `-` as the file argument, while the libraries can fetch the bytes directly and pass them to the parser.

## CLI usage

```bash
# Parse a remote PDF
curl -sL https://example.com/report.pdf | lit parse -

# With options
curl -sL https://example.com/report.pdf | lit parse --no-ocr --format json -

# Save to a file
curl -sL https://example.com/report.pdf | lit parse -o report.txt -
```

The `-` argument tells LiteParse to read from stdin instead of a file path. Any tool that writes to stdout works — `curl`, `wget`, `aws s3 cp - -`, etc.

## Library usage

The TypeScript library accepts `Buffer`/`Uint8Array` directly, so you can handle the download however you like.

For example, using `fetch` in a Node.js environment:

```typescript
import { LiteParse } from "@llamaindex/liteparse";

const response = await fetch("https://example.com/report.pdf");
const buffer = Buffer.from(await response.arrayBuffer());

const parser = new LiteParse({ ocrEnabled: false });
const result = await parser.parse(buffer);
console.log(result.text);
```
