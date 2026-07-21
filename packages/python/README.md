# LiteParse Python

Python bindings for [LiteParse](https://github.com/run-llama/liteparse) — fast, lightweight PDF and document parsing with spatial text extraction.

## Installation

```bash
pip install liteparse
```

This also installs the `lit` CLI command.

## Quick Start

```python
from liteparse import LiteParse

parser = LiteParse()
result = parser.parse("document.pdf")
print(result.text)

# Access structured data
for page in result.pages:
    print(f"Page {page.page_num}: {len(page.text_items)} text items")
```

## Markdown Output

LiteParse can render documents directly to Markdown including headings, tables, lists,
images, and links reconstructed from the spatial layout. Great for feeding LLMs
and RAG pipelines. The rendered Markdown is returned on `result.text`:

```python
parser = LiteParse(
    output_format="markdown",   # "json" | "text" | "markdown"
    image_mode="placeholder",   # "placeholder" | "off" | "embed"
    extract_links=True,         # render [text](url) link syntax (default: True)
)
result = parser.parse("document.pdf")
print(result.text)  # rendered Markdown
```

> Reconstruction quality varies with document complexity.

## Configuration

All options are passed to the constructor:

```python
parser = LiteParse(
    ocr_enabled=True,              # Enable OCR (default: True)
    ocr_language="eng",            # Tesseract language code
    ocr_server_url=None,           # HTTP OCR server URL (optional)
    tessdata_path=None,            # Path to tessdata directory (optional)
    max_pages=1000,                # Max pages to parse
    target_pages="1-5,10",         # Specific pages (optional)
    dpi=150,                       # Rendering DPI
    output_format="json",          # "json" | "text" | "markdown"
    image_mode="placeholder",      # Markdown image handling: "placeholder" | "off" | "embed"
    extract_links=True,            # Render [text](url) links in markdown output
    preserve_very_small_text=False, # Keep tiny text
    include_text_metadata=False,    # Opt in to MCID, font metrics, colors, char codes, and tsg
    password=None,                 # Password for protected documents
    quiet=False,                   # Suppress progress output
    num_workers=4,                 # Concurrent OCR workers
)
```

## Parsing from Bytes

Pass raw PDF bytes directly — useful for web uploads or downloaded files:

```python
with open("document.pdf", "rb") as f:
    result = parser.parse(f.read())
print(result.text)
```

## Screenshots

Generate PNG screenshots of document pages:

```python
screenshots = parser.screenshot("document.pdf", page_numbers=[1, 2, 3])
for s in screenshots:
    print(f"Page {s.page_num}: {s.width}x{s.height}")
    with open(f"page_{s.page_num}.png", "wb") as f:
        f.write(s.image_bytes)
```

## Document Complexity

Before committing to a full parse, check whether a document needs OCR or heavier
processing. `is_complex` is a cheap, text-layer-only pass that returns one entry per page
with a `needs_ocr` verdict and the signals behind it — useful for routing documents to
different pipelines, rejecting ones you can't handle, or estimating cost.

```python
parser = LiteParse()
pages = parser.is_complex("document.pdf")

if any(p.needs_ocr for p in pages):
    # Route to the OCR-enabled pipeline
    result = parser.parse("document.pdf")
else:
    # Cheap path — skip OCR entirely
    result = LiteParse(ocr_enabled=False).parse("document.pdf")

# Inspect why specific pages were flagged
for page in pages:
    if page.needs_ocr:
        print(f"Page {page.page_number}: {', '.join(page.reasons)}")
```

`reasons` is one of `"scanned"`, `"no-text"`, `"sparse-text"`, `"embedded-images"`,
`"garbled"`, or `"vector-text"`. Raw bytes work here too.

## Supported Formats

- PDF (`.pdf`)
- Microsoft Office (`.docx`, `.xlsx`, `.pptx`, etc.) — requires LibreOffice
- OpenDocument (`.odt`, `.ods`, `.odp`) — requires LibreOffice
- Images (`.png`, `.jpg`, `.tiff`, etc.) — requires ImageMagick
- And more!

## CLI

The Python package includes the `lit` CLI:

```bash
lit parse document.pdf
lit parse document.pdf --format json -o output.json
lit screenshot document.pdf -o ./screenshots
lit batch-parse ./input ./output
lit is-complex document.pdf
```
