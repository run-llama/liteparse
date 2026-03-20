# LiteParse Eval Utils

Utilities for generating and evaluating datasets for PDF parsing performance. Compares text extraction quality across multiple PDF parsers using LLM-based QA evaluation.

## Setup

Requires Python 3.12+.

```bash
# Install the package (from the dataset_eval_utils directory)
pip install -e .
```

You'll need an LLM API key for the evaluation tools:
- **Anthropic** (default): Set `ANTHROPIC_API_KEY` environment variable
- **MiniMax**: Set `MINIMAX_API_KEY` environment variable

## Dataset

An existing dataset that was generated and evaluated using this framework can be found on [huggingface]().

You can download the dataset using the Hugging Face CLI:

```bash
hf download run-llama/liteparse-eval-dataset --repo-type dataset --local-dir ./liteparse-eval-dataset
```

## CLI Tools

### `lp-process` — Generate Ground Truth Datasets

Processes PDF and image files using Claude's vision capabilities to generate structured QA ground truth data.

```bash
lp-process /path/to/documents --output-dir ./ground_truth
```

Options:
- `--output-dir` — Directory to save output JSON files (default: `./output`)
- `--model` — Claude model to use (default: `claude-sonnet-4-5-20250929`)
- `--api-key` — Anthropic API key (or set `ANTHROPIC_API_KEY` env var)

Each output JSON file contains document metadata and QA pairs extracted from the document pages.

### `lp-evaluate` — Run QA Evaluation

Evaluates parser text extraction quality by having an LLM answer questions from extracted text and judging correctness against ground truth answers.

```bash
lp-evaluate \
  --data-dir ./documents \
  --ground-truth-dir ./ground_truth \
  --parse-provider liteparse \
  --output ./results/run1
```

Options:
- `--data-dir` — Directory containing source PDF documents (required)
- `--ground-truth-dir` — Directory containing ground truth JSON files (required)
- `--output` — Path to save results (JSON + HTML report)
- `--parse-provider` — Parser to evaluate: `liteparse`, `pymupdf`, `pypdf`, `markitdown` (default: `liteparse`)
- `--llm-provider` — LLM for answering questions: `anthropic`, `minimax` (default: `anthropic`)

Outputs:
- `<output>.json` — Aggregate results with pass rates
- `<output>_detailed.json` — Per-document results with extracted text and individual QA results
- `<output>_report.html` — Interactive HTML report with PDF previews and QA breakdowns

### `lp-benchmark` — Performance Benchmarking

Measures parse latency and memory usage across providers.

```bash
lp-benchmark document.pdf --providers pymupdf liteparse --runs 20
```

Options:
- `--providers` — Providers to benchmark (default: all local providers)
- `--runs` — Number of benchmark runs per provider (default: 10)
- `--warmup` — Number of warmup runs (default: 1)
- `--output` — Path to save JSON results

## Parser Providers

| Provider | Library | Notes |
|----------|---------|-------|
| `liteparse` | [liteparse](https://github.com/run-llama/liteparse) | Spatial text extraction with OCR support |
| `pymupdf` | [PyMuPDF](https://pymupdf.readthedocs.io/) | Fast, mature PDF library |
| `pypdf` | [pypdf](https://pypdf.readthedocs.io/) | Pure-Python PDF library |
| `markitdown` | [MarkItDown](https://github.com/microsoft/markitdown) | Microsoft's document-to-markdown converter |

## Evaluation Pipeline

1. **Extract text** from PDF using the selected parser provider
2. **Answer questions** — LLM reads the extracted text and answers ground truth questions
3. **Judge answers** — A separate LLM judge evaluates whether predicted answers are semantically equivalent to expected answers
4. **Aggregate** — Pass rates are computed per-document and overall
