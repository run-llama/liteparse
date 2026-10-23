"""Exercise the built extension and real Vision, including worker processes."""

import subprocess
import sys
from pathlib import Path

import pytest
from liteparse import LiteParse, ParseError

FIXTURE = Path(__file__).resolve().parents[3] / "integration_tests_data/macos_ocr.png"
MACOS = pytest.mark.skipif(
    sys.platform != "darwin", reason="Apple Vision requires macOS"
)


@MACOS
@pytest.mark.parametrize("pool_size", [None, 2])
def test_real_vision_recognition_and_config(pool_size):
    with LiteParse(
        ocr_enabled=True,
        ocr_backend="macos-vision",
        mac_ocr_languages=["en-US"],
        mac_ocr_recognition="accurate",
        emit_word_boxes=True,
        output_format="text",
        pool_size=pool_size,
        quiet=True,
    ) as parser:
        config = parser.get_config()
        assert config.ocr_backend == "macos-vision"
        assert config.mac_ocr_languages == ["en-US"]
        assert config.mac_ocr_recognition == "accurate"
        result = parser.parse(FIXTURE)
        assert "Invoice 12345" in result.text
        words = [word for item in result.pages[0].text_items for word in item.words]
        invoice = next(word for word in words if word.text == "Invoice")
        assert abs(invoice.x - 38.4) < 10
        assert abs(invoice.y - 139.2) < 15


@MACOS
def test_python_cli_selects_vision():
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "liteparse.cli",
            "parse",
            str(FIXTURE),
            "--ocr-backend",
            "macos-vision",
            "--mac-ocr-language",
            "en-US",
            "--mac-ocr-recognition",
            "fast",
            "--format",
            "text",
            "--quiet",
        ],
        capture_output=True,
        text=True,
        check=True,
        timeout=60,
    )
    assert "Invoice 12345" in result.stdout


def test_invalid_backend_and_level():
    with pytest.raises(ValueError, match="invalid OCR backend"):
        LiteParse(ocr_backend="invalid")
    with pytest.raises(ValueError, match="invalid Mac OCR recognition"):
        LiteParse(mac_ocr_recognition="invalid")


@pytest.mark.skipif(sys.platform == "darwin", reason="requires a non-macOS target")
def test_vision_platform_requirement():
    with pytest.raises(ParseError, match="requires macOS"):
        LiteParse(ocr_enabled=True, ocr_backend="macos-vision").parse(FIXTURE)
