# LiteParse - Agent Documentation

> This file provides comprehensive context for AI coding agents working on this codebase. Each subdirectory contains its own README with file-specific documentation.

## Project Overview

**LiteParse** is an open-source PDF parsing library focused on fast, lightweight document processing with spatial text extraction. It runs entirely locally with zero cloud dependencies by default.

### Key Capabilities
- **Spatial text extraction** with precise bounding boxes
- **Flexible OCR** (built-in Tesseract.js or pluggable HTTP servers)
- **Multi-format support** (PDFs, DOCX, XLSX, PPTX, images via conversion)
- **TypeScript/Node.js** with both library and CLI interfaces

## Directory Structure

```
liteparse/
├── src/
│   ├── core/           # Configuration, types, main orchestrator
│   ├── engines/        # Pluggable PDF and OCR engines
│   │   ├── pdf/        # PDF parsing engines (PDF.js, PDFium)
│   │   └── ocr/        # OCR engines (Tesseract, HTTP)
│   ├── processing/     # Text extraction and spatial analysis
│   ├── output/         # Output formatters (JSON, text)
│   ├── conversion/     # Multi-format conversion to PDF
│   ├── vendor/         # Bundled dependencies (PDF.js)
│   ├── index.ts        # CLI entry point
│   └── lib.ts          # Library public API
├── packages/
│   ├── python/          # Python wrapper package (liteparse)
│   └── pypdf-compatible/ # pypdf drop-in replacement (liteparse-pypdf-compatible)
├── cli/                # CLI implementation
├── ocr/                # Example OCR server implementations
│   ├── easyocr/        # EasyOCR wrapper server
│   └── paddleocr/      # PaddleOCR wrapper server
└── dist/               # Compiled JavaScript output
```

## Data Flow

1. **Input**: File path received (any supported format)
2. **Conversion** (if dependencies installed): Non-PDF formats converted to PDF via LibreOffice/ImageMagick
3. **PDF Loading**: PDF.js extracts text items, images, metadata
4. **OCR** (if enabled): Images rendered and OCR'd for text-sparse areas
5. **Grid Projection**: Spatial reconstruction of text layout using anchor system
6. **Post-processing**: Bounding boxes, text cleanup
7. **Output**: Formatted as JSON or plain text

## Key Design Decisions

### 1. Engine Abstraction Pattern
Both PDF and OCR functionality use interface-based abstraction (`PdfEngine`, `OcrEngine`). This allows:
- Swapping implementations without changing core logic
- Auto-detection: HTTP OCR if URL provided, otherwise Tesseract.js
- Future extensibility for new engines
- Future possibility of custom conversion engines for non-PDF formats

### 2. Spatial Grid Projection
The most complex (and important!) part of the codebase (`src/processing/gridProjection.ts`, ~1650 lines). Uses:
- **Anchor-based layout**: Tracks text alignment (left, right, center, floating)
- **Forward anchors**: Carry alignment information between lines
- **Column detection**: Identifies multi-column layouts
- **Rotation handling**: Transforms 90°, 180°, 270° rotated text to correct reading order
- **OCR merging**: Combines native PDF text with OCR results, preserving confidence scores and source flags in output

### 3. Selective OCR
OCR only runs on embedded images where text extraction failed, not the entire document. This balances accuracy with performance.

### 4. Configuration Merging
Uses a default-first approach where users only override what they need. Configuration flows: defaults → file config → CLI options.

### 5. Format Conversion via External Tools
Rather than implementing format parsers, LiteParse converts non-PDF formats using system tools (LibreOffice, ImageMagick) into a single format (PDF). This provides broad format support with minimal code.

## Common Tasks

### Adding a New Output Format
1. Create new file in `src/output/` implementing the formatter
2. Add format option to `cli/parse.ts`
3. Update `src/core/parser.ts` to use new formatter

### Adding a New OCR Engine
1. Implement `OcrEngine` interface in `src/engines/ocr/`
2. Add initialization logic in `src/core/parser.ts`
3. Add configuration options in `src/core/types.ts`

### Modifying Text Extraction Logic
The processing pipeline is in `src/processing/`. Key files:
- `gridProjection.ts` - Layout reconstruction (most complex)
- `bbox.ts` - Bounding box calculation
- `cleanText.ts` - Text cleanup

### Adding CLI Options
1. Update `cli/parse.ts` with new Commander.js option
2. Add corresponding config field in `src/core/types.ts`
3. Update `src/core/config.ts` with default value
4. Use the option in `src/core/parser.ts`

## Testing Approach

Currently tested via manual verification with sample documents. The project would benefit from:
- Unit tests for processing utilities
- Integration tests with known PDFs
- Snapshot tests for output formats

## Key Dependencies

| Dependency | Purpose |
|------------|---------|
| `pdfjs-dist` | PDF parsing and text extraction |
| `@hyzyla/pdfium` | High-quality PDF rendering for screenshots |
| `tesseract.js` | In-process OCR (zero setup) |
| `sharp` | Image processing |
| `commander` | CLI framework |
| `zod` | Schema validation |

## Entry Points

- **CLI**: `src/index.ts` → `cli/parse.ts`
- **Library**: `src/lib.ts` exports `LiteParse` class and types
- **Main Class**: `src/core/parser.ts` contains `LiteParse` orchestrator
- **Python Package**: `packages/python/` — CLI wrapper (`pip install liteparse`)
- **pypdf Drop-in**: `packages/pypdf-compatible/` — pypdf-compatible wrapper (`pip install liteparse-pypdf-compatible`)

## Related Documentation

These files are key to understanding the codebase and should be referenced for specific implementation details.

If changes to the codebase are being made, please update the relevant documentation files to reflect those changes and keep them up to date.

- [User-facing documentation](README.md)
- [src/conversion/README.md](src/conversion/README.md) - Format conversion details
- [src/core/README.md](src/core/README.md) - Core architecture and configuration
- [src/engines/README.md](src/engines/README.md) - Engine abstraction and implementations
  - [src/engines/pdf/README.md](src/engines/pdf/README.md) - PDF engines (PDF.js, PDFium)
  - [src/engines/ocr/README.md](src/engines/ocr/README.md) - OCR engines (Tesseract, HTTP)
- [src/output/README.md](src/output/README.md) - Output formatters
- [src/processing/README.md](src/processing/README.md) - Text extraction and spatial processing
- [ocr/README.md](ocr/README.md) - OCR server implementations (EasyOCR, PaddleOCR)
- [cli/README.md](cli/README.md) - CLI usage and options
