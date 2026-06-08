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
    preserve_very_small_text=False, # Keep tiny text
    password=None,                 # Password for protected documents
    quiet=False,                   # Suppress progress output
    num_workers=4,                 # Concurrent OCR workers
    inline_images=False,           # Inline embedded images as base64 markdown images
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
```
