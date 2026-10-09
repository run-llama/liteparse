# CPU image for the PaddleOCR-VL LiteParse server. For GPU serving, run the
# official paddleocr genai vLLM server container alongside and point this one
# at it with PADDLEOCR_VL_SERVER_URL (see README.md).
FROM python:3.12-slim-bookworm

COPY --from=ghcr.io/astral-sh/uv:0.11.21 /uv /usr/local/bin/uv

# Runtime libs for paddlepaddle and Pillow/OpenCV.
RUN apt-get update && apt-get install -y --no-install-recommends \
    libgl1 \
    libglib2.0-0 \
    libgomp1 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/app

COPY pyproject.toml README.md server.py test_server.py ./

RUN uv sync

# Model weights (layout detector + 0.9B VL model) download on first run;
# mount /root/.paddlex to cache them across containers.
EXPOSE 8831
CMD ["uv", "run", "python", "server.py"]
