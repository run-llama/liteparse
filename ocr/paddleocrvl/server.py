import io
import logging
import os
import re
import threading
import traceback
from html import unescape
from html.parser import HTMLParser
from typing import Any

import numpy as np
import uvicorn
from fastapi import FastAPI, HTTPException
from fastapi.datastructures import UploadFile
from fastapi.param_functions import File, Form
from PIL import Image
from pydantic import BaseModel

# PaddleOCR-VL 1.6 pairs a small layout detector (fast on CPU) with a 0.9B
# vision-language model for recognition. The defaults below run everything
# locally through PaddlePaddle on CPU — zero extra setup. Override for speed:
#
#   PADDLEOCR_VL_DEVICE      Pipeline device, e.g. "gpu:0" (needs a GPU
#                            build of PaddlePaddle) or "cpu".
#   PADDLEOCR_VL_ENGINE      "transformers" runs the VL model through PyTorch
#                            (install the `transformers` extra). This is the
#                            practical GPU path on Windows, where vLLM/SGLang
#                            cannot run natively.
#   PADDLEOCR_VL_SERVER_URL  Attach VL recognition to a running inference
#                            server (e.g. the official paddleocr genai vLLM
#                            docker image), e.g. "http://127.0.0.1:8080/v1".
#                            Implies PADDLEOCR_VL_BACKEND=vllm-server unless
#                            PADDLEOCR_VL_BACKEND is set explicitly.
#   PADDLEOCR_VL_BACKEND     Explicit vl_rec_backend for the pipeline
#                            (e.g. "vllm-server", "sglang-server").

_BLOCK_OR_BREAK = {
    "br", "p", "div", "li", "tr", "td", "th",
    "h1", "h2", "h3", "h4", "h5", "h6",
}

# Layout regions that never carry recognized text (the VL model describes or
# skips them); everything else is kept when its content is non-empty.
_NON_TEXT_LABELS = {"image", "figure"}


class _TextExtractor(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self._parts: list[str] = []

    def handle_data(self, data: str) -> None:
        self._parts.append(data)

    def handle_starttag(self, tag: str, attrs: object) -> None:
        if tag in _BLOCK_OR_BREAK:
            self._parts.append(" ")

    def handle_endtag(self, tag: str) -> None:
        if tag in _BLOCK_OR_BREAK:
            self._parts.append(" ")

    def text(self) -> str:
        return "".join(self._parts)


def _content_to_text(content: str) -> str:
    """Flatten block content (plain text, HTML tables, markdown) to one line.

    PaddleOCR-VL emits HTML for tables and markdown/LaTeX for formulas; the
    LiteParse OCR API wants plain text, so markup is stripped with stdlib only.
    """
    if not content:
        return ""
    if "<" in content and ">" in content:
        parser = _TextExtractor()
        parser.feed(content)
        parser.close()
        content = unescape(parser.text())
    # The VL model LaTeX-escapes special characters in plain text ("\%", "\$").
    content = re.sub(r"\\([%$&#_{}])", r"\1", content)
    return re.sub(r"\s+", " ", content).strip()


def _coerce_bbox(bbox: Any) -> list[int] | None:
    """Return an integer [x1, y1, x2, y2] box, or None if the shape is invalid."""
    if bbox is None:
        return None
    if hasattr(bbox, "tolist"):
        bbox = bbox.tolist()
    try:
        flat = [float(v) for v in np.asarray(bbox, dtype=float).reshape(-1)]
    except (TypeError, ValueError):
        return None
    if len(flat) != 4:
        return None
    return [int(round(v)) for v in flat]


def _coerce_polygon(polygon: Any) -> list[list[float]] | None:
    """Return a 4x2 float polygon, or None if the shape is invalid."""
    if polygon is None:
        return None
    if hasattr(polygon, "tolist"):
        polygon = polygon.tolist()
    try:
        if len(polygon) == 4 and all(len(pt) == 2 for pt in polygon):
            return [[float(pt[0]), float(pt[1])] for pt in polygon]
    except TypeError:
        pass
    return None


def _layout_entries(
    layout_det_res: Any,
) -> list[tuple[list[int], float, list[list[float]] | None]]:
    """Extract (bbox, score, polygon) triples from the layout detection result."""
    if not isinstance(layout_det_res, dict):
        return []
    entries: list[tuple[list[int], float, list[list[float]] | None]] = []
    for box in layout_det_res.get("boxes", []) or []:
        bbox = _coerce_bbox(box.get("coordinate"))
        score = box.get("score")
        if bbox is not None and score is not None:
            entries.append((bbox, float(score), _coerce_polygon(box.get("polygon_points"))))
    return entries


def _layout_match(
    bbox: list[int],
    entries: list[tuple[list[int], float, list[list[float]] | None]],
) -> tuple[float, list[list[float]] | None]:
    """(score, polygon) of the layout box matching `bbox` within a few pixels.

    parsing_res_list carries no per-block score or polygon; the layout
    detector does. The VL model does not expose recognition confidence, so
    the detector's score is the best available signal. Falls back to
    (1.0, None) when no layout box lines up.
    """
    for candidate, score, polygon in entries:
        if all(abs(a - b) <= 3 for a, b in zip(bbox, candidate)):
            return score, polygon
    return 1.0, None


def _block_to_result(
    block: Any, entries: list[tuple[list[int], float, list[list[float]] | None]]
) -> dict[str, Any] | None:
    """Map a parsing_res_list block to the LiteParse OCR shape, or None to skip."""
    get = (
        block.get if isinstance(block, dict) else lambda k, d=None: getattr(block, k, d)
    )

    label = str(get("block_label", "") or "").lower()
    if label in _NON_TEXT_LABELS:
        return None

    text = _content_to_text(str(get("block_content", "") or ""))
    if not text:
        return None

    bbox = _coerce_bbox(get("block_bbox"))
    if bbox is None:
        return None

    confidence, polygon = _layout_match(bbox, entries)
    result: dict[str, Any] = {"text": text, "bbox": bbox, "confidence": confidence}
    if polygon is not None:
        result["polygon"] = polygon
    return result


def _result_to_data(res: Any) -> dict[str, Any]:
    """Unwrap a pipeline Result into its plain-dict form."""
    data = res.json if hasattr(res, "json") else res
    if isinstance(data, dict) and isinstance(data.get("res"), dict):
        data = data["res"]
    return data if isinstance(data, dict) else {}


class OcrResponse(BaseModel):
    results: list[Any]


class StatusResponse(BaseModel):
    status: str


class PaddleOCRVLServer:
    def __init__(self) -> None:
        # Imported here so the mapping helpers above stay importable (and
        # testable) without PaddlePaddle installed.
        from paddleocr import PaddleOCRVL

        kwargs: dict[str, Any] = {"pipeline_version": "v1.6"}
        if os.environ.get("PADDLEOCR_VL_DEVICE"):
            kwargs["device"] = os.environ["PADDLEOCR_VL_DEVICE"]
        if os.environ.get("PADDLEOCR_VL_ENGINE"):
            kwargs["engine"] = os.environ["PADDLEOCR_VL_ENGINE"]
        server_url = os.environ.get("PADDLEOCR_VL_SERVER_URL")
        backend = os.environ.get("PADDLEOCR_VL_BACKEND")
        if server_url:
            kwargs["vl_rec_server_url"] = server_url
            kwargs["vl_rec_backend"] = backend or "vllm-server"
        elif backend:
            kwargs["vl_rec_backend"] = backend

        self.pipeline = PaddleOCRVL(**kwargs)
        # The pipeline is not thread-safe; serialize requests.
        self._lock = threading.Lock()

    def _create_ocr_server(self) -> FastAPI:
        app = FastAPI()

        @app.post("/ocr")
        def ocr_endpoint(
            file: UploadFile = File(...), language: str = Form(default="en")
        ) -> OcrResponse:
            # `language` is accepted for API compatibility but unused:
            # PaddleOCR-VL is multilingual (109 languages) with no
            # per-language model reload.
            try:
                image = Image.open(io.BytesIO(file.file.read()))
                if image.mode != "RGB":
                    image = image.convert("RGB")
            except Exception as e:
                raise HTTPException(status_code=400, detail=f"Invalid image: {e}")

            try:
                with self._lock:
                    predictions = list(self.pipeline.predict(np.asarray(image)))
            except Exception as e:
                logging.error("OCR failed:\n%s", traceback.format_exc())
                raise HTTPException(status_code=500, detail=str(e))

            formatted: list[dict[str, Any]] = []
            for res in predictions:
                data = _result_to_data(res)
                entries = _layout_entries(data.get("layout_det_res"))
                for block in data.get("parsing_res_list", []) or []:
                    mapped = _block_to_result(block, entries)
                    if mapped is not None:
                        formatted.append(mapped)

            return OcrResponse(results=formatted)

        @app.get("/health")
        def health() -> StatusResponse:
            return StatusResponse(status="healthy")

        return app

    def serve(self) -> None:
        app = self._create_ocr_server()
        uvicorn.run(app, host="0.0.0.0", port=8831)


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO)
    logging.info("Starting server on port 8831")
    PaddleOCRVLServer().serve()
