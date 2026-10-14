# Indra OCR API (CPU only — Indra runs on ONNX Runtime, no GPU or torch needed).
FROM python:3.12-slim

# Bring uv in from its official image (no external install script needed).
COPY --from=ghcr.io/astral-sh/uv:0.11.21 /uv /usr/local/bin/uv

WORKDIR /opt/app

# Copy project metadata, lockfile, and server code
COPY pyproject.toml uv.lock README.md server.py test_server.py ./

# Install dependencies (uses the committed lockfile)
RUN uv sync --frozen --no-dev

# Bake the model weights (~18 MB) into the image so the container starts
# without network access.
ENV HF_HOME=/opt/models
RUN uv run --frozen --no-dev python -c "from indra_ocr import model_dir; print(model_dir())"

EXPOSE 8831
CMD ["uv", "run", "--frozen", "--no-dev", "python", "server.py"]
