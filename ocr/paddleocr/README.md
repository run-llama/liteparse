# PaddleOCR Service

This is a simple Flask server that wraps PaddleOCR to conform to the LiteParse OCR API specification (see `../../OCR_API_SPEC.md`).

PaddleOCR is especially fast and accurate for Chinese, Japanese, and Korean languages.

## Build and Run

```bash
# Build Docker image
docker build -t liteparse-paddleocr .

# Run container
docker run -p 8829:8829 liteparse-paddleocr
```

## Usage

The service exposes a single endpoint:

- `POST /ocr` - Perform OCR on an uploaded image

### Parameters

- `file` - Image file (multipart/form-data)
- `language` - Language code (e.g., 'en', 'zh', 'ja', 'ko')

### Example

```bash
curl -X POST -F "file=@image.png" -F "language=zh" http://localhost:8829/ocr
```

### Response Format

```json
{
  "results": [
    {
      "text": "recognized text",
      "bbox": [x1, y1, x2, y2],
      "confidence": 0.95
    }
  ]
}
```

This conforms to the LiteParse OCR API specification.

## Supported Languages

PaddleOCR supports 80+ languages with excellent support for CJK:

- `en` - English
- `zh` / `zh-cn` - Chinese (Simplified)
- `zh-tw` / `zh-hant` - Chinese (Traditional)
- `ja` - Japanese
- `ko` - Korean
- `fr` - French
- `de` - German
- `es` - Spanish
- `pt` - Portuguese
- `ru` - Russian
- `ar` - Arabic
- `hi` - Hindi/Devanagari

Full list: https://github.com/PaddlePaddle/PaddleOCR

## Performance

PaddleOCR is optimized for speed and accuracy:

- **Fast**: 2-3x faster than EasyOCR
- **Accurate**: Especially for Asian languages (Chinese, Japanese, Korean)
- **Lightweight**: Smaller model sizes

## Use with LiteParse

Once the server is running, use it with LiteParse:

```bash
# Parse with PaddleOCR
pnpm parse document.pdf --ocr-server-url http://localhost:8829/ocr

# With specific language
pnpm parse document.pdf --ocr-server-url http://localhost:8829/ocr --ocr-language zh
```

Or in code:

```typescript
import { LiteParse } from 'liteparse';

const parser = new LiteParse({
  ocrServerUrl: 'http://localhost:8829/ocr',
  ocrLanguage: 'zh',
});

const result = await parser.parse('document.pdf');
```

## GPU Support

For GPU acceleration, modify the Dockerfile to use a CUDA base image and set `use_gpu=True` in `server.py`.

## Notes

- First request may be slow as PaddleOCR downloads models
- Models are cached after first use
- Default port is 8829 (different from EasyOCR's 8828)
