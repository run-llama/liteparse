---
title: "OCRia-nano-1"
description: OCRia-nano-1 is LlamaIndex's 8.1M-parameter OCR, table and layout model pipeline built for LiteParse.
sidebar:
  label: OCRia-nano-1
  order: 5
---

OCRia-nano-1 is LiteParse's own OCR model pipeline, built to run fast on CPU. It is 8.1M parameters in
total and its open weights are on [Hugging Face](https://huggingface.co/llamaindex/OCRia-nano-1).

With OCRia-nano-1, LiteParse scores 59.2 on ParseBench-2 (vs. 48.7 with Tesseract) and parses over twice as many pages per
second, at 426 ms per page on a single CPU core. The model is available for personal and research use, under a modified 
OpenRAIL license.

- [Launch blog](https://www.llamaindex.ai/blog/introducing-ocria-nano-1)
- [Technical report (PDF)](https://www.llamaindex.ai/ocria/ocria-nano-1-report.pdf)
- [Model card on Hugging Face](https://huggingface.co/llamaindex/OCRia-nano-1)

## What's in OCRia-nano-1

- **Text-line detector**: finds text lines on your documents.
- **Script router**: picks a recognizer for each line, so documents that mix scripts get the best recognizer.
- **Four recognizers**: Latin, CJK (Chinese, Japanese, Korean), right-to-left (Arabic, Hebrew, ...) and a world-script
  model covering about 17 more scripts.
- **LayoutNet**: a joint table and layout model that finds tables, reconstructs their cells and groups layout blocks together.

Everything runs in-process through ONNX Runtime, which is downloaded once with the models.

## Install

The weights are not bundled with LiteParse. You can download them with:

```bash
lit models pull ocria-nano     # models + ONNX Runtime (60-75 MB), sha256 verified
lit models list                # installed packages
lit models rm ocria-nano       # remove
```

Offline or air-gapped machines can install from a directory holding the package files:

```bash
lit models pull ocria-nano --from <dir>
```

`LITEPARSE_MODEL_DIR` moves the model directory (default `~/.cache/liteparse/models`), and `ORT_DYLIB_PATH` points at
an existing ONNX Runtime library.

## Use it from the CLI

Once pulled, OCRia-nano-1 becomes the default backend:

```bash
lit parse scan.pdf                           # OCRia-nano-1 OCR + LayoutNet tables
lit parse scan.pdf --format markdown         # tables come out as Markdown tables
lit parse scan.pdf --ocr-backend tesseract   # back to Tesseract for one run
```

- Pages render at 250 dpi (Tesseract: 150). `--dpi` overrides it.
- `--ocr-language` is ignored: OCRia-nano-1 detects each line's script and picks the recognizer itself.

See the [CLI reference](/liteparse/cli-reference/#lit-models) for every option.

## Use it from Python

```python
from liteparse import LiteParse, models

models.pull("ocria-nano")  # once
result = LiteParse(ocr_backend="ocria-nano").parse("scan.pdf")
```

## Use it from Node.js / TypeScript

```typescript
import { LiteParse, models } from "@llamaindex/liteparse";

await models.pull("ocria-nano"); // once
const result = await new LiteParse({ ocrBackend: "ocria-nano" }).parse("scan.pdf");
```

## Use it in the browser (WASM)

The WASM package runs the same OCRia-nano-1 models inside WebAssembly, in browsers, Node, Deno and Cloudflare Workers.
The models download on first use:

```ts
import init, { LiteParse, OcriaNano } from "@llamaindex/liteparse-wasm";
await init();

const ocria = await OcriaNano.fromUrl(); // downloads from Hugging Face, checks sha256
const parser = LiteParse.withOcriaNano({ ocrEnabled: true, outputFormat: "markdown" }, ocria);
const result = await parser.parse(pdfBytes);
```

`OcriaNano.fromUrl({ scripts: ["latin"], layout: false })` is the smallest setup (1.5 MB of models). See
[Browser usage](/liteparse/guides/browser-usage/#ocr-with-ocria-nano-1) for self-hosting and bundling the files.

## Benchmarks

From the [OCRia-nano-1 technical report](https://www.llamaindex.ai/ocria/ocria-nano-1-report.pdf):

| System (CPU) | ParseBench-2 | OmniParseBench | ParseBench-1 | olmOCR-bench | BHL CER ↓ | pages/s |
|---|---:|---:|---:|---:|---:|---:|
| **LiteParse + OCRia-nano-1** | **59.2** | **74.8** | **52.7** | **60.9** | **0.048** | 27.8 |
| Indra 1.1.2 | 55.1 | 70.9 | 42.6 | 60.8 | 0.062 | **29.6** |
| Marker 2.2.0 (fast) | 52.5 | 69.8 | 40.4 | 60.8 | 0.067 | 24.7 |
| LiteParse (Tesseract, default) | 48.7 | 42.3 | 40.5 | 45.7 | 0.275 | 14.2 |

## License

The OCRia-nano-1 weights are licensed separately from LiteParse, under a modified OpenRAIL license (see the `LICENSE` in
the [model repository](https://huggingface.co/llamaindex/OCRia-nano-1)).
