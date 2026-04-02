# liteparse-pypdf-compatible

Drop-in replacement for [pypdf](https://github.com/py-pdf/pypdf) powered by [LiteParse](https://github.com/run-llama/liteparse) for higher quality text extraction.

**How it works:** `extract_text()` uses LiteParse's spatial reconstruction engine for superior text extraction (especially multi-column layouts and scanned documents with OCR). Everything else — metadata, images, annotations, outlines, form fields, page manipulation — is delegated to the real pypdf library.

## Installation

```bash
pip install liteparse-pypdf-compatible
```

**Prerequisite:** The LiteParse Node.js CLI must be installed:

```bash
npm install -g @llamaindex/liteparse
```

## Usage

Replace your pypdf import — no other code changes needed:

```python
# Before
from pypdf import PdfReader

# After
from liteparse_pypdf import PdfReader

reader = PdfReader("document.pdf")

for page in reader.pages:
    text = page.extract_text()
    print(text)

print(f"Title: {reader.metadata.title}")
print(f"Pages: {len(reader.pages)}")
```

## What's enhanced vs delegated

### Enhanced by LiteParse

| Feature | Benefit |
|---|---|
| `page.extract_text()` | Spatial text reconstruction with proper column alignment |
| `page.extract_text(visitor_text=...)` | Callback with LiteParse text items |
| OCR support | Automatic OCR on scanned/image-heavy pages |

### Delegated to pypdf

Everything else passes through to the real pypdf, including:

- `reader.metadata` — title, author, subject, creator, dates
- `reader.outline` — bookmarks
- `reader.page_labels` — page label strings
- `reader.named_destinations` — named destinations
- `reader.attachments` — file attachments
- `reader.get_fields()` / `reader.get_form_text_fields()` — form data
- `reader.xfa` — XFA form data
- `reader.page_layout` / `reader.page_mode` — viewer settings
- `reader.is_encrypted` / `reader.pdf_header`
- `page.images` — embedded images
- `page.annotations` — page annotations
- `page.mediabox` / `cropbox` / `trimbox` / `artbox` / `bleedbox` — page boxes
- `page.rotation` / `page.user_unit`
- `page.rotate()` / `page.scale()` / `page.merge_page()` — page manipulation
- `page["/Resources"]` — dict-like PDF dictionary access
- `PdfWriter` — re-exported directly from pypdf

### Re-exported from pypdf

These are available directly from `liteparse_pypdf` so you don't need a separate pypdf import:

```python
from liteparse_pypdf import (
    PdfReader,          # Enhanced with LiteParse
    PdfWriter,          # Direct from pypdf
    DocumentInformation,
    PdfReadError,
    PdfStreamError,
    PageSizeNotDefinedError,
    FileNotDecryptedError,
    PdfReadWarning,
)
```

## LiteParse extras

Pages expose additional spatial data not available in pypdf:

```python
page = reader.pages[0]
page.text_items      # List of TextItem with x, y, width, height, font info
page.bounding_boxes  # List of BoundingBox with x1, y1, x2, y2
```

## LiteParse options

Pass LiteParse-specific options to the reader constructor:

```python
reader = PdfReader(
    "document.pdf",
    ocr_enabled=True,       # Enable OCR for scanned pages
    ocr_language="en",      # OCR language
    dpi=300,                # Higher DPI for better OCR
    ocr_server_url="...",   # Optional HTTP OCR server
)
```
