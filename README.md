# LiteParse

[![CI](https://github.com/run-llama/liteparse/actions/workflows/ci.yml/badge.svg)](https://github.com/run-llama/liteparse/actions/workflows/ci.yml)
|
[![npm version](https://img.shields.io/npm/v/@llamaindex/liteparse.svg)](https://www.npmjs.com/package/@llamaindex/liteparse)
|
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
|
[Docs](https://developers.llamaindex.ai/liteparse/)

<img src="https://github.com/user-attachments/assets/07ba6a82-6bb1-4dea-b0ef-cad7df7d1622" alt="out" width="600">

LiteParse is a standalone OSS PDF parsing tool focused exclusively on **fast and light** parsing. It provides high-quality spatial text parsing with bounding boxes, without proprietary LLM features or cloud dependencies. Everything runs locally on your machine. 

**Hitting the limits of local parsing?**
For complex documents (dense tables, multi-column layouts, charts, handwritten text, or 
scanned PDFs), you'll get significantly better results with [LlamaParse](https://developers.llamaindex.ai/python/cloud/llamaparse/?utm_source=github&utm_medium=liteparse), 
our cloud-based document parser built for production document pipelines. LlamaParse handles the 
hard stuff so your models see clean, structured data and markdown.

>  👉 [Sign up for LlamaParse free](https://cloud.llamaindex.ai?utm_source=github&utm_medium=liteparse)

## Overview

- **Fast Text Parsing**: Spatial text parsing using PDF.js
- **Flexible OCR System**:
  - **Built-in**: Tesseract.js (zero setup, works out of the box!)
  - **HTTP Servers**: Plug in any OCR server (EasyOCR, PaddleOCR, custom)
  - **Standard API**: Simple, well-defined OCR API specification
- **Screenshot Generation**: Generate high-quality page screenshots for LLM agents
- **Multiple Output Formats**: JSON and Text
- **Bounding Boxes**: Precise text positioning information
- **Standalone Binary**: No cloud dependencies, runs entirely locally
- **Multi-platform**: Linux, macOS (Intel/ARM), Windows

## Installation

### CLI Tool

#### Option 1: Global Install (Recommended)

Install globally via npm to use the `lit` command anywhere:

```bash
npm i -g @llamaindex/liteparse
```

Then use it:

```bash
lit parse document.pdf
lit screenshot document.pdf
```

For macOS and Linux users, `liteparse` can be also installed via `brew`:

```bash
brew tap run-llama/liteparse
brew install llamaindex-liteparse
```

#### Option 2: Install from Source

You can clone the repo and install the CLI globally from source:

```
git clone https://github.com/run-llama/liteparse.git
cd liteparse
npm run build
npm pack
npm install -g ./liteparse-*.tgz
```

### Agent Skill

You can use `liteparse` as an agent skill, downloading it with the `skills` CLI tool:

```bash
npx skills add run-llama/llamaparse-agent-skills --skill liteparse
```

Or copy-pasting the [`SKILL.md`](https://github.com/run-llama/llamaparse-agent-skills/blob/main/skills/liteparse/SKILL.md) file to your own skills setup.

## Usage

### Parse Files

```bash
# Basic parsing
lit parse document.pdf

# Parse with specific format
lit parse document.pdf --format json -o output.md

# Parse specific pages
lit parse document.pdf --target-pages "1-5,10,15-20"

# Parse without OCR
lit parse document.pdf --no-ocr

# Parse a remote PDF
curl -sL https://example.com/report.pdf | lit parse -
```

### Batch Parsing

You can also parse an entire directory of documents:

```bash
lit batch-parse ./input-directory ./output-directory
```

### Generate Screenshots

Screenshots are essential for LLM agents to extract visual information that text alone cannot capture.

```bash
# Screenshot all pages
lit screenshot document.pdf -o ./screenshots

# Screenshot specific pages
lit screenshot document.pdf --target-pages "1,3,5" -o ./screenshots

# Custom DPI
lit screenshot document.pdf --dpi 300 -o ./screenshots

# Screenshot page range
lit screenshot document.pdf --target-pages "1-10" -o ./screenshots
```

### Library Usage

Install as a dependency in your project:

```bash
npm install @llamaindex/liteparse
# or
pnpm add @llamaindex/liteparse
```

```typescript
import { LiteParse } from '@llamaindex/liteparse';

const parser = new LiteParse({ ocrEnabled: true });
const result = await parser.parse('document.pdf');
console.log(result.text);
```

#### Buffer / Uint8Array Input

You can pass raw bytes directly instead of a file path, which is useful for remote files:

```typescript
import { LiteParse } from '@llamaindex/liteparse';
import { readFile } from 'fs/promises';

const parser = new LiteParse();

// From a file read
const pdfBytes = await readFile('document.pdf');
const result = await parser.parse(pdfBytes);

// From an HTTP response
const response = await fetch('https://example.com/document.pdf');
const buffer = Buffer.from(await response.arrayBuffer());
const result2 = await parser.parse(buffer);
```

Non-PDF buffers (images, Office documents) are written to a temp directory for format conversion. Screenshots also work with buffer input:

```typescript
const screenshots = await parser.screenshot(pdfBytes, [1, 2, 3]);
```

### Browser Usage

LiteParse's core parsing engine (PDF.js text extraction, grid projection, OCR via Tesseract.js) can run in the browser. Since the library has Node-only dependencies (sharp, fs, child_process), you'll need a bundler like Vite to swap those out with browser stubs.

#### Vite Configuration

The key is a Vite plugin that redirects Node-only source files to browser-safe replacements, plus `resolve.alias` entries that stub out Node built-in modules:

```typescript
// vite.config.ts
import { defineConfig, type Plugin } from "vite";
import { resolve, dirname } from "node:path";

// Node-only files → browser stubs (you write these)
const FILE_REDIRECTS = [
  { match: /\/engines\/pdf\/pdfium-renderer(\.js|\.ts)?$/, target: "stubs/pdfium-renderer.ts" },
  { match: /\/engines\/pdf\/pdfjsImporter(\.js|\.ts)?$/,   target: "stubs/pdfjsImporter.ts" },
  { match: /\/engines\/ocr\/http-simple(\.js|\.ts)?$/,     target: "stubs/http-simple.ts" },
  { match: /\/conversion\/convertToPdf(\.js|\.ts)?$/,      target: "stubs/convertToPdf.ts" },
  { match: /\/processing\/gridDebugLogger(\.js|\.ts)?$/,   target: "stubs/gridDebugLogger.ts" },
  { match: /\/processing\/gridVisualizer(\.js|\.ts)?$/,    target: "stubs/gridVisualizer.ts" },
];

function liteparseNodeRedirects(): Plugin {
  return {
    name: "liteparse-node-redirects",
    enforce: "pre",
    async resolveId(source, importer) {
      if (!importer) return null;
      const abs = source.startsWith(".") ? resolve(dirname(importer), source) : source;
      for (const { match, target } of FILE_REDIRECTS) {
        if (match.test(abs) || match.test(source)) return resolve(target);
      }
      return null;
    },
  };
}

export default defineConfig({
  plugins: [liteparseNodeRedirects()],
  optimizeDeps: { include: ["tesseract.js"] },
  resolve: {
    alias: [
      { find: "node:fs/promises", replacement: "stubs/empty.ts" },
      { find: "node:fs",          replacement: "stubs/empty.ts" },
      { find: "node:url",         replacement: "stubs/empty.ts" },
      { find: "node:path",        replacement: "stubs/empty.ts" },
      { find: "node:os",          replacement: "stubs/empty.ts" },
      { find: "node:child_process", replacement: "stubs/empty.ts" },
      { find: /^fs$/,             replacement: "stubs/empty.ts" },
      { find: /^path$/,           replacement: "stubs/empty.ts" },
      { find: /^os$/,             replacement: "stubs/empty.ts" },
      { find: /^child_process$/,  replacement: "stubs/empty.ts" },
      { find: "form-data",        replacement: "stubs/empty.ts" },
      { find: "axios",            replacement: "stubs/empty.ts" },
      { find: "file-type",        replacement: "stubs/file-type.ts" },
    ],
  },
});
```

See [`scripts/browser-compat/`](scripts/browser-compat/) for a complete working example with all the stub files.

#### What works in the browser

- PDF parsing from `Uint8Array` input (use `file.arrayBuffer()` to get bytes from a `<input type="file">`)
- OCR via Tesseract.js (runs in Web Workers, fetches language data from CDN on first use)
- Text and JSON output formats

#### What doesn't work

- File path input (pass `Uint8Array` instead)
- DOCX/XLSX/PPTX/image conversion (requires LibreOffice/ImageMagick)
- HTTP OCR server backend
- Screenshots (these use PDFium + sharp, which are native Node addons)

### CLI Options

#### Parse Command

```
$ lit parse --help
Usage: lit parse [options] <file>

Parse a document file (PDF, DOCX, XLSX, PPTX, images, etc.)

Options:
  -o, --output <file>     Output file path
  --format <format>       Output format: json|text (default: "text")
  --ocr-server-url <url>  HTTP OCR server URL (uses Tesseract if not provided)
  --no-ocr                Disable OCR
  --ocr-language <lang>   OCR language(s) (default: "en")
  --num-workers <n>       Number of pages to OCR in parallel (default: CPU cores - 1)
  --max-pages <n>         Max pages to parse (default: "10000")
  --target-pages <pages>  Target pages (e.g., "1-5,10,15-20")
  --dpi <dpi>             DPI for rendering (default: "150")
  --no-precise-bbox       Disable precise bounding boxes
  --preserve-small-text   Preserve very small text
  --password <password>   Password for encrypted/protected documents
  --config <file>         Config file (JSON)
  -q, --quiet             Suppress progress output
  -h, --help              display help for command
```

#### Batch Parse Command

```
$ lit batch-parse --help
Usage: lit batch-parse [options] <input-dir> <output-dir>

Parse multiple documents in batch mode (reuses PDF engine for efficiency)

Options:
  --format <format>       Output format: json|text (default: "text")
  --ocr-server-url <url>  HTTP OCR server URL (uses Tesseract if not provided)
  --no-ocr                Disable OCR
  --ocr-language <lang>   OCR language(s) (default: "en")
  --num-workers <n>       Number of pages to OCR in parallel (default: CPU cores - 1)
  --max-pages <n>         Max pages to parse per file (default: "10000")
  --dpi <dpi>             DPI for rendering (default: "150")
  --no-precise-bbox       Disable precise bounding boxes
  --recursive             Recursively search input directory
  --extension <ext>       Only process files with this extension (e.g., ".pdf")
  --password <password>   Password for encrypted/protected documents (applied to all files)
  --config <file>         Config file (JSON)
  -q, --quiet             Suppress progress output
  -h, --help              display help for command
```

#### Screenshot Command

```
$ lit screenshot --help
Usage: lit screenshot [options] <file>

Generate screenshots of PDF pages

Options:
  -o, --output-dir <dir>  Output directory for screenshots (default: "./screenshots")
  --target-pages <pages>  Page numbers to screenshot (e.g., "1,3,5" or "1-5")
  --dpi <dpi>             DPI for rendering (default: "150")
  --format <format>       Image format: png|jpg (default: "png")
  --password <password>   Password for encrypted/protected documents
  --config <file>         Config file (JSON)
  -q, --quiet             Suppress progress output
  -h, --help              display help for command
```

## OCR Setup

### Default: Tesseract.js

```bash
# Tesseract is enabled by default
lit parse document.pdf

# Specify language
lit parse document.pdf --ocr-language fra

# Disable OCR
lit parse document.pdf --no-ocr
```

By default, Tesseract.js downloads language data from the internet on first use. For offline or air-gapped environments, set the `TESSDATA_PREFIX` environment variable to a directory containing pre-downloaded `.traineddata` files:

```bash
export TESSDATA_PREFIX=/path/to/tessdata
lit parse document.pdf --ocr-language eng
```

You can also pass `tessdataPath` in the library config:

```typescript
const parser = new LiteParse({ tessdataPath: '/path/to/tessdata' });
```

### Optional: HTTP OCR Servers

For higher accuracy or better performance, you can use an HTTP OCR server. We provide ready-to-use example wrappers for popular OCR engines:

- [EasyOCR](ocr/easyocr/README.md)
- [PaddleOCR](ocr/paddleocr/README.md)

You can integrate any OCR service by implementing the simple LiteParse OCR API specification (see [`OCR_API_SPEC.md`](OCR_API_SPEC.md)).

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

Just install the dependency and LiteParse will automatically convert these formats to PDF for parsing:

```bash
# macOS
brew install --cask libreoffice

# Ubuntu/Debian
apt-get install libreoffice

# Windows
choco install libreoffice-fresh # might require admin permissions
```

> _For Windows, you might need to add the path to the directory containing LibreOffice CLI executable (generally `C:\Program Files\LibreOffice\program`) to the environment variables and re-start the machine._

#### Images (via ImageMagick)
- **Formats**: `.jpg`, `.jpeg`, `.png`, `.gif`, `.bmp`, `.tiff`, `.webp`, `.svg`

Just install ImageMagick and LiteParse will convert images to PDF for parsing (with OCR):

```bash
# macOS
brew install imagemagick

# Ubuntu/Debian
apt-get install imagemagick

# Windows
choco install imagemagick.app # might require admin permissions
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `TESSDATA_PREFIX` | Path to a directory containing Tesseract `.traineddata` files. Used for offline/air-gapped environments where Tesseract.js cannot download language data from the internet. |
| `LITEPARSE_TMPDIR` | Override the temp directory used for format conversion and intermediate files. Defaults to the OS temp directory (`os.tmpdir()`). Useful in containerized or read-only filesystem environments. |

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
  "preciseBoundingBox": true,
  "preserveVerySmallText": false,
  "password": "optional_password"
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
lit parse document.pdf --config liteparse.config.json
```

## Development

We provide a fairly rich `AGENTS.md`/`CLAUDE.md` that we recommend using to help with development + coding agents.

```bash
# Install dependencies
npm install

# Build TypeScript (Linux/macOs)
npm run build

# Build Typescript (Windows)
npm run build:windows

# Watch mode
npm run dev

# Test parsing
npm test
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
