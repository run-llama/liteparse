# LiteParse

Open-source PDF parsing with spatial text extraction - no LLMs, no cloud dependencies.

## Overview

LiteParse is a standalone OSS PDF parsing tool focused exclusively on **fast and light** parsing. It provides high-quality spatial text extraction with bounding boxes, without proprietary LLM features or cloud dependencies. Everything runs locally on your machine. 

### Features

- **Fast Text Extraction**: Spatial text extraction using PDF.js
- **Flexible OCR System**:
  - **Built-in**: Tesseract.js (zero setup, works out of the box!)
  - **HTTP Servers**: Plug in any OCR server (EasyOCR, PaddleOCR, custom)
  - **Standard API**: Simple, well-defined OCR API specification
- **Screenshot Generation**: Generate high-quality page screenshots for LLM agents
- **Multiple Output Formats**: JSON and Text
- **Bounding Boxes**: Precise text positioning information
- **Table Detection**: Heuristic table detection (outlined tables)
- **Standalone Binary**: No cloud dependencies, runs entirely locally
- **Multi-platform**: Linux, macOS (Intel/ARM), Windows

## Installation

### Option 1: Use as a Library

Install via npm/pnpm:

```bash
pnpm add liteparse
```

```typescript
import { LiteParse } from 'liteparse';

const parser = new LiteParse({ ocrEnabled: true });
const result = await parser.parse('document.pdf');
console.log(result.text);
```

### Option 2: Build from Source

#### Prerequisites

- Node.js 22.4.1 or higher
- pnpm package manager
- [Bun](https://bun.sh) (for building binaries)

#### Install Dependencies

```bash
cd liteparse
pnpm install
```

#### Build TypeScript

```bash
pnpm build
```

#### Build Binary (Optional)

Build a standalone executable that you can run anywhere:

```bash
# Install Bun if you haven't already
curl -fsSL https://bun.sh/install | bash

# Build binary for your current platform
pnpm build:binary

# Or build for specific platforms
pnpm build:bun:linux    # Linux x64
pnpm build:bun:macos    # macOS ARM64
pnpm build:bun:windows  # Windows x64
pnpm build:bun:all      # All platforms

# The binary will be in ./bin/lp (or lp-{platform})
```

Once built, you can use the binary directly:

```bash
# Add to your PATH
export PATH="$PATH:/path/to/liteparse/bin"

# Now use it anywhere
lp parse document.pdf
lp screenshot document.pdf
```

## Development Usage

### Parse PDF Files

You can run tests without building the binary using the CLI:

```bash
# Basic parsing
pnpm parse document.pdf

# Parse with specific format
pnpm parse document.pdf --format json -o output.md

# Parse specific pages
pnpm parse document.pdf --target-pages "1-5,10,15-20"

# Parse without OCR
pnpm parse document.pdf --no-ocr

# Use custom config file
pnpm parse document.pdf --config config.json
```

### Generate Screenshots

Screenshots are essential for LLM agents to extract visual information that text alone cannot capture.

```bash
# Screenshot all pages
pnpm screenshot document.pdf

# Screenshot specific pages
pnpm screenshot document.pdf --pages "1,3,5"

# Custom output directory and DPI
pnpm screenshot document.pdf -o ./images --dpi 300

# Screenshot page range
pnpm screenshot document.pdf --pages "1-10"
```

### CLI Options

#### Parse Command

```
lp parse <file> [options]

Options:
  -o, --output <file>              Output file path
  --format <format>                Output format: json|text (default: "text")
  --ocr-server-url <url>           HTTP OCR server URL (uses Tesseract if not provided)
  --no-ocr                         Disable OCR
  --ocr-language <lang>            OCR language(s) (default: "en")
  --max-pages <n>                  Max pages to parse (default: "1000")
  --target-pages <pages>           Target pages (e.g., "1-5,10,15-20")
  --pdf-engine <engine>            PDF engine: pdfjs|pdfium (default: "pdfjs")
  --dpi <dpi>                      DPI for rendering (default: "150")
  --no-tables                      Disable table detection
  --no-precise-bbox                Disable precise bounding boxes
  --skip-diagonal-text             Skip diagonal text
  --preserve-small-text            Preserve very small text
  --config <file>                  Config file (JSON)
```

#### Screenshot Command

```
lp screenshot <file> [options]

Options:
  -o, --output-dir <dir>           Output directory for screenshots (default: "./screenshots")
  --pages <pages>                  Page numbers to screenshot (e.g., "1,3,5" or "1-5")
  --dpi <dpi>                      DPI for rendering (default: "150")
  --format <format>                Image format: png|jpg (default: "png")
  --config <file>                  Config file (JSON)
```

## OCR Setup

**OCR works out of the box!** The default Tesseract.js engine runs in-process with zero setup.

### Default: Tesseract.js

```bash
# Tesseract is enabled by default
pnpm parse document.pdf

# Specify language
pnpm parse document.pdf --ocr-language fra

# Disable OCR
pnpm parse document.pdf --no-ocr
```

### Optional: HTTP OCR Servers

For higher accuracy or better performance, you can use an HTTP OCR server. We provide ready-to-use example wrappers for popular OCR engines:

#### EasyOCR

**Setup:**
```bash
cd ocr/easyocr
docker build -t liteparse-easyocr .
docker run -p 8828:8828 liteparse-easyocr
```

**Usage:**
```bash
pnpm parse document.pdf --ocr-server-url http://localhost:8828/ocr
```

#### PaddleOCR

**Setup:**
```bash
cd ocr/paddleocr
docker build -t liteparse-paddleocr .
docker run -p 8829:8829 liteparse-paddleocr
```

**Usage:**
```bash
pnpm parse document.pdf --ocr-server-url http://localhost:8829/ocr --ocr-language zh
```

### Custom OCR Server

You can integrate **any OCR service** by implementing the simple LiteParse OCR API specification (see `OCR_API_SPEC.md`).

The API requires:
- POST `/ocr` endpoint
- Accepts `file` and `language` parameters
- Returns JSON: `{ results: [{ text, bbox: [x1,y1,x2,y2], confidence }] }`

See the example servers in `ocr/easyocr/` and `ocr/paddleocr/` as templates.

For the complete OCR API specification, see [`OCR_API_SPEC.md`](OCR_API_SPEC.md).

## Multi-Format Input Support

LiteParse supports **automatic conversion** of various document formats to PDF before parsing. This makes it unique compared to other PDF-only parsing tools!

### Supported Input Formats

#### Office Documents (via LibreOffice)
- **Word**: `.doc`, `.docx`, `.docm`, `.odt`, `.rtf`
- **PowerPoint**: `.ppt`, `.pptx`, `.pptm`, `.odp`
- **Spreadsheets**: `.xls`, `.xlsx`, `.xlsm`, `.ods`, `.csv`, `.tsv`

**Setup:**
```bash
# macOS
brew install --cask libreoffice

# Ubuntu/Debian
apt-get install libreoffice
```

**Usage:**
```bash
# Parse Word document
pnpm parse report.docx -o report.txt

# Parse PowerPoint
pnpm parse slides.pptx --format json -o slides.json

# Parse Excel spreadsheet
pnpm parse data.xlsx -o data.txt
```

#### Images (via ImageMagick)
- **Formats**: `.jpg`, `.jpeg`, `.png`, `.gif`, `.bmp`, `.tiff`, `.webp`, `.svg`

**Setup:**
```bash
# macOS
brew install imagemagick

# Ubuntu/Debian
apt-get install imagemagick
```

**Usage:**
```bash
# Parse image (automatically runs OCR)
pnpm parse receipt.jpg --ocr-engine tesseract -o receipt.txt

# Parse scanned diagram
pnpm parse diagram.png --format json -o diagram.json
```

## Configuration

You can configure parsing options via CLI flags or a JSON config file. The config file allows you to set sensible defaults and override as needed.

### Config File Example

Create a `liteparse.config.json` file:

```json
{
  "ocrLanguage": "en",
  "ocrEnabled": true,
  "maxPages": 1000,
  "dpi": 150,
  "outputFormat": "json",
  "includeImages": true,
  "includeCharts": true,
  "tableDetection": true,
  "preciseBoundingBox": true,
  "skipDiagonalText": false,
  "preserveVerySmallText": false
}
```

For HTTP OCR servers, just add `ocrServerUrl`:

```json
{
  "ocrServerUrl": "http://localhost:8828/ocr",
  "ocrLanguage": "en",
  "outputFormat": "json"
}
```

Use with:

```bash
pnpm parse document.pdf --config liteparse.config.json
```

## Programmatic Usage

### Installation

```bash
npm install liteparse
# or
pnpm add liteparse
```

### Basic Example

```typescript
import { LiteParse, type LiteParseConfig } from 'liteparse';

// Default configuration: uses built-in Tesseract OCR
const parser = new LiteParse({
  outputFormat: 'json',
  ocrEnabled: true,
  tableDetection: true,
});

// Parse a PDF file
const result = await parser.parse('document.pdf');

// Access parsed text
console.log(result.text);

// Access structured data
console.log(result.json);

// Access individual pages
for (const page of result.pages) {
  console.log(`Page ${page.page}: ${page.text}`);
  console.log(`Text items:`, page.textItems);
  console.log(`Tables:`, page.tables);
}
```

### Using HTTP OCR Server

```typescript
import { LiteParse } from 'liteparse';

// Configure with HTTP OCR server (EasyOCR, PaddleOCR, or custom)
const parser = new LiteParse({
  ocrServerUrl: 'http://localhost:8828/ocr',
  ocrLanguage: 'zh',
  outputFormat: 'json',
});

const result = await parser.parse('chinese-document.pdf');
console.log(result.text);
```

### Generating Screenshots

```typescript
import { LiteParse } from 'liteparse';
import fs from 'fs/promises';

const parser = new LiteParse({ dpi: 300 });

// Generate screenshots for specific pages
const screenshots = await parser.screenshot('document.pdf', [1, 2, 3]);

// Save screenshots
for (const screenshot of screenshots) {
  const filename = `page_${screenshot.pageNum}.png`;
  await fs.writeFile(filename, screenshot.imageBuffer);
  console.log(`Saved: ${filename} (${screenshot.width}x${screenshot.height})`);
}

// Generate screenshots for all pages (omit page numbers)
const allScreenshots = await parser.screenshot('document.pdf');
```

### Configuration Options

```typescript
import { LiteParse, type LiteParseConfig } from 'liteparse';

const config: LiteParseConfig = {
  // Output format
  outputFormat: 'json', // 'json' | 'text'

  // OCR configuration
  ocrEnabled: true,
  ocrServerUrl: 'http://localhost:8828/ocr', // Optional HTTP server
  ocrLanguage: 'en', // Language code

  // Parsing options
  maxPages: 1000,
  targetPages: '1-10,15', // Specific pages to parse
  dpi: 150, // Resolution for rendering

  // Feature toggles
  tableDetection: true,
  preciseBoundingBox: true,
  skipDiagonalText: false,
  preserveVerySmallText: false,
};

const parser = new LiteParse(config);
```

### TypeScript Types

```typescript
import {
  LiteParse,
  type LiteParseConfig,
  type ParseResult,
  type PageResult,
  type TextItem,
  type BoundingBox,
  type TableItem,
  type ImageItem,
} from 'liteparse';

// ParseResult structure
const result: ParseResult = {
  text: string,           // Plain text content
  json: object,           // Structured JSON data
  pages: PageResult[],    // Per-page results
};

// PageResult structure
const page: PageResult = {
  page: number,           // Page number (1-indexed)
  width: number,          // Page width in points
  height: number,         // Page height in points
  text: string,           // Page text content
  textItems: TextItem[],  // Individual text items with positions
  boundingBoxes: BoundingBox[], // Text bounding boxes
  tables: TableItem[],    // Detected tables
  images: ImageItem[],    // Embedded images
};
```

## Output Formats

### JSON Format

```json
{
  "pages": [
    {
      "page": 1,
      "width": 612,
      "height": 792,
      "text": "Page text content...",
      "textItems": [
        {
          "text": "Hello",
          "x": 100,
          "y": 200,
          "width": 50,
          "height": 12,
          "fontName": "Arial",
          "fontSize": 12
        }
      ],
      "boundingBoxes": [...],
      "tables": [...],
      "images": [...]
    }
  ]
}
```

### Text Format

Plain text with page separators:

```
--- Page 1 ---
Page text content...

--- Page 2 ---
More text...
```

## Architecture

```
liteparse/
├── src/
│   ├── core/              # Configuration and main Parser class
│   ├── engines/           # PDF and OCR engine abstractions
│   │   ├── pdf/          # PDF.js engine
│   │   └── ocr/          # OCR engines (Tesseract, HTTP)
│   ├── processing/        # Core processing pipeline
│   │   ├── grid.ts       # Spatial text projection
│   │   ├── bbox.ts       # Bounding box building
│   │   └── tables.ts     # Table detection
│   ├── output/            # Output formatters
│   └── vendor/            # Bundled PDF.js
├── cli/                   # Command-line interface
├── ocr/                   # Example OCR servers
│   ├── easyocr/          # EasyOCR wrapper server
│   └── paddleocr/        # PaddleOCR wrapper server
├── OCR_API_SPEC.md        # Standard OCR API specification
└── examples/              # Usage examples
```

The OCR system uses a simple client-server model:
- **Client**: Single HTTP OCR engine that calls standard API
- **Server**: Your choice (EasyOCR, PaddleOCR, custom) conforming to API spec
- **Default**: Built-in Tesseract.js (no server needed)

## Development

```bash
# Install dependencies
pnpm install

# Build TypeScript
pnpm build

# Watch mode
pnpm dev

# Test parsing
pnpm test
```

## License

Apache 2.0

## Credits

Built on top of:

- [PDF.js](https://github.com/mozilla/pdf.js) - PDF parsing engine
- [Tesseract.js](https://github.com/naptha/tesseract.js) - In-process OCR engine
- [EasyOCR](https://github.com/JaidedAI/EasyOCR) - HTTP OCR server (optional)
- [PaddleOCR](https://github.com/PaddlePaddle/PaddleOCR) - HTTP OCR server (optional)
- [Sharp](https://github.com/lovell/sharp) - Image processing

## Contributing

Contributions welcome! This is Phase 1 of the implementation. See `ROADMAP.md` for planned features.
