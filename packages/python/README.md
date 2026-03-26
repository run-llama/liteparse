# LiteParse Python

Python wrapper for [LiteParse](https://github.com/run-llama/liteparse) - fast, lightweight document parsing with optional OCR.

## Installation

```bash
pip install liteparse
```

While the python package can auto-install the LiteParse CLI if not installed, it is recommended to do it separately:

```bash
npm install -g liteparse
# or
npx liteparse --version
```

## Quick Start

```python
from liteparse import LiteParse

# Create parser
parser = LiteParse()

# Parse a document
result = parser.parse("document.pdf")
print(result.text)

# Access structured data
for page in result.pages:
    print(f"Page {page.pageNum}: {len(page.textItems)} text items")
```

## Configuration

```python
from liteparse import LiteParse

parser = LiteParse()

result = parser.parse(
    "document.pdf",
    ocr_enabled=False,
    max_pages=10,
    dpi=150,
    preserve_small_text=True,
)
print(result.text)
```

## Batch Processing

For parsing multiple files, batch mode is significantly faster as it reuses the PDF engine:

```python
from liteparse import LiteParse

parser = LiteParse(ocr_enabled=False)

# Parse all documents in a directory
result = parser.batch_parse(
    input_dir="./documents",
    output_dir="./output",
    recursive=True,              # Include subdirectories
    extension_filter=".pdf",     # Only PDF files
)

print(f"Parsed {result.success_count} files in {result.total_time_seconds}s")
print(f"Average: {result.avg_time_ms}ms per file")
```

## Supported Formats

- PDF (`.pdf`)
- Microsoft Office (`.docx`, `.xlsx`, `.pptx`, etc.) - requires LibreOffice
- OpenDocument (`.odt`, `.ods`, `.odp`) - requires LibreOffice
- Images (`.png`, `.jpg`, `.tiff`, etc.) - requires ImageMagick
- And more!

## Performance Tips

1. **Disable OCR** if your documents have selectable text:
   ```python
   parser = LiteParse(ocr_enabled=False)
   ```

2. **Use batch mode** for multiple files to avoid cold-start overhead:
   ```python
   parser.batch_parse("./input", "./output")
   ```

3. **Limit pages** if you only need specific pages:
   ```python
   result = parser.parse("doc.pdf", target_pages="1-5")
   ```
