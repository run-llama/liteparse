import io
import logging
import os
import threading
from copy import deepcopy
from pathlib import Path
from typing import Any

import uvicorn
from fastapi import FastAPI, HTTPException
from fastapi.datastructures import UploadFile
from fastapi.param_functions import File, Form
from minicpmvocr import (
    DEFAULT_CONFIG,
    create_backend,
    create_layout_detector,
    get_element_cfg,
    load_prompt_map,
)
from minicpmvocr.layouts.download import ensure_layout_model
from PIL import Image
from pydantic import BaseModel
from starlette.concurrency import run_in_threadpool

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

DEFAULT_MODEL = "minicpm-ocr"
DEFAULT_BACKEND = "vllm"
DEFAULT_MAX_MODEL_LEN = 40960
DEFAULT_STARTUP_TIMEOUT = 60
DEFAULT_REQUEST_TIMEOUT = 300
DEFAULT_LAYOUT_DIR = "~/.cache/minicpm-ocr/layout"


class OcrResponse(BaseModel):
    results: list[Any]


class StatusResponse(BaseModel):
    status: str


def _env(name: str, default: str = "") -> str:
    return (os.getenv(name) or default).strip()


def _env_int(name: str, default: int) -> int:
    raw = _env(name, str(default))
    try:
        return int(raw)
    except ValueError:
        logger.warning("Invalid %s=%r; using %s", name, raw, default)
        return default


def _normalize_server_url(url: str) -> str:
    """The SDK expects the server root, without a trailing /v1."""
    url = (url or "").strip().rstrip("/")
    if url.endswith("/v1"):
        url = url[: -len("/v1")]
    return url


def _resize_to_min_side(image: Image.Image, target_min_side: int | None) -> Image.Image:
    if not target_min_side:
        return image
    width, height = image.size
    if min(width, height) >= target_min_side:
        return image
    scale = target_min_side / min(width, height)
    return image.resize((int(width * scale), int(height * scale)), Image.Resampling.LANCZOS)


def build_config(base_url: str) -> dict:
    cfg = deepcopy(DEFAULT_CONFIG)
    cfg["engine"].update(
        {
            "type": _env("MINICPM_OCR_BACKEND", DEFAULT_BACKEND),
            "model_name": _env("MINICPM_OCR_MODEL", DEFAULT_MODEL),
            "server_url": base_url,
            "api_key": _env("MINICPM_OCR_API_KEY"),
            "max_model_len": _env_int("MINICPM_OCR_MAX_MODEL_LEN", DEFAULT_MAX_MODEL_LEN),
            "startup_timeout_sec": _env_int("MINICPM_OCR_STARTUP_TIMEOUT", DEFAULT_STARTUP_TIMEOUT),
            "request_timeout_sec": _env_int("MINICPM_OCR_REQUEST_TIMEOUT", DEFAULT_REQUEST_TIMEOUT),
            # MiniCPM-OCR puts its output in content; leaving thinking enabled can
            # move the transcription into reasoning_content, which the SDK never reads.
            "extra_body": {"chat_template_kwargs": {"enable_thinking": False}},
        }
    )
    cfg["layout"].update(
        {
            "provider": "ppdoclayout3_paddlex",
            "model_dir": "",
            "device": _env("MINICPM_OCR_LAYOUT_DEVICE", "cpu"),
        }
    )
    return cfg


def _valid_bbox(bbox: Any) -> list[float] | None:
    if not isinstance(bbox, (list, tuple)) or len(bbox) != 4:
        return None
    try:
        x1, y1, x2, y2 = (float(value) for value in bbox)
    except (TypeError, ValueError):
        return None
    if x2 <= x1 or y2 <= y1:
        return None
    return [x1, y1, x2, y2]


def _block_to_result(block: dict, text: str) -> dict[str, Any]:
    bbox = _valid_bbox(block.get("bbox")) or [0, 0, 0, 0]
    result: dict[str, Any] = {
        "text": text,
        "bbox": [int(round(value)) for value in bbox],
        # MiniCPM-OCR returns no OCR confidence; the layout detection score is
        # not a recognition confidence, so report a neutral value.
        "confidence": 1.0,
    }
    polygon = block.get("polygon")
    if isinstance(polygon, list) and len(polygon) == 4:
        try:
            result["polygon"] = [[float(point[0]), float(point[1])] for point in polygon]
        except (TypeError, ValueError, IndexError):
            pass
    return result


class MiniCPMOCRServer:
    """LiteParse OCR server backed by MiniCPM-OCR 1.0.

    Layout detection runs in this process with the model's bundled PP-DocLayout
    provider; text recognition is delegated to a MiniCPM-OCR inference server
    (vLLM, SGLang, llama.cpp or Ollama) because the 1.3B VLM is expected to run
    on its own serving engine, as with the Surya adapter.

    The request ``language`` field is accepted for API compatibility but unused:
    MiniCPM-OCR 1.0 handles Chinese and English with one model.
    """

    def __init__(self) -> None:
        base_url = _normalize_server_url(_env("MINICPM_OCR_BASE_URL"))
        if not base_url:
            raise ValueError(
                "MINICPM_OCR_BASE_URL is required and must point at a MiniCPM-OCR "
                "server root, for example http://localhost:8000 (without /v1)."
            )

        self.cfg = build_config(base_url)
        self.prompt_map = load_prompt_map(self.cfg)
        self.layout_detector = None
        self.engine = None
        self._init_lock = threading.Lock()
        # Paddle's predictor and the SDK client are kept single-threaded; the
        # endpoint offloads this work to a thread so /health stays responsive.
        self._infer_lock = threading.Lock()

    def _ensure_models(self) -> None:
        if self.layout_detector is not None and self.engine is not None:
            return
        with self._init_lock:
            if self.layout_detector is None:
                layout_dir = Path(_env("MINICPM_OCR_LAYOUT_DIR", DEFAULT_LAYOUT_DIR)).expanduser()
                self.cfg["layout"]["model_dir"] = str(ensure_layout_model(layout_dir))
                self.layout_detector = create_layout_detector(self.cfg["layout"])
            if self.engine is None:
                engine = create_backend(self.cfg)
                engine.start()
                # Only publish the engine after it is ready: a failed start must
                # leave the attribute unset so the next request retries.
                self.engine = engine

    def _ocr(self, image: Image.Image) -> list[dict[str, Any]]:
        self._ensure_models()

        with self._infer_lock:
            detections = self.layout_detector.detect([image])[0]
            blocks = self.layout_detector.crop(image, detections)

            results: list[dict[str, Any]] = []
            pixel_array = None
            for block in blocks:
                label = block.get("label", "")
                elem_cfg = get_element_cfg(self.prompt_map, label)
                if elem_cfg is None or block.get("skip_vlm", False):
                    continue

                bbox = _valid_bbox(block.get("bbox"))
                if bbox is None:
                    logger.warning("Skipping %s block with invalid bbox: %r", label, block.get("bbox"))
                    continue
                x1, y1, x2, y2 = (int(round(value)) for value in bbox)

                crop = block.get("image")
                if crop is None:
                    # Some providers return only boxes; fall back to a page crop.
                    if x2 <= x1 or y2 <= y1:
                        continue
                    if pixel_array is None:
                        import numpy as np

                        pixel_array = np.asarray(image.convert("RGB"))
                    crop = pixel_array[y1:y2, x1:x2]
                if crop is None or getattr(crop, "size", 0) == 0:
                    continue

                crop_image = Image.fromarray(crop).convert("RGB")
                model_image = _resize_to_min_side(crop_image, elem_cfg.get("resize_min_side"))
                prediction = self.engine.recognize(
                    model_image,
                    prompt=elem_cfg["prompt"],
                    max_new_tokens=elem_cfg["max_new_tokens"],
                    slice_nums=elem_cfg["slice_nums"],
                )

                finish_reason = prediction.get("finish_reason")
                if finish_reason not in (None, "", "stop"):
                    logger.warning(
                        "Skipping truncated %s block: finish_reason=%s", label, finish_reason
                    )
                    continue

                text = (prediction.get("text") or "").strip()
                if not text:
                    continue
                results.append(_block_to_result(block, text))

        return results

    def _create_ocr_server(self) -> FastAPI:
        app = FastAPI()

        @app.post("/ocr")
        async def ocr_endpoint(
            file: UploadFile = File(...), language: str = Form(default="en")
        ) -> OcrResponse:
            try:
                image_data = await file.read()
                image = Image.open(io.BytesIO(image_data))
                if image.mode != "RGB":
                    image = image.convert("RGB")
            except Exception as e:
                raise HTTPException(status_code=400, detail=f"Invalid image: {e}")

            try:
                # Layout detection and recognition are blocking; keep the event
                # loop free so /health and other requests stay responsive.
                results = await run_in_threadpool(self._ocr, image)
            except Exception as e:
                logger.exception("MiniCPM-OCR failed")
                raise HTTPException(status_code=500, detail=str(e))

            return OcrResponse(results=results)

        @app.get("/health")
        def health() -> StatusResponse:
            return StatusResponse(status="healthy")

        return app

    def serve(self) -> None:
        app = self._create_ocr_server()
        uvicorn.run(app, host="0.0.0.0", port=8832)


if __name__ == "__main__":
    logging.info("Starting server on port 8832")
    MiniCPMOCRServer().serve()
