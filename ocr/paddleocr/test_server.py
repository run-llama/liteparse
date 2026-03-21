import io
from typing import Any

import pytest
from fastapi.testclient import TestClient
from paddleocr import PaddleOCR
from PIL import Image

import server as paddle_server_module
from server import PaddleOCRServer


@pytest.fixture(scope="module")
def server() -> PaddleOCRServer:
    return PaddleOCRServer()


class MockPaddleOcr:
    def __init__(self, *args, **kwargs) -> None:
        self.results = [
            {
                "res": {
                    "rec_texts": ["Hello World", "Total: $42.00", "Thank you!"],
                    "rec_scores": [0.98, 0.95, 0.87],
                    "rec_boxes": [
                        [10, 20, 200, 40],
                        [10, 50, 250, 70],
                        [10, 80, 180, 100],
                    ],
                }
            }
        ]
        self.transformed_results = [
            {"text": "Hello World", "bbox": [10, 20, 200, 40], "confidence": 0.98},
            {"text": "Total: $42.00", "bbox": [10, 50, 250, 70], "confidence": 0.95},
            {"text": "Thank you!", "bbox": [10, 80, 180, 100], "confidence": 0.87},
        ]

    def predict(self, *args, **kwargs) -> list[Any]:
        return self.results


def test_server_init(server: PaddleOCRServer) -> None:
    assert server.current_language == "en"
    assert isinstance(server.ocr, PaddleOCR)


def test_server_health_endpoint(server: PaddleOCRServer) -> None:
    app = server._create_ocr_server()
    client = TestClient(app)
    response = client.get("/health")
    assert response.status_code == 200
    assert response.json() == {"status": "healthy"}


def test_server_ocr_endpoint(server: PaddleOCRServer) -> None:
    image = Image.new("RGB", (1, 1), color=(255, 255, 255))

    # Save to bytes (to simulate a file upload)
    buffer = io.BytesIO()
    image.save(buffer, format="PNG")
    buffer.seek(0)
    app = server._create_ocr_server()
    mock_ocr = MockPaddleOcr()
    server.ocr = mock_ocr  # type: ignore
    client = TestClient(app)

    response = client.post(
        "/ocr",
        files={"file": ("test.png", buffer, "image/png")},
        data={"language": "en"},
    )
    assert response.status_code == 200
    assert response.json().get("results", []) == mock_ocr.transformed_results


def test_server_normalizes_documented_language_aliases(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    image = Image.new("RGB", (1, 1), color=(255, 255, 255))
    buffer = io.BytesIO()
    image.save(buffer, format="PNG")
    buffer.seek(0)

    captured_langs: list[str] = []

    class CapturingPaddleOcr(MockPaddleOcr):
        def __init__(self, *args, **kwargs) -> None:
            captured_langs.append(kwargs.get("lang", ""))
            super().__init__(*args, **kwargs)

    monkeypatch.setattr(paddle_server_module, "PaddleOCR", CapturingPaddleOcr)

    server = PaddleOCRServer()
    app = server._create_ocr_server()
    client = TestClient(app)

    response = client.post(
        "/ocr",
        files={"file": ("test.png", buffer, "image/png")},
        data={"language": "zh"},
    )

    assert response.status_code == 200
    assert captured_langs == ["en", "ch"]
    assert server.current_language == "ch"
