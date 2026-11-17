# CPU image for the LiteParse OCR API. The MiniCPM-OCR inference server runs
# separately (vLLM, SGLang, llama.cpp, or Ollama) and is reached through
# MINICPM_OCR_BASE_URL; this image only adds the PP-DocLayout layout detector.
FROM python:3.12-slim

COPY --from=ghcr.io/astral-sh/uv:0.11.21 /uv /usr/local/bin/uv

RUN apt-get update && apt-get install -y --no-install-recommends \
    libgl1 \
    libglib2.0-0 \
    libgomp1 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/app

COPY pyproject.toml uv.lock README.md server.py ./

RUN uv sync --frozen --no-dev

ENV MINICPM_OCR_LAYOUT_DIR=/models/layout

EXPOSE 8832
CMD ["/opt/app/.venv/bin/python", "server.py"]
