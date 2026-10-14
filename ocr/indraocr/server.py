import io
import logging
import os
import queue
import traceback
from typing import Any

import numpy as np
import uvicorn
from fastapi import FastAPI, HTTPException
from fastapi.datastructures import UploadFile
from fastapi.param_functions import File, Form
from PIL import Image
from pydantic import BaseModel

PORT = 8831


class OcrResponse(BaseModel):
    results: list[Any]


class StatusResponse(BaseModel):
    status: str


def _to_input_frame(
    x: float, y: float, quarter_turns: int, width: int, height: int
) -> tuple[float, float]:
    """Map a point from Indra's upright frame back onto the image we were sent.

    Indra turns a sideways page upright before reading it (``np.rot90(img, -k)``,
    i.e. ``k`` clockwise quarter turns) and reports line boxes in that upright
    frame. LiteParse expects coordinates in the image it uploaded, so undo the
    turn. ``width``/``height`` are the dimensions of the uploaded image.
    Small-angle deskew is already undone by Indra itself.
    """
    turns = (4 - quarter_turns % 4) % 4  # the same turn, as counter-clockwise steps
    if turns == 0:
        return x, y
    if turns == 1:
        return width - 1 - y, x
    if turns == 2:
        return width - 1 - x, height - 1 - y
    return y, height - 1 - x


def _line_to_result(
    line: Any, quarter_turns: int, width: int, height: int
) -> dict[str, Any] | None:
    """Map an Indra line to the LiteParse OCR shape, or None to skip it."""
    text = (getattr(line, "text", "") or "").strip()
    if not text:
        return None

    x0, y0, x1, y1 = (float(v) for v in line.box)
    # Corners in the upright reading frame (TL, TR, BR, BL), as the spec asks,
    # expressed in the coordinates of the uploaded image.
    polygon = [
        _to_input_frame(px, py, quarter_turns, width, height)
        for px, py in ((x0, y0), (x1, y0), (x1, y1), (x0, y1))
    ]
    xs = [p[0] for p in polygon]
    ys = [p[1] for p in polygon]
    bbox = [int(round(min(xs))), int(round(min(ys))), int(round(max(xs))), int(round(max(ys)))]

    confidence = getattr(line, "conf", None)
    confidence = 1.0 if confidence is None else min(1.0, max(0.0, float(confidence)))

    return {
        "text": text,
        "bbox": bbox,
        "confidence": confidence,
        "polygon": [[float(px), float(py)] for px, py in polygon],
    }


class IndraOCRServer:
    def __init__(self, workers: int | None = None, threads: int | None = None) -> None:
        # Imported here so the tests can build the app without the models.
        from indra_ocr import OrtOCR, model_dir, usable_cores

        # LiteParse sends one request per page and runs several pages at once
        # (`num_workers`), so keep a small pool of recognizers rather than one
        # shared instance behind a lock. Each instance owns its ONNX Runtime
        # sessions; `threads` is the intra-op thread count of each.
        workers = workers or int(os.environ.get("INDRA_WORKERS", 0)) or min(4, usable_cores())
        threads = threads or int(os.environ.get("INDRA_THREADS", 1))
        models = model_dir()  # downloads the weights (~18 MB) on first run
        self.pool: "queue.Queue[Any]" = queue.Queue()
        for _ in range(workers):
            self.pool.put(OrtOCR(models, threads=threads))
        logging.info("Indra OCR ready: %d workers x %d threads", workers, threads)

    def _recognize(self, gray: np.ndarray) -> list[dict[str, Any]]:
        engine = self.pool.get()
        try:
            page = engine(gray)
        finally:
            self.pool.put(engine)

        height, width = gray.shape[:2]
        quarter_turns = (int(getattr(page, "rotation", 0) or 0) // 90) % 4
        results = []
        for line in page.lines:
            mapped = _line_to_result(line, quarter_turns, width, height)
            if mapped is not None:
                results.append(mapped)
        return results

    def _create_ocr_server(self) -> FastAPI:
        app = FastAPI()

        @app.post("/ocr")
        def ocr_endpoint(
            file: UploadFile = File(...), language: str = Form(default="en")
        ) -> OcrResponse:
            # `language` is accepted for API compatibility but unused: Indra
            # reads every script it supports with a single model.
            try:
                image = Image.open(io.BytesIO(file.file.read()))
                gray = np.asarray(image.convert("L"))
            except Exception as e:
                raise HTTPException(status_code=400, detail=f"Invalid image: {e}")

            try:
                return OcrResponse(results=self._recognize(gray))
            except Exception as e:
                logging.error("OCR failed:\n%s", traceback.format_exc())
                raise HTTPException(status_code=500, detail=str(e))

        @app.get("/health")
        def health() -> StatusResponse:
            return StatusResponse(status="healthy")

        return app

    def serve(self) -> None:
        app = self._create_ocr_server()
        uvicorn.run(app, host="0.0.0.0", port=PORT)


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO)
    logging.info("Starting server on port %d", PORT)
    IndraOCRServer().serve()
