# LiteParse - Agent Documentation

> This file provides comprehensive context for AI coding agents working on this codebase.

## Project Overview

**LiteParse** is an open-source PDF parsing library written in **Rust**, focused on fast, lightweight document processing with spatial text extraction. It runs entirely locally with zero cloud dependencies by default.

Language bindings are provided for **Node.js/TypeScript** (via napi-rs), **Python** (via PyO3), and **WebAssembly** (via wasm-bindgen).

### Key Capabilities
- **Spatial text extraction** with precise bounding boxes
- **Flexible OCR** (built-in Tesseract or pluggable HTTP servers)
- **Multi-format support** (PDFs, DOCX, XLSX, PPTX, images via conversion)
- **Multi-language bindings**: Rust, Node.js/TypeScript, Python, Browser (WASM)
- **CLI** available from all installation methods (`cargo`, `npm`, `pip`)

## Directory Structure

```
liteparse/
├── crates/
│   ├── liteparse/          # Core Rust library + CLI binary
│   │   └── src/
│   │       ├── main.rs         # CLI entry point (clap)
│   │       ├── lib.rs          # Library root
│   │       ├── parser.rs       # LiteParse orchestrator
│   │       ├── config.rs       # Configuration types and defaults
│   │       ├── types.rs        # Core data types (ParseResult, TextItem, etc.)
│   │       ├── projection.rs   # Spatial grid projection (layout reconstruction)
│   │       ├── extract.rs      # Raw text extraction from PDFium
│   │       ├── render.rs       # Page rendering / screenshots
│   │       ├── conversion.rs   # Non-PDF format conversion (LibreOffice, image/resvg/usvg rust crates)
│   │       ├── ocr_merge.rs    # Merging OCR results with native text
│   │       ├── error.rs        # Error types
│   │       ├── ocr/            # OCR engine implementations
│   │       │   ├── mod.rs          # OcrEngine trait
│   │       │   ├── tesseract.rs    # Built-in Tesseract OCR
│   │       │   └── http_simple.rs  # HTTP OCR server client
│   │       └── output/         # Output formatters
│   │           ├── mod.rs
│   │           ├── json.rs
│   │           └── text.rs
│   ├── liteparse-napi/     # Node.js bindings (napi-rs)
│   ├── liteparse-python/   # Python bindings (PyO3 / maturin)
│   ├── liteparse-wasm/     # WASM bindings (wasm-bindgen)
│   ├── pdfium/             # Rust wrapper around PDFium C API
│   └── pdfium-sys/         # PDFium FFI (C → Rust) bindings
├── packages/
│   ├── node/               # npm package: TS wrapper + CLI around native binary
│   │   └── src/
│   │       ├── lib.ts          # Public LiteParse class for Node.js
│   │       ├── cli.ts          # CLI entry point (commander)
│   │       └── native.ts       # Native binary loader
│   ├── python/             # PyPI package: Python wrapper around native binary
│   │   └── liteparse/
│   │       ├── __init__.py
│   │       ├── parser.py       # Public LiteParse class for Python
│   │       ├── types.py        # Python dataclass types
│   │       └── cli.py          # CLI entry point
│   └── wasm/               # WASM npm package
├── ocr/                    # Example OCR server implementations
│   ├── easyocr/            # EasyOCR wrapper server
│   └── paddleocr/          # PaddleOCR wrapper server
└── Cargo.toml              # Workspace root
```

## Data Flow

1. **Input**: File path or raw bytes received (any supported format)
2. **Conversion** (if needed): Non-PDF formats converted to PDF via LibreOffice and image/resvg/usvg rust crates
3. **PDF Loading**: PDFium extracts text items, images, metadata
4. **OCR** (if enabled): Pages rendered and OCR'd for text-sparse areas
5. **Grid Projection**: Spatial reconstruction of text layout using anchor system
6. **Post-processing**: Bounding boxes, text cleanup
7. **Output**: Formatted as JSON or plain text

## Key Design Decisions

### 1. Rust Core with Language Bindings
The core parsing logic is written in Rust for performance and safety. Language-specific crates expose the same API surface:
- `liteparse-napi` → Node.js via napi-rs
- `liteparse-python` → Python via PyO3/maturin
- `liteparse-wasm` → Browser via wasm-bindgen

Each binding crate is thin — it wraps the core `liteparse` crate's types and async API.

### 2. OCR Engine Trait
OCR functionality uses a trait-based abstraction (`OcrEngine`). This allows:
- Built-in Tesseract (default, compiled in via `tesseract-rs`)
- HTTP OCR server client for remote engines
- Custom JS-side OCR in the WASM build via a callback interface

### 3. Spatial Grid Projection
The most complex (and important!) part of the codebase (`crates/liteparse/src/projection.rs`). Uses:
- **Anchor-based layout**: Tracks text alignment (left, right, center, floating)
- **Forward anchors**: Carry alignment information between lines
- **Column detection**: Identifies multi-column layouts
- **Rotation handling**: Transforms 90°, 180°, 270° rotated text to correct reading order
- **OCR merging**: Combines native PDF text with OCR results, preserving confidence scores and source flags in output

### 4. Selective OCR
OCR only runs on embedded images where text extraction failed, not the entire document. This balances accuracy with performance.

### 5. Configuration
Uses a default-first approach where users only override what they need. See `crates/liteparse/src/config.rs` for defaults.

### 6. Format Conversion via External Tools
Rather than implementing format parsers, LiteParse converts office file formats using system tools (LibreOffice) into PDF. This provides broad format support with minimal code.

## Common Tasks

### Adding a New Output Format
1. Create new file in `crates/liteparse/src/output/`
2. Add variant to `OutputFormat` enum in `config.rs`
3. Wire it up in `main.rs` and binding crates

### Adding a New OCR Engine
1. Implement `OcrEngine` trait in `crates/liteparse/src/ocr/`
2. Add initialization logic in `parser.rs`
3. Add configuration options in `config.rs`

### Modifying Text Extraction Logic
Key files in `crates/liteparse/src/`:
- `projection.rs` — Layout reconstruction (most complex)
- `extract.rs` — Raw text item extraction from PDFium
- `text_clip.rs` — Shared glyph visibility: PDFium clip points already include the object matrix; compose only ancestor form matrices and inherit ancestor clips.
- `text_paint.rs` — Drops fill-only text that never changes a pixel: white on bare page, or the colour of the rectangle under it. Checked against a render.
- `ocr_merge.rs` — Merging OCR and native text

### Adding CLI Options
1. Add field to `LiteParseConfig` in `config.rs`
2. Add clap arg in `main.rs`
3. Wire through `parser.rs`
4. Expose in binding crates (`liteparse-napi`, `liteparse-python`, `liteparse-wasm`)

### Adding / Modifying Node.js Wrapper
- Edit `packages/node/src/lib.ts` for library API changes
- Edit `packages/node/src/cli.ts` for CLI changes
- The native binary interface is defined in `packages/node/src/native.ts`

### Adding / Modifying Python Wrapper
- Edit `packages/python/liteparse/parser.py` for library API changes
- Types are in `packages/python/liteparse/types.py`
- CLI entry point is `packages/python/liteparse/cli.py`

## Key Dependencies

| Dependency | Purpose |
|------------|---------|
| `pdfium` (C library) | PDF text extraction and rendering |
| `tesseract-rs` | Built-in OCR engine (optional, via `tesseract` feature) |
| `clap` | CLI framework |
| `serde` / `serde_json` | Serialization |
| `tokio` | Async runtime |
| `reqwest` | HTTP client (for OCR server) |
| `image` | Image processing (PNG encoding) |
| `napi-rs` | Node.js native bindings |
| `pyo3` / `maturin` | Python native bindings |
| `wasm-bindgen` | WASM bindings |

## Entry Points

- **Rust CLI**: `crates/liteparse/src/main.rs`
- **Rust Library**: `crates/liteparse/src/lib.rs` → `parser.rs` contains `LiteParse` struct
- **Node.js**: `packages/node/src/lib.ts` exports `LiteParse` class
- **Python**: `packages/python/liteparse/parser.py` exports `LiteParse` class
- **WASM**: `crates/liteparse-wasm/` exposes `LiteParse` via wasm-bindgen

## Related Documentation

- [User-facing documentation](README.md)
- [OCR API Specification](OCR_API_SPEC.md)
- [WASM package README](packages/wasm/README.md)
- [Python package README](packages/python/README.md)
- [OCR server examples](ocr/README.md)
