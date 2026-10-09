import io
import threading

import pytest
from fastapi.testclient import TestClient
from PIL import Image

from server import PaddleOCRVLServer
from server import _content_to_text
from server import _layout_match


def test_content_to_text_passes_plain_text_through() -> None:
    assert _content_to_text("Hello World") == "Hello World"
    assert _content_to_text("  spaced   out  ") == "spaced out"


def test_content_to_text_flattens_html_tables() -> None:
    html = "<table><tr><td>Q1</td><td>100</td></tr><tr><td>Q2</td><td>200</td></tr></table>"
    assert _content_to_text(html) == "Q1 100 Q2 200"
    assert _content_to_text("<p>Total: &amp; $42</p>") == "Total: & $42"


def test_content_to_text_unescapes_latex_specials() -> None:
    assert _content_to_text(r"Symbols: ! @ # $\% ^ \& * ()") == "Symbols: ! @ # $% ^ & * ()"
    assert _content_to_text(r"50\% off") == "50% off"


def test_content_to_text_empty_for_markup_only() -> None:
    assert _content_to_text("") == ""
    assert _content_to_text("<br>") == ""
    assert _content_to_text("   ") == ""


def test_layout_match_within_tolerance() -> None:
    poly = [[10.0, 20.0], [200.0, 20.0], [200.0, 40.0], [10.0, 40.0]]
    entries = [([10, 20, 200, 40], 0.97, poly), ([0, 0, 5, 5], 0.5, None)]
    score, polygon = _layout_match([12, 20, 199, 41], entries)
    assert score == pytest.approx(0.97)
    assert polygon == poly
    assert _layout_match([500, 500, 600, 600], entries) == (1.0, None)


class MockPipeline:
    def __init__(self, data: dict) -> None:
        self._data = data

    def predict(self, image, *args, **kwargs) -> list:
        return [{"res": self._data}]


def _make_server(data: dict) -> PaddleOCRVLServer:
    server = PaddleOCRVLServer.__new__(PaddleOCRVLServer)  # skip model load
    server.pipeline = MockPipeline(data)  # type: ignore
    server._lock = threading.Lock()
    return server


def _png_bytes() -> io.BytesIO:
    image = Image.new("RGB", (4, 4), color=(255, 255, 255))
    buffer = io.BytesIO()
    image.save(buffer, format="PNG")
    buffer.seek(0)
    return buffer


def test_server_health_endpoint() -> None:
    server = _make_server({})
    client = TestClient(server._create_ocr_server())
    response = client.get("/health")
    assert response.status_code == 200
    assert response.json() == {"status": "healthy"}


def test_server_ocr_endpoint_maps_blocks() -> None:
    data = {
        "layout_det_res": {
            "boxes": [
                {
                    "label": "text",
                    "score": 0.98,
                    "coordinate": [10.2, 20.0, 200.4, 40.1],
                    "polygon_points": [[10.0, 20.0], [200.0, 20.0], [200.0, 40.0], [10.0, 40.0]],
                },
            ]
        },
        "parsing_res_list": [
            {
                "block_label": "text",
                "block_content": "Hello World",
                "block_bbox": [10.2, 20.0, 200.4, 40.1],
            },
        ],
    }
    server = _make_server(data)
    client = TestClient(server._create_ocr_server())
    response = client.post(
        "/ocr",
        files={"file": ("test.png", _png_bytes(), "image/png")},
        data={"language": "en"},
    )
    assert response.status_code == 200
    results = response.json()["results"]
    assert len(results) == 1
    assert results[0]["text"] == "Hello World"
    assert results[0]["bbox"] == [10, 20, 200, 40]
    assert results[0]["confidence"] == pytest.approx(0.98)
    assert results[0]["polygon"] == [
        [10.0, 20.0], [200.0, 20.0], [200.0, 40.0], [10.0, 40.0]
    ]


def test_server_skips_image_blocks_and_empty_content() -> None:
    data = {
        "parsing_res_list": [
            {"block_label": "image", "block_content": "a chart", "block_bbox": [0, 0, 5, 5]},
            {"block_label": "text", "block_content": "", "block_bbox": [0, 0, 5, 5]},
            {"block_label": "text", "block_content": "keep", "block_bbox": [1, 1, 9, 9]},
        ],
    }
    server = _make_server(data)
    client = TestClient(server._create_ocr_server())
    response = client.post(
        "/ocr", files={"file": ("t.png", _png_bytes(), "image/png")}
    )
    results = response.json()["results"]
    assert len(results) == 1
    assert results[0]["text"] == "keep"


def test_server_defaults_confidence_without_layout_match() -> None:
    data = {
        "parsing_res_list": [
            {"block_label": "text", "block_content": "x", "block_bbox": [0, 0, 5, 5]},
        ],
    }
    server = _make_server(data)
    client = TestClient(server._create_ocr_server())
    response = client.post(
        "/ocr", files={"file": ("t.png", _png_bytes(), "image/png")}
    )
    assert response.json()["results"][0]["confidence"] == pytest.approx(1.0)


def test_server_flattens_table_blocks() -> None:
    data = {
        "parsing_res_list": [
            {
                "block_label": "table",
                "block_content": "<table><tr><td>A</td><td>1</td></tr></table>",
                "block_bbox": [5, 5, 100, 50],
            },
        ],
    }
    server = _make_server(data)
    client = TestClient(server._create_ocr_server())
    response = client.post(
        "/ocr", files={"file": ("t.png", _png_bytes(), "image/png")}
    )
    assert response.json()["results"][0]["text"] == "A 1"


def test_server_skips_blocks_with_invalid_bbox() -> None:
    data = {
        "parsing_res_list": [
            {"block_label": "text", "block_content": "no box", "block_bbox": None},
            {"block_label": "text", "block_content": "bad box", "block_bbox": [1, 2]},
        ],
    }
    server = _make_server(data)
    client = TestClient(server._create_ocr_server())
    response = client.post(
        "/ocr", files={"file": ("t.png", _png_bytes(), "image/png")}
    )
    assert response.json()["results"] == []


def test_server_rejects_invalid_image() -> None:
    server = _make_server({})
    client = TestClient(server._create_ocr_server())
    response = client.post(
        "/ocr", files={"file": ("t.png", io.BytesIO(b"not an image"), "image/png")}
    )
    assert response.status_code == 400
