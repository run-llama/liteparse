import io
import queue
from types import SimpleNamespace

import numpy as np
import pytest
from fastapi.testclient import TestClient
from PIL import Image

from server import IndraOCRServer
from server import _to_input_frame


class MockOrtOCR:
    """Stands in for indra_ocr.OrtOCR: returns a fixed page for any image."""

    def __init__(self, lines: list, rotation: int = 0, error: Exception | None = None) -> None:
        self._lines = lines
        self._rotation = rotation
        self._error = error
        self.calls = 0

    def __call__(self, gray: np.ndarray):
        self.calls += 1
        if self._error is not None:
            raise self._error
        return SimpleNamespace(lines=self._lines, rotation=self._rotation)


def _make_server(engine: MockOrtOCR) -> IndraOCRServer:
    server = IndraOCRServer.__new__(IndraOCRServer)  # skip model load
    server.pool = queue.Queue()
    server.pool.put(engine)
    return server


def _png_bytes(width: int = 40, height: int = 20) -> io.BytesIO:
    image = Image.new("RGB", (width, height), color=(255, 255, 255))
    buffer = io.BytesIO()
    image.save(buffer, format="PNG")
    buffer.seek(0)
    return buffer


def _post(server: IndraOCRServer, **kwargs):
    client = TestClient(server._create_ocr_server())
    return client.post(
        "/ocr",
        files={"file": ("test.png", kwargs.pop("image", _png_bytes()), "image/png")},
        data={"language": kwargs.pop("language", "en")},
    )


def _line(text: str, box: list[float], conf: float | None = 0.9) -> SimpleNamespace:
    return SimpleNamespace(text=text, box=box, conf=conf)


def test_server_health_endpoint() -> None:
    client = TestClient(_make_server(MockOrtOCR([]))._create_ocr_server())
    response = client.get("/health")
    assert response.status_code == 200
    assert response.json() == {"status": "healthy"}


def test_server_ocr_endpoint_maps_lines() -> None:
    engine = MockOrtOCR([_line("Hello World", [10.2, 20.0, 200.7, 40.0], 0.97)])
    response = _post(_make_server(engine))
    assert response.status_code == 200
    results = response.json()["results"]
    assert len(results) == 1
    assert results[0]["text"] == "Hello World"
    assert results[0]["bbox"] == [10, 20, 201, 40]
    assert results[0]["confidence"] == pytest.approx(0.97)
    assert results[0]["polygon"] == [
        [10.2, 20.0], [200.7, 20.0], [200.7, 40.0], [10.2, 40.0]
    ]
    assert engine.calls == 1


def test_server_skips_empty_lines() -> None:
    engine = MockOrtOCR([_line("", [0, 0, 5, 5]), _line("   ", [0, 0, 5, 5]), _line("kept", [1, 1, 9, 9])])
    results = _post(_make_server(engine)).json()["results"]
    assert [r["text"] for r in results] == ["kept"]


def test_server_strips_text_and_clamps_confidence() -> None:
    engine = MockOrtOCR([
        _line("  high  ", [0, 0, 5, 5], 1.7),
        _line("low", [0, 0, 5, 5], -0.2),
        _line("missing", [0, 0, 5, 5], None),
    ])
    results = _post(_make_server(engine)).json()["results"]
    assert [r["text"] for r in results] == ["high", "low", "missing"]
    assert [r["confidence"] for r in results] == [1.0, 0.0, 1.0]


def test_server_accepts_any_language() -> None:
    engine = MockOrtOCR([_line("text", [0, 0, 5, 5])])
    response = _post(_make_server(engine), language="zh")
    assert response.status_code == 200
    assert response.json()["results"][0]["text"] == "text"


def test_server_rejects_invalid_image() -> None:
    engine = MockOrtOCR([])
    response = _post(_make_server(engine), image=io.BytesIO(b"not an image"))
    assert response.status_code == 400
    assert engine.calls == 0


def test_server_returns_500_when_ocr_fails() -> None:
    engine = MockOrtOCR([], error=RuntimeError("boom"))
    server = _make_server(engine)
    response = _post(server)
    assert response.status_code == 500
    assert "boom" in response.json()["detail"]
    # The recognizer goes back into the pool even when it raises.
    assert server.pool.qsize() == 1


@pytest.mark.parametrize("quarter_turns", [0, 1, 2, 3])
def test_to_input_frame_inverts_indra_upright_turn(quarter_turns: int) -> None:
    # Mark one pixel, turn the image the way Indra does (np.rot90(img, -k)),
    # find the mark in the upright frame, and map it back.
    height, width = 50, 100
    image = np.zeros((height, width), dtype=np.uint8)
    image[10, 70] = 255
    upright = np.rot90(image, -quarter_turns)
    uy, ux = (int(v) for v in np.argwhere(upright == 255)[0])
    assert _to_input_frame(ux, uy, quarter_turns, width, height) == (70, 10)


def test_server_maps_boxes_of_a_turned_page_back_to_the_input_image() -> None:
    # Indra read the page turned upright by one clockwise quarter turn, so its
    # upright frame is 20 wide x 40 tall for a 40 x 20 upload.
    engine = MockOrtOCR([_line("sideways", [2.0, 5.0, 12.0, 9.0])], rotation=90)
    result = _post(_make_server(engine), image=_png_bytes(40, 20)).json()["results"][0]
    # Turning clockwise brought the upload's bottom-left corner to the upright
    # top-left, so the upright TL, TR, BR, BL land bottom-left-first here.
    assert result["polygon"] == [[5.0, 17.0], [5.0, 7.0], [9.0, 7.0], [9.0, 17.0]]
    assert result["bbox"] == [5, 7, 9, 17]
