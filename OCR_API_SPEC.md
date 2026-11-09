# LiteParse OCR API Specification

This document defines the standard HTTP API that OCR servers must implement to work with LiteParse.

## Overview

LiteParse expects a simple HTTP endpoint that accepts an image and returns text with bounding boxes. Your OCR server can internally use any OCR engine (EasyOCR, PaddleOCR, Tesseract, Cloud APIs, etc.) as long as it conforms to this API.

## Endpoint

```
POST /ocr
```

## Request Format

**Content-Type:** `multipart/form-data`

**Fields:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `file` | binary | Yes | Image file (PNG, JPG, etc.) |
| `language` | string | No | Language code (default: `en`) |
| `correct_rotation` | string | No | `true` lets the server detect and undo the page's rotation before reading it; `false` reads the page as sent. |
| `return_word_boxes` | string | No | `true` asks a line-level server to also measure each word's box (see `results[].words`). |
| `dense_ocr_detection` | string | No | `true` runs denser text detection. |
| `return_bbox_angles` | string | No | `true` returns each line and word as its rotated quad, with its angle. |

LiteParse sends `file` and `language`, plus every field in `ocr_request_fields` / `ocrRequestFields` as given, with the page's entry in `ocr_page_request_fields` / `ocrPageRequestFields` merged over them. A `language` entry replaces the default. LiteParse adds no other field, so `correct_rotation`, `return_word_boxes`, `dense_ocr_detection` and `return_bbox_angles` are sent only when the caller sets them. Servers may ignore any of these fields.

### Language Codes

Use ISO 639-1 two-letter codes:
- `en` - English
- `zh` - Chinese
- `ja` - Japanese
- `ko` - Korean
- `fr` - French
- `de` - German
- `es` - Spanish
- `ar` - Arabic
- etc.

Your server should map these to whatever format your underlying OCR engine expects.

## Response Format

**Content-Type:** `application/json`

**Structure:**

```json
{
  "results": [
    {
      "text": "recognized text",
      "bbox": [x1, y1, x2, y2],
      "confidence": 0.95,
      "polygon": [[x1, y1], [x2, y2], [x3, y3], [x4, y4]],
      "words": [{"text": "recognized", "bbox": [x1, y1, x2, y2]}]
    }
  ]
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `results` | array | Array of text detection results |
| `results[].text` | string | Recognized text content |
| `results[].bbox` | [number, number, number, number] | Axis-aligned bounding box `[x1, y1, x2, y2]` where (x1,y1) is top-left and (x2,y2) is bottom-right |
| `results[].confidence` | number | Confidence score between 0.0 and 1.0 |
| `results[].polygon` | [[number, number], ×4] | **Optional.** 4-point detection polygon ordered top-left → top-right → bottom-right → bottom-left **in the glyphs' upright reading frame**. Lets LiteParse recover rotation for vertical/sideways text. |
| `results[].words` | array | **Optional.** Measured word boxes inside this result, in reading order, each `{"text", "bbox": [x1, y1, x2, y2]}` in image pixels. LiteParse uses them as the result's word boxes when their count matches the words in `text` after cleanup. Otherwise, and when a result has no measured words, the result keeps one box for the whole line. |

A server that returns the `{"result": [[quad, text, score], ...]}` shape may send the measured words as a top-level `words` list parallel to `result`: `words[i]` is a list of `{"bbox": [[x, y] ×4], "text", "r"?}` for `result[i]`, where `r` is the word's rotation in the same convention as `result_meta` below. LiteParse drops all of them when the two lists differ in length. Each word's box is the bounds of its quad.

The same shape may also send `result_meta`, a list parallel to `result`: `result_meta[i]` is `{"r": degrees}`, the rotation of `result[i]` clockwise on screen in (-180, 180], or `{}` when the line is upright. LiteParse takes each line's rotation from it, snapped to the nearest right angle, and drops the whole list when its length differs from `result`. A line with no `result_meta` angle takes the angle of its first measured word that has one. With neither, LiteParse infers rotation from the order of the quad's points.

## Example

### Request

```bash
curl -X POST http://localhost:8080/ocr \
  -F "file=@document.png" \
  -F "language=en"
```

### Response

```json
{
  "results": [
    {
      "text": "Hello",
      "bbox": [10, 20, 60, 40],
      "confidence": 0.98
    },
    {
      "text": "World",
      "bbox": [70, 20, 130, 40],
      "confidence": 0.97
    }
  ]
}
```

## Error Handling

Return appropriate HTTP status codes:

- `200 OK` - Success
- `400 Bad Request` - Invalid request (missing file, invalid language, etc.)
- `500 Internal Server Error` - OCR processing failed

Error response format:

```json
{
  "error": "Description of the error"
}
```

## Implementation Notes

### Coordinate System

- Origin (0,0) is at the **top-left** of the image
- X increases to the right
- Y increases downward
- All coordinates are in pixels

### Bounding Box Format

Always return axis-aligned bounding boxes as `[x1, y1, x2, y2]`:
- `x1, y1` = top-left corner
- `x2, y2` = bottom-right corner
- `x2 > x1` and `y2 > y1`

If your OCR engine returns rotated boxes or polygon coordinates, convert them to axis-aligned boxes by taking min/max coordinates. **Additionally**, you are encouraged to forward the raw 4-point polygon as `polygon` (TL → TR → BR → BL in the upright reading frame) — LiteParse uses it to detect vertical/sideways text (e.g. legal-document sidebars) and route it through its rotation reading-order handler instead of flattening it into body lines.

### Confidence Scores

- Normalize to range 0.0 to 1.0
- 1.0 = 100% confident
- 0.0 = 0% confident
- If your OCR engine doesn't provide confidence, use `1.0`

### Text Ordering

Results should be ordered by reading order (top-to-bottom, left-to-right for most languages).

## Example Implementations

See the `/ocr` directory for reference implementations:

- `ocr/easyocr/` - Wrapper for EasyOCR
- `ocr/paddleocr/` - Wrapper for PaddleOCR
- `ocr/suryaocr/` - Wrapper for Surya OCR 2 (multilingual)
- `ocr/indraocr/` - Wrapper for Indra OCR (fast, CPU only)

## Testing Your Server

Quick test:

```bash
# 1. Start your server
python server.py

# 2. Test with curl
curl -X POST http://localhost:8080/ocr \
  -F "file=@test.png" \
  -F "language=en" \
  | jq .

# 3. Expected output:
# {
#   "results": [
#     {
#       "text": "...",
#       "bbox": [x1, y1, x2, y2],
#       "confidence": 0.xx
#     }
#   ]
# }
```

Use with LiteParse:

```bash
lit parse document.pdf --ocr-server-url http://localhost:8080/ocr
```

## FAQ

### Q: What if my OCR returns rotated bounding boxes?

Convert to axis-aligned boxes:

```python
def polygon_to_bbox(polygon):
    """Convert polygon [[x1,y1], [x2,y2], ...] to [x1, y1, x2, y2]"""
    xs = [point[0] for point in polygon]
    ys = [point[1] for point in polygon]
    return [min(xs), min(ys), max(xs), max(ys)]
```

### Q: What if my OCR doesn't return confidence scores?

Just return `1.0` for all results.

### Q: Can I return empty results?

Yes, return `{"results": []}` if no text is detected.

### Q: Should I filter low-confidence results?

You can, but LiteParse will also handle filtering based on its own thresholds.

### Q: What image formats should I accept?

At minimum: PNG, JPG. Optionally: TIFF, WebP, BMP, GIF.

### Q: Should I handle rotation correction?

Optional. If your OCR engine supports it, you can auto-correct rotation before processing.

### Q: What about multi-page documents?

LiteParse handles page splitting. Your server only needs to process single images.

### Q: Performance considerations?

- Keep server response time under 10 seconds per image
- Support concurrent requests
- Consider GPU acceleration for better performance
- Cache OCR models in memory (don't reload per request)

## Compliance Checklist

- [ ] Accepts `POST /ocr` endpoint
- [ ] Accepts `file` and `language` form fields
- [ ] Returns JSON with `results` array
- [ ] Each result has `text`, `bbox`, and `confidence`
- [ ] Bounding boxes in `[x1, y1, x2, y2]` format
- [ ] (Optional but recommended) `polygon` field with 4-point TL→TR→BR→BL polygon for rotated detections
- [ ] Confidence normalized to 0.0-1.0 range
- [ ] Returns 200 status on success
- [ ] Returns appropriate error codes and messages
- [ ] Handles common image formats (PNG, JPG)
- [ ] Processes images in under 10 seconds

## Support

Questions? Open an issue on GitHub or refer to the example implementations in `/ocr`.
