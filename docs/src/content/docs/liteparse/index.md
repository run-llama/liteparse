---
title: What is LiteParse?
description: Fast, local PDF parsing with spatial text parsing, OCR, and bounding boxes.
sidebar:
  order: 0
---

LiteParse is an open-source document parsing library that parses text with spatial layout information and bounding boxes. Written in Rust for speed and reliability, it runs entirely on your machine with no cloud dependencies, no LLMs, and no API keys.

LiteParse is designed specifically for use cases that require fast, accurate text parsing: real-time applications, coding agents, and local workflows. It provides a simple CLI and library API for parsing PDFs, Office documents, and images, with built-in OCR support.

<img src="https://github.com/user-attachments/assets/67830896-4705-4051-9bdd-aa2d88fbb63f" alt="out">

## What can LiteParse do?

- **Parse PDFs** with precise spatial layout. Text comes back positioned where it appears on the page
- **Render to Markdown** with headings, tables, lists, images, and links — clean structured output for LLMs and RAG pipelines
- **Extract bounding boxes** for every text line, ready for downstream processing or visualization
- **OCR scanned documents** using OCRia-nano-1, built-in Tesseract, or your own OCR server
- **Parse Office files and images** with support for DOCX, XLSX, PPTX, PNG, JPG, and more via automatic conversion
- **Screenshot PDF pages** as high-quality images for LLM-based workflows
- **Pull structured PDF data** — embedded images, vector graphics, annotations, AcroForm fields, and tagged-structure trees
- **Score document complexity** before you parse, so you can route scanned or multi-column documents to the right pipeline
- **Use from Node.js/TypeScript, Python, Rust, or the browser (WASM)** — whatever fits your stack

## Using OCRia-nano-1 with LiteParse

OCRia-nano-1 is our 8.1M parameter model pipeline for better parsing with LiteParse, built to be the fastest and most accurate 
parser that runs locally on your CPU. Its open weights are on Hugging Face under a modified OpenRAIL license. 

- [Learn how to use it](/liteparse/guides/ocr/#ocria-nano-1-liteparses-own-models)
- [Read the launch blog](https://www.llamaindex.ai/blog/introducing-ocria-nano-1)
- [Read the technical report](https://www.llamaindex.ai/ocria/ocria-nano-1-report.pdf)
- [Find it on Hugging Face](https://huggingface.co/llamaindex/OCRia-nano-1)

## Get started

- [Getting started](/liteparse/getting_started/): Install LiteParse and parse your first document.
- [Markdown output](/liteparse/guides/markdown/): Render documents to clean, structured Markdown.
- [Extraction options](/liteparse/guides/extraction/): Opt in to images, form fields, annotations, and more.
- [Document complexity](/liteparse/guides/complexity/): Detect scanned, multi-column, and table-heavy pages up front.
- [Library usage](/liteparse/guides/library-usage/): Use LiteParse from TypeScript or Python code.
- [Browser usage (WASM)](/liteparse/guides/browser-usage/): Run LiteParse in the browser with zero server dependencies.
- [Agent skill](/liteparse/guides/agent-skill/): Give Claude Code, Cursor, or Codex the ability to parse documents locally.
- [CLI reference](/liteparse/cli-reference/): Complete command and option reference.
- [API reference](/liteparse/api/): Detailed API documentation (rust) for all public types and functions. The same types apply across all language bindings.
