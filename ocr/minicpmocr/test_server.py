import io
import threading

import numpy as np
import pytest
from fastapi.testclient import TestClient
from minicpmvocr.prompts import DEFAULT_PROMPT_MAP
from PIL import Image

import server as server_module
from server import MiniCPMOCRServer, _normalize_server_url, build_config


class MockLayoutDetector:
    def __init__(self, blocks: list[dict]) -> None:
        self._blocks = blocks
        self.detect_calls = 0
        self.crop_calls = 0

    def detect(self, images):
        self.detect_calls += 1
        return [[] for _ in images]

    def crop(self, image, detections):
        self.crop_calls += 1
        return self._blocks


class MockEngine:
    def __init__(self, texts: list[str], finish_reason: str = "stop") -> None:
        self._texts = list(texts)
        self.finish_reason = finish_reason
        self.calls: list[dict] = []

    def recognize(self, image, prompt: str, max_new_tokens: int, slice_nums=None):
        self.calls.append(
            {
                "image": image,
                "prompt": prompt,
                "max_new_tokens": max_new_tokens,
                "slice_nums": slice_nums,
            }
        )
        text = self._texts.pop(0) if self._texts else ""
        return {"text": text, "finish_reason": self.finish_reason}


def _block(
    label: str = "text",
    bbox: list[int] | None = None,
    image: np.ndarray | None = None,
    polygon: list[list[float]] | None = None,
    skip_vlm: bool = False,
) -> dict:
    return {
        "label": label,
        "bbox": bbox or [0, 0, 10, 10],
        "score": 0.9,
        "image": image if image is not None else np.full((10, 10, 3), 255, dtype=np.uint8),
        "polygon": polygon or [],
        "skip_vlm": skip_vlm,
    }


def _make_server(blocks: list[dict], engine: MockEngine | None = None) -> MiniCPMOCRServer:
    server = MiniCPMOCRServer.__new__(MiniCPMOCRServer)
    server.cfg = {}
    server.prompt_map = DEFAULT_PROMPT_MAP
    server.layout_detector = MockLayoutDetector(blocks)
    server.engine = engine or MockEngine(["recognized text"])
    server._init_lock = threading.Lock()
    server._infer_lock = threading.Lock()
    return server


def _png_bytes(size: tuple[int, int] = (4, 4)) -> io.BytesIO:
    image = Image.new("RGB", size, color=(255, 255, 255))
    buffer = io.BytesIO()
    image.save(buffer, format="PNG")
    buffer.seek(0)
    return buffer


def test_requires_base_url(monkeypatch) -> None:
    monkeypatch.delenv("MINICPM_OCR_BASE_URL", raising=False)

    with pytest.raises(ValueError, match="MINICPM_OCR_BASE_URL"):
        MiniCPMOCRServer()


def test_engine_start_failure_is_retried(monkeypatch) -> None:
    server = MiniCPMOCRServer.__new__(MiniCPMOCRServer)
    server.cfg = {"layout": {}, "engine": {}}
    server.prompt_map = {}
    server.layout_detector = None
    server.engine = None
    server._init_lock = threading.Lock()
    server._infer_lock = threading.Lock()

    class Detector:
        pass

    calls = {"start": 0}

    class Engine:
        def start(self) -> None:
            calls["start"] += 1
            if calls["start"] == 1:
                raise RuntimeError("inference server down")

    monkeypatch.setattr(server_module, "ensure_layout_model", lambda directory: directory)
    monkeypatch.setattr(server_module, "create_layout_detector", lambda cfg: Detector())
    monkeypatch.setattr(server_module, "create_backend", lambda cfg: Engine())

    with pytest.raises(RuntimeError, match="inference server down"):
        server._ensure_models()
    assert server.engine is None

    server._ensure_models()
    assert server.engine is not None
    assert calls["start"] == 2


def test_normalize_server_url_strips_v1_and_slash() -> None:
    assert _normalize_server_url("http://localhost:8000/v1/") == "http://localhost:8000"
    assert _normalize_server_url("http://localhost:8000/v1") == "http://localhost:8000"
    assert _normalize_server_url("http://localhost:8000/") == "http://localhost:8000"


def test_build_config_reads_env(monkeypatch) -> None:
    monkeypatch.setenv("MINICPM_OCR_BACKEND", "openai")
    monkeypatch.setenv("MINICPM_OCR_MODEL", "custom-model")
    monkeypatch.setenv("MINICPM_OCR_API_KEY", "secret")
    monkeypatch.setenv("MINICPM_OCR_MAX_MODEL_LEN", "8192")

    cfg = build_config("http://localhost:8000")

    assert cfg["engine"]["type"] == "openai"
    assert cfg["engine"]["model_name"] == "custom-model"
    assert cfg["engine"]["api_key"] == "secret"
    assert cfg["engine"]["max_model_len"] == 8192
    assert cfg["engine"]["server_url"] == "http://localhost:8000"
    assert cfg["engine"]["extra_body"] == {"chat_template_kwargs": {"enable_thinking": False}}
    assert cfg["layout"]["provider"] == "ppdoclayout3_paddlex"


def test_health_endpoint() -> None:
    client = TestClient(_make_server([])._create_ocr_server())
    response = client.get("/health")
    assert response.status_code == 200
    assert response.json() == {"status": "healthy"}


def test_ocr_maps_blocks_to_liteparse_shape() -> None:
    block = _block(
        bbox=[1.4, 2.6, 10.2, 20.8],
        image=np.full((18, 9, 3), 255, dtype=np.uint8),
        polygon=[[1, 2], [10, 2], [10, 20], [1, 20]],
    )
    engine = MockEngine(["Hello"])
    client = TestClient(_make_server([block], engine)._create_ocr_server())

    response = client.post("/ocr", files={"file": ("page.png", _png_bytes(), "image/png")})

    assert response.status_code == 200
    assert response.json() == {
        "results": [
            {
                "text": "Hello",
                "bbox": [1, 3, 10, 21],
                "confidence": 1.0,
                "polygon": [[1.0, 2.0], [10.0, 2.0], [10.0, 20.0], [1.0, 20.0]],
            }
        ]
    }
    assert engine.calls[0]["prompt"] == "Text Recognition:"
    assert engine.calls[0]["max_new_tokens"] == 4096


def test_ocr_uses_table_prompt_for_table_blocks() -> None:
    engine = MockEngine(["<table></table>"])
    client = TestClient(_make_server([_block(label="table")], engine)._create_ocr_server())

    client.post("/ocr", files={"file": ("page.png", _png_bytes(), "image/png")})

    assert engine.calls[0]["prompt"] == "Table Recognition:"


def test_ocr_skips_blocks_marked_skip_vlm() -> None:
    engine = MockEngine(["kept"])
    blocks = [_block(skip_vlm=True), _block(label="text")]
    client = TestClient(_make_server(blocks, engine)._create_ocr_server())

    response = client.post("/ocr", files={"file": ("page.png", _png_bytes(), "image/png")})

    assert response.status_code == 200
    assert [item["text"] for item in response.json()["results"]] == ["kept"]
    assert len(engine.calls) == 1


def test_unknown_label_uses_default_prompt() -> None:
    engine = MockEngine(["fallback"])
    client = TestClient(_make_server([_block(label="unknown_label")], engine)._create_ocr_server())

    response = client.post("/ocr", files={"file": ("page.png", _png_bytes(), "image/png")})

    assert response.status_code == 200
    assert response.json()["results"][0]["text"] == "fallback"
    assert engine.calls[0]["prompt"] == "Text Recognition:"


def test_ocr_skips_blocks_with_invalid_bbox() -> None:
    engine = MockEngine(["never used"])
    blocks = [_block(bbox=[10, 10, 5, 5]), _block(bbox=[0, 0, 10])]
    client = TestClient(_make_server(blocks, engine)._create_ocr_server())

    response = client.post("/ocr", files={"file": ("page.png", _png_bytes(), "image/png")})

    assert response.status_code == 200
    assert response.json() == {"results": []}
    assert engine.calls == []


def test_ocr_skips_empty_and_truncated_blocks(caplog) -> None:
    engine = MockEngine(["", "partial text"], finish_reason="length")
    client = TestClient(_make_server([_block(), _block()], engine)._create_ocr_server())

    response = client.post("/ocr", files={"file": ("page.png", _png_bytes(), "image/png")})

    assert response.status_code == 200
    assert response.json() == {"results": []}
    assert "truncated" in caplog.text


def test_ocr_crops_page_when_block_has_no_image() -> None:
    block = _block(image=None, bbox=[0, 0, 2, 2])
    block["image"] = None
    engine = MockEngine(["from page crop"])
    client = TestClient(_make_server([block], engine)._create_ocr_server())

    response = client.post("/ocr", files={"file": ("page.png", _png_bytes(), "image/png")})

    assert response.status_code == 200
    assert response.json()["results"][0]["text"] == "from page crop"


def test_ocr_rejects_invalid_image() -> None:
    client = TestClient(_make_server([])._create_ocr_server())

    response = client.post("/ocr", files={"file": ("page.png", b"not an image", "image/png")})

    assert response.status_code == 400
    assert "Invalid image" in response.json()["detail"]


def test_ocr_returns_500_when_backend_fails() -> None:
    class FailingEngine(MockEngine):
        def recognize(self, *args, **kwargs):
            raise RuntimeError("backend down")

    client = TestClient(_make_server([_block()], FailingEngine([]))._create_ocr_server())

    response = client.post("/ocr", files={"file": ("page.png", _png_bytes(), "image/png")})

    assert response.status_code == 500
    assert "backend down" in response.json()["detail"]


def test_ocr_accepts_language_but_does_not_use_it() -> None:
    engine = MockEngine(["text"])
    client = TestClient(_make_server([_block()], engine)._create_ocr_server())

    response = client.post(
        "/ocr",
        files={"file": ("page.png", _png_bytes(), "image/png")},
        data={"language": "zh"},
    )

    assert response.status_code == 200
    assert len(engine.calls) == 1
