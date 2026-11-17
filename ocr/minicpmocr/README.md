# MiniCPM-OCR Service

A FastAPI server wrapping [MiniCPM-OCR 1.0](https://github.com/OpenBMB/MiniCPM-OCR)
to conform to the LiteParse OCR API specification (see `../../OCR_API_SPEC.md`).

MiniCPM-OCR is a 1.3B document-parsing model for Chinese and English. It is
block-level: this server runs the model's bundled PP-DocLayout layout detector
in-process, crops each detected block, and asks a MiniCPM-OCR inference server
to recognize the crop with the recipe for that block type (`Text Recognition:`,
`Table Recognition:`, `Formula Recognition:`, ...).

## Inference backend (required)

The adapter does not bundle the 1.3B VLM weights. Run MiniCPM-OCR with one of
its supported serving engines and point `MINICPM_OCR_BASE_URL` at the server
root. The [MiniCPM-OCR deployment guide](https://github.com/OpenBMB/MiniCPM-OCR/blob/main/docs/minicpm_ocr_1dot0_en.md)
covers vLLM, SGLang, llama.cpp, and Ollama.

```bash
# vLLM example
vllm serve openbmb/MiniCPM-OCR-1.0 \
  --served-model-name minicpm-ocr --host 127.0.0.1 --port 18499
```

## Environment

| Variable                      | Default                       | Meaning                                                             |
| ----------------------------- | ----------------------------- | ------------------------------------------------------------------- |
| `MINICPM_OCR_BASE_URL`        | required                      | Inference server root, without `/v1` (a trailing `/v1` is stripped) |
| `MINICPM_OCR_MODEL`           | `minicpm-ocr`                 | Served model name                                                   |
| `MINICPM_OCR_BACKEND`         | `vllm`                        | `vllm`, `sglang`, `openai`, `llamacpp`, or `ollama`                 |
| `MINICPM_OCR_API_KEY`         | empty                         | Optional bearer token for the inference server                      |
| `MINICPM_OCR_MAX_MODEL_LEN`   | `40960`                       | Context length reported to the SDK; keep it equal to the server's   |
| `MINICPM_OCR_STARTUP_TIMEOUT` | `60`                          | Seconds to wait for the inference server on the first request       |
| `MINICPM_OCR_REQUEST_TIMEOUT` | `300`                         | Per-request timeout in seconds                                      |
| `MINICPM_OCR_LAYOUT_DEVICE`   | `cpu`                         | Paddle device for layout detection (`cpu` or `cuda:0`)              |
| `MINICPM_OCR_LAYOUT_DIR`      | `~/.cache/minicpm-ocr/layout` | Cache for the PP-DocLayoutV3 model                                  |

## Build and Run

```bash
# install and run
uv run server.py
```

The first `POST /ocr` downloads the PP-DocLayoutV3 layout model into
`MINICPM_OCR_LAYOUT_DIR` and connects to the inference server; `GET /health`
answers before either happens.

```bash
curl -X POST http://localhost:8832/ocr \
  -F "file=@page.png" \
  -F "language=en"
```

```bash
lit parse document.pdf --ocr-server-url http://localhost:8832/ocr
```

## Docker

The image contains the API and the layout model only; the inference server
runs separately and is reached through `MINICPM_OCR_BASE_URL`.

```bash
docker build -t minicpm-ocr-liteparse .
docker run --rm -p 8832:8832 \
  -e MINICPM_OCR_BASE_URL=http://host.docker.internal:18499 \
  -v minicpm-ocr-layout:/models/layout \
  minicpm-ocr-liteparse
```

Mount `/models/layout` to keep the PP-DocLayoutV3 download across container
runs. For air-gapped deployments, pre-populate that volume before starting the
container.

## Notes

- `language` is accepted for API compatibility but unused: one MiniCPM-OCR
  model handles Chinese and English.
- `confidence` is reported as `1.0`. MiniCPM-OCR does not return a recognition
  confidence, and the layout score is not a substitute for one.
- A block whose completion ends with an explicit non-`stop` `finish_reason`
  (for example `length`) is skipped with a warning instead of returning a
  truncated prefix.
