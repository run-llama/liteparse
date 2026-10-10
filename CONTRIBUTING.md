# Contributing to LiteParse

Thank you for your interest in contributing to LiteParse! This document provides guidelines and information for contributors.

## Getting Started

1. Fork the repository
2. Clone your fork:
   ```bash
   git clone https://github.com/YOUR_USERNAME/liteparse.git
   cd liteparse
   ```
3. Install prerequisites (see [Development Prerequisites](#development-prerequisites))

## What to Contribute?

In this project, we welcome a wide range of contributions, but we do want to maintain the spirit of the project. We are primarily focused on:

- Core algorithms for PDF parsing and text extraction
- OCR integrations and improvements
- Different types or modifications to output formats

We are less interested in:

- Markdown output
- Any LLM integration or agent code
- Anything that doesn't directly relate to improving the core parsing and extraction capabilities

## Architecture Overview

LiteParse is written in Rust with bindings for multiple platforms:

```
crates/
├── liteparse/           # Core Rust library (parsing, grid projection, OCR, output)
├── pdfium-sys/          # Raw FFI bindings to PDFium (auto-downloads pdfium)
├── pdfium/              # Safe Rust wrapper around pdfium-sys
├── liteparse-napi/      # Node.js native addon (napi-rs)
├── liteparse-python/    # Python extension module (PyO3 + maturin)
└── liteparse-wasm/      # WebAssembly bindings (wasm-bindgen + wasm-pack)

packages/
├── node/                # Node.js package (@llamaindex/liteparse)
├── python/              # Python package (liteparse)
└── wasm/                # WASM package (@llamaindex/liteparse-wasm)
```

## Development Prerequisites

You'll need the following tools installed:

| Tool | Purpose | Install |
|------|---------|---------|
| **Rust toolchain** | Core library and all bindings | [rustup.rs](https://rustup.rs) |
| **napi-rs CLI** | Node.js native addon builds | `npm i -g @napi-rs/cli` |
| **maturin** | Python extension builds | `pip install maturin` |
| **wasm-pack** | WebAssembly builds | `cargo install wasm-pack` |

PDFium is **auto-downloaded** by the `pdfium-sys` build script — no manual setup needed. For WASM, a static `libpdfium.a` is downloaded and linked into the `.wasm` binary. For native targets, a shared library (`.dylib`/`.so`/`.dll`) is downloaded and copied to the build output.

## Building

### Important: Workspace-wide `cargo build` will fail

The binding crates (`liteparse-python`, `liteparse-napi`, `liteparse-wasm`) each require their own specialized toolchain to link correctly. A plain `cargo build` at the workspace root will fail because, for example, the Python bindings need a Python interpreter to resolve `_Py*` symbols.

### Core Rust library only

```bash
cargo build -p liteparse
```

### Node.js bindings (napi-rs)

```bash
cd packages/node
npm install
npm run build          # Builds Rust → .node addon, copies pdfium, compiles TS
```

Individual steps:
```bash
npm run build:rs       # napi build (compiles liteparse-napi crate)
npm run build:pdfium   # Copies pdfium shared library alongside the addon
npm run build:ts       # Compiles TypeScript wrapper
```

To test locally, import from the package directly or use `npm link`:
```js
import { LiteParse } from './packages/node/dist/lib.js';
```

### Python bindings (maturin + PyO3)

```bash
cd packages/python
maturin develop        # Builds Rust and installs into active virtualenv
```

`maturin develop` compiles the `liteparse-python` crate and installs the resulting package into your current Python virtual environment. Then test with:
```python
import liteparse
```

### WASM bindings (wasm-pack)

```bash
cd packages/wasm
npm run build          # Browser target (--target web)
npm run build:bundler  # Bundler target (webpack/vite)
npm run build:nodejs   # Node.js target
```

PDFium is statically linked into the `.wasm` binary — the output in `packages/wasm/pkg/` is fully self-contained.

Run the WASM OCR stage tests from the repository root:

```bash
NODE_PATH="$PWD/scripts/browser-compat/stage-test-support" \
  wasm-pack test --node crates/liteparse-wasm --test ocr_stages
```

These tests run the WASM recognition stage in Node.js. They use a mock OCR
engine and control job completion without timers. The test support module
resolves linked WASI imports and fails if a test calls one of them.

## Development Workflow

### Testing Local Changes

```bash
# Parse a document (Node.js CLI)
cd packages/node && npm run build
node dist/cli.js parse document.pdf

# Python CLI
cd packages/python && maturin develop
lit parse document.pdf
```

### Linting & Formatting

```bash
cargo fmt              # Format Rust code
cargo clippy           # Lint Rust code
```

### Debugging Grid Projection

When working on the grid projection algorithm, you can enable built-in debug logging and visual output instead of adding ad-hoc `console.log` statements.

**Debug logging** traces every decision the projection makes — block detection, anchor extraction, snap assignment, rendering, and flowing text classification:

```bash
lit parse document.pdf --debug
lit parse document.pdf --debug --debug-page 3
lit parse document.pdf --debug --debug-text-filter "Total" "Revenue"
lit parse document.pdf --debug --debug-region "0,100,300,200"
lit parse document.pdf --debug --debug-output ./debug-output
```

**Visual grid export** generates PNG images showing text boxes color-coded by snap type (blue=left, red=right, green=center, gray=floating, yellow=flowing) with anchor lines overlaid:

```bash
lit parse document.pdf --debug-visualize
lit parse document.pdf --debug-visualize --debug-output ./my-debug
```

## Pull Requests

1. Fork and create a feature branch from `main`
2. Make your changes
3. Ensure linting passes (`cargo fmt --check && cargo clippy`)
4. Submit a pull request

When you submit a PR, a number of CICD checks will run. Among these, your code will be tested against a regression suite of documents to ensure that your changes don't break existing parsing capabilities. It will be up to the maintainers discretion to determine if any changes to the regression set are expected/positive or unexpected/negative.

### PR Guidelines

- Keep PRs focused on a single change
- Update documentation if needed
- Add tests for new functionality
- For parsing issues, include a test document if possible

## Reporting Issues

### Parsing Issues

If you're reporting a problem with document parsing:

1. **You must attach the document** or provide a way to reproduce the issue
2. Include the command you ran
3. Show the expected vs actual output
4. Include your LiteParse version (`lit --version`)

Issues without reproducible examples will be closed.

### Bug Reports

For other bugs:
1. Describe what you expected vs what happened
2. Include steps to reproduce
3. Include error messages/stack traces
4. Include version information

## Questions?

- Open a [Discussion](https://github.com/run-llama/liteparse/discussions) for questions
- Check existing issues before opening new ones
- Read the [README](README.md) for usage documentation

## License

By contributing, you agree that your contributions will be licensed under the Apache 2.0 License.
