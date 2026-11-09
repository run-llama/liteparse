"""Exercise image reuse through the Python wrapper and native extension."""

from collections.abc import Iterator
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import struct
from threading import Thread

import pytest

from liteparse import LiteParse


def blank_pdf() -> bytes:
    objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
        *["<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 144] >>"] * 3,
    ]
    pdf = b"%PDF-1.7\n"
    offsets = []
    for index, obj in enumerate(objects, 1):
        offsets.append(len(pdf))
        pdf += f"{index} 0 obj\n{obj}\nendobj\n".encode("ascii")
    xref = len(pdf)
    pdf += b"xref\n0 6\n0000000000 65535 f \n"
    for offset in offsets:
        pdf += f"{offset:010} 00000 n \n".encode("ascii")
    return pdf + f"trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF".encode("ascii")


class ImageSizeOcr(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        body = self.rfile.read(int(self.headers["Content-Length"]))
        png = body.index(b"\x89PNG\r\n\x1a\n")
        width = struct.unpack_from(">I", body, png + 16)[0]
        scale = width / 72
        payload = json.dumps({"results": [{
            "text": f"IMAGE{width}",
            "bbox": [value * scale for value in (40, 20, 60, 30)],
            "confidence": 0.93,
        }]}).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


@pytest.fixture
def ocr_server() -> Iterator[str]:
    server = ThreadingHTTPServer(("127.0.0.1", 0), ImageSizeOcr)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def test_page_images_survive_python_options_and_batch_page_numbering(
    tmp_path: Path, ocr_server: str,
) -> None:
    pdf = tmp_path / "document.pdf"
    pdf.write_bytes(blank_pdf())
    screenshot = LiteParse(ocr_enabled=False, dpi=144, quiet=True).screenshot(
        pdf, page_numbers=[3],
    )[0]
    image = tmp_path / "page-three.png"
    image.write_bytes(screenshot.image_bytes)
    images = {3: (str(image), 144.0)}
    parser = LiteParse(
        ocr_enabled=True, ocr_server_url=ocr_server, ocr_page_images=images,
        target_pages="3", dpi=0, quiet=True, emit_word_boxes=True,
    )
    result = parser.parse(pdf)
    assert result.pages[0].page_num == 3
    item = next(item for item in result.pages[0].text_items if item.text == "IMAGE144")
    assert item.x == 40
    assert item.words[0].width == 20
    parser = LiteParse(
        ocr_enabled=True, ocr_server_url=ocr_server, ocr_page_images=images,
        dpi=72, quiet=True,
    )
    assert [batch.result.text.strip() for batch in parser.parse_batches(pdf, batch_size=1)] == [
        "IMAGE72", "IMAGE72", "IMAGE144",
    ]
