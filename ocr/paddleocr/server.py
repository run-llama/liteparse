import io
import logging
from typing import Any

import numpy as np
import uvicorn
from fastapi import FastAPI, HTTPException
from fastapi.datastructures import UploadFile
from fastapi.param_functions import File, Form
from paddleocr import PaddleOCR
from PIL import Image
from pydantic import BaseModel


class OcrResponse(BaseModel):
    results: list[Any]


class StatusResponse(BaseModel):
    status: str


class PaddleOCRServer:
    def __init__(self) -> None:
        self.ocr: PaddleOCR = PaddleOCR(
            lang="en",
            use_doc_orientation_classify=False,
            use_doc_unwarping=False,
            use_textline_orientation=True,
        )
        self.current_language: str = "en"

    @staticmethod
    def normalize_language(language: str) -> str:
        normalized = language.lower()
        aliases = {
            "zh": "ch",
            "zh-cn": "ch",
            "zh-hans": "ch",
            "zh-tw": "chinese_cht",
            "zh-hant": "chinese_cht",
            "ja": "japan",
            "ko": "korean",
        }
        return aliases.get(normalized, normalized)

    def _create_ocr_server(
        self,
    ) -> FastAPI:
        app = FastAPI()

        @app.post("/ocr")
        async def ocr_endpoint(
            file: UploadFile = File(...), language: str = Form(default="en")
        ) -> OcrResponse:
            # Get language from request
            language = self.normalize_language(language)

            try:
                # Initialize OCR if needed or language changed
                if self.current_language != language:
                    # PaddleOCR 3.x parameters
                    self.ocr = PaddleOCR(
                        lang=language,
                        use_doc_orientation_classify=False,
                        use_doc_unwarping=False,
                        use_textline_orientation=True,
                    )
                    self.current_language = language

                # Load image

                image_data = await file.read()
                image = Image.open(io.BytesIO(image_data))

                # Convert to numpy array (RGB)
                if image.mode != "RGB":
                    image = image.convert("RGB")
                image_array = np.array(image)

                # Run OCR
                # PaddleOCR 3.x returns: list of result dicts
                # Each result has: res['rec_texts'], res['rec_scores'], res['rec_boxes']
                results = self.ocr.predict(image_array)
            except ValueError as ve:
                if "No models are available for the language" in str(ve):
                    raise HTTPException(status_code=400, detail=str(ve))
                raise HTTPException(status_code=500, detail=str(ve))
            except Exception as e:
                raise HTTPException(status_code=500, detail=str(e))

            # Format results according to LiteParse OCR API spec
            # Convert to: { text, bbox: [x1, y1, x2, y2], confidence }
            formatted = []

            if results and len(results) > 0:
                # Get the first result
                result = results[0]

                res_data = (
                    result.get("res", result) if isinstance(result, dict) else result
                )
                # Extract texts, scores, and boxes from the result
                if isinstance(res_data, dict):
                    texts = res_data.get("rec_texts", [])
                    scores = res_data.get("rec_scores", [])
                    boxes = res_data.get("rec_boxes", [])
                else:
                    # Fallback for result object with attributes
                    texts = getattr(res_data, "rec_texts", []) or []
                    scores = getattr(res_data, "rec_scores", []) or []
                    boxes = getattr(res_data, "rec_boxes", []) or []

                # Convert numpy arrays to lists if needed
                if hasattr(texts, "tolist"):
                    texts = texts.tolist()
                if hasattr(scores, "tolist"):
                    scores = scores.tolist()
                if hasattr(boxes, "tolist"):
                    boxes = boxes.tolist()

                # Combine them - they should be parallel arrays
                for i in range(len(texts)):
                    text = texts[i]
                    confidence = float(scores[i]) if i < len(scores) else 0.0

                    # Get bounding box coordinates
                    # rec_boxes format is typically [x_min, y_min, x_max, y_max]
                    if i < len(boxes):
                        box = boxes[i]
                        # Convert to list and ensure 4 coordinates
                        if hasattr(box, "tolist"):
                            bbox = box.tolist()
                        else:
                            bbox = list(box)
                    else:
                        bbox = [0, 0, 0, 0]

                    formatted.append(
                        {"text": text, "bbox": bbox, "confidence": confidence}
                    )

            return OcrResponse(results=formatted)

        @app.get("/health")
        def health() -> StatusResponse:
            return StatusResponse(status="healthy")

        return app

    def serve(self) -> None:
        app = self._create_ocr_server()
        uvicorn.run(app, host="0.0.0.0", port=8829)


if __name__ == "__main__":
    logging.basicConfig(
        level=logging.DEBUG,
    )
    logging.info("Starting server on port 8829")
    server = PaddleOCRServer()
    server.serve()
