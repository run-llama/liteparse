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
print(f"Source document pages: {result.total_pages}")

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
from liteparse import LiteParse

parser = LiteParse(
    ocr_enabled=True,              # Enable OCR (default: True)
    ocr_language="eng",            # Tesseract language code
    ocr_server_url=None,           # HTTP OCR server URL (optional)
    tessdata_path=None,            # Path to tessdata directory (optional)
    max_pages=1000,                # Max pages to parse
    target_pages="1-5,10",         # Specific pages (optional)
    extract_screenshots=False,      # Return rendered pages
    screenshot={
        "format": "png",            # "png" (default) or packed "rgb8"
        "png": {
            "compression": "fast",   # "fast" (default), "default", or "best"
        },
    },
    continue_on_page_error=False,   # Skip broken pages and return page_errors
    dpi=150,                       # Rendering DPI
    output_format="json",          # "json" | "text" | "markdown"
    image_mode="placeholder",      # Markdown image handling: "placeholder" | "off" | "embed"
    extract_images=True,           # Extract image bytes + metadata (default: False)
    image_output_dir="./images",   # Write images and return name/path metadata (optional)
    extract_links=True,            # Render [text](url) links in markdown output
    keep_headers_footers=False,    # Keep running headers/footers in markdown output
    extract_vector_graphics=False, # Opt-in shapes + merged H/V lines per page
    extract_annotations=False,     # Include page annotations in structured output
    extract_form_fields=False,      # Include AcroForm widget fields and values
    extract_structure_tree=False,   # Include tagged-PDF logical structure
    preserve_very_small_text=False, # Keep tiny text
    extract_text_metadata=False,    # Opt in to MCID, font metrics, colors, char codes, and trailing_space_generated
    password=None,                 # Password for protected documents
    quiet=False,                   # Suppress progress output
    num_workers=4,                 # Concurrent OCR workers
)
```

When ``extract_images=True``, image extraction is enabled. ``image_output_dir``
requires that explicit opt-in and writes the extracted bytes to disk. Each
``result.images`` entry includes its page bbox, intrinsic pixel dimensions, rotation,
format, ``name``, and ``path``. Valid source JPEGs are preserved, exact duplicates
reuse one file, and JSON CLI output contains metadata only (no base64 image data).
``image_mode`` controls Markdown presentation only and does not imply extraction.
With ``extract_images=False``, lightweight Markdown placement refs are still collected
and ``result.images`` stays empty.

When `extract_annotations` is enabled, each parsed page has an `annotations`
list containing the subtype, contents, author/title, PDF date strings,
viewport-space rectangle and quadpoint rectangles, and URI for external link
annotations. It is independent of `extract_links`, which controls Markdown
link rendering. The field is `None` when extraction is disabled.

When ``extract_structure_tree=True``, each page has a ``structure_tree`` containing
all tagged-PDF roots and recursive elements with type, ID, actual/alternate text,
title, typed attributes, MCIDs, children, and referenced link annotations. Untagged
pages have an empty ``roots`` list; the field is ``None`` when disabled.

Every result also carries ``creator``/``producer`` from the PDF ``/Info``
dictionary. With ``extract_document_metadata=True``, ``result.doc_meta`` adds a
provenance object with dates, PDF version/security, signature state,
incremental-save markers, trailer ID comparison, the catalog's XMP packet
(capped at 64 KiB; skipped for sources over 16 MiB), and source size. It is off by default because it streams the whole source file,
and it is ``None`` for inputs converted from a non-PDF format. These document
fields are API-only and do not alter default CLI JSON.

## Parsing from Bytes

Pass raw PDF bytes directly — useful for web uploads or downloaded files:

```python
with open("document.pdf", "rb") as f:
    result = parser.parse(f.read())
print(result.text)
```

## Screenshots

Generate screenshots of document pages:

```python
screenshots = parser.screenshot("document.pdf", page_numbers=[1, 2, 3])
for s in screenshots:
    print(f"Page {s.page_num}: {s.width}x{s.height}")
    with open(f"page_{s.page_num}.png", "wb") as f:
        f.write(s.image_bytes)
```

Raw RGB avoids an intermediate PNG when another encoder should own the final format:

```python
from io import BytesIO
from PIL import Image
from liteparse import LiteParse

parser = LiteParse(screenshot={"format": "rgb8"})
page = parser.screenshot("document.pdf", page_numbers=[1])[0]
image = Image.frombytes("RGB", (page.width, page.height), page.image_bytes)
output = BytesIO()
image.save(output, format="JPEG", quality=85, optimize=True)
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
`"garbled"`, `"vector-text"`, or `"annotation-text"`. Raw bytes work here too.

## Supported Formats

- PDF (`.pdf`)
- Microsoft Office (`.docx`, `.xlsx`, `.pptx`, etc.) — requires LibreOffice
- OpenDocument (`.odt`, `.ods`, `.odp`) — requires LibreOffice
- Images (`.png`, `.jpg`, `.tiff`, etc.)
- And more!

## CLI

The Python package includes the `lit` CLI:

```bash
lit parse document.pdf
lit parse document.pdf --format json -o output.json
lit parse document.pdf --format json --extract-annotations
lit parse document.pdf --format json --extract-form-fields
lit screenshot document.pdf -o ./screenshots
lit batch-parse ./input ./output
lit is-complex document.pdf
```
