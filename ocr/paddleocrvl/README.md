# PaddleOCR-VL Service

A FastAPI server wrapping [PaddleOCR-VL 1.6](https://huggingface.co/PaddlePaddle/PaddleOCR-VL-1.6)
to conform to the LiteParse OCR API specification (see `../../OCR_API_SPEC.md`).

PaddleOCR-VL 1.6 is a compact (0.9B-parameter) document parsing model that
pairs a lightweight layout detector with an ERNIE-based vision-language model.
It holds the top score on OmniDocBench v1.6 (96.33) and reads text, tables,
formulas, and charts across 109 languages — a single model handles all
languages with no per-language setup. Apache 2.0 licensed.

## Build and Run

```bash
# install and run (in one command)
uv run server.py
```

The first run downloads the layout detector and the 0.9B VL model weights
(~2 GB total) and may take a few minutes; weights are cached afterward.

The server listens on port **8831**:

```bash
lit parse document.pdf --ocr-server-url http://localhost:8831/ocr
```

## Inference backends

By default everything runs locally on CPU through PaddlePaddle — no extra
setup, works on Linux, macOS, and Windows. A 0.9B VLM on CPU is accurate but
slow (tens of seconds per page), so for real workloads pick one of:

- **PyTorch GPU (`transformers` engine):** install the extra and set the
  engine — the VL model runs through PyTorch/CUDA while the layout detector
  stays on CPU. This is the practical GPU path on Windows, where vLLM and
  SGLang cannot run natively.

  ```bash
  uv sync --extra transformers
  PADDLEOCR_VL_ENGINE=transformers uv run server.py
  ```

  (For NVIDIA GPUs install a CUDA build of PyTorch, e.g.
  `uv pip install torch --index-url https://download.pytorch.org/whl/cu128`.)

- **External inference server (vLLM/SGLang):** run the official
  PaddleOCR genai server image and point this server at it — the highest
  throughput option:

  ```bash
  docker run --rm --gpus all -p 8080:8080 \
    ccr-2vdh3abv-pub.cnc.bj.baidubce.com/paddlepaddle/paddleocr-genai-vllm-server:latest-nvidia-gpu \
    paddleocr genai_server --model_name PaddleOCR-VL-1.6-0.9B \
    --host 0.0.0.0 --port 8080 --backend vllm

  PADDLEOCR_VL_SERVER_URL=http://127.0.0.1:8080/v1 uv run server.py
  ```

- **Paddle GPU:** install a GPU build of PaddlePaddle and set
  `PADDLEOCR_VL_DEVICE=gpu:0` (Linux; the Windows GPU wheels currently ship a
  broken cuDNN dependency pin).

### Environment variables

| Variable | Effect |
|----------|--------|
| `PADDLEOCR_VL_DEVICE` | Pipeline device, e.g. `gpu:0` or `cpu` |
| `PADDLEOCR_VL_ENGINE` | `transformers` to run the VL model through PyTorch |
| `PADDLEOCR_VL_SERVER_URL` | URL of a running genai inference server (implies `vllm-server` backend) |
| `PADDLEOCR_VL_BACKEND` | Explicit `vl_rec_backend`, e.g. `sglang-server` |

## Docker (CPU)

```bash
docker build -t paddleocrvl-liteparse .
docker run --rm -p 8831:8831 \
  -v "$HOME/.cache/paddleocrvl-models:/root/.paddlex" \
  paddleocrvl-liteparse
```

For GPU serving, combine the CPU image with the external vLLM genai server
above (`PADDLEOCR_VL_SERVER_URL` works inside compose networks too).

## API

**`POST /ocr`** — multipart form with `file` (image) and optional `language`
(accepted for API compatibility; the model is multilingual and ignores it).

```bash
curl -X POST http://localhost:8831/ocr -F "file=@page.png" -F "language=en"
```

```json
{
  "results": [
    { "text": "Hello World", "bbox": [10, 20, 200, 40], "confidence": 0.98 }
  ]
}
```

Results are **layout blocks** (paragraphs, table cells flattened to plain
text, formula regions), not word-level boxes. `confidence` is the layout
detector's region score; the VL model does not expose per-token confidence.

**`GET /health`** — returns `{"status": "healthy"}`.

## Tests

```bash
uv run pytest
```

The tests mock the pipeline, so they run without downloading model weights.
