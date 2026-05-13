---
"@llamaindex/liteparse": minor
---

Add `ocrTextMode` with an `ocr-only` option for pages where OCR runs. This lets callers ignore native PDF text on OCR'd pages when the document has a corrupted or hidden text layer, while keeping the existing merge behavior as the default.
