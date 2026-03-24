"""E2E tests for LiteParse.screenshot() — validates Python types match CLI output."""

import tempfile
from pathlib import Path

import pytest

from liteparse import (
    ImageFormat,
    LiteParse,
    ScreenshotBatchResult,
    ScreenshotResult,
)


class TestScreenshotBasic:
    """Basic screenshot functionality."""

    def test_screenshot_returns_batch_result(
        self, parser: LiteParse, invoice_pdf: Path
    ):
        result = parser.screenshot(invoice_pdf)
        assert isinstance(result, ScreenshotBatchResult)

    def test_screenshot_has_screenshots(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.screenshot(invoice_pdf)
        assert len(result) > 0
        assert len(result.screenshots) > 0

    def test_screenshot_result_fields(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.screenshot(invoice_pdf)
        ss = result.screenshots[0]
        assert isinstance(ss, ScreenshotResult)
        assert isinstance(ss.page_num, int)
        assert ss.page_num >= 1
        assert isinstance(ss.image_path, str)
        assert Path(ss.image_path).exists()

    def test_screenshot_output_dir(self, parser: LiteParse, invoice_pdf: Path):
        with tempfile.TemporaryDirectory() as tmpdir:
            result = parser.screenshot(invoice_pdf, output_dir=tmpdir)
            assert result.output_dir == tmpdir
            # Files should be in the specified directory
            for ss in result.screenshots:
                assert ss.image_path.startswith(tmpdir)

    def test_screenshot_png_format(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.screenshot(invoice_pdf, image_format=ImageFormat.PNG)
        for ss in result.screenshots:
            assert ss.image_path.endswith(".png")

    def test_screenshot_jpg_format(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.screenshot(invoice_pdf, image_format="jpg")
        for ss in result.screenshots:
            assert ss.image_path.endswith(".jpg")


class TestScreenshotOptions:
    """Test screenshot options are forwarded correctly."""

    def test_target_pages(self, parser: LiteParse, invoice_pdf: Path):
        # invoice.pdf has 2 pages — screenshot only page 1
        result = parser.screenshot(invoice_pdf, target_pages="1")
        assert len(result) == 1
        assert result.screenshots[0].page_num == 1

    def test_load_bytes(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.screenshot(invoice_pdf, load_bytes=True)
        for ss in result.screenshots:
            assert ss.image_bytes is not None
            assert len(ss.image_bytes) > 0

    def test_no_load_bytes(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.screenshot(invoice_pdf, load_bytes=False)
        for ss in result.screenshots:
            assert ss.image_bytes is None

    def test_get_page_helper(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.screenshot(invoice_pdf)
        page1 = result.get_page(1)
        assert page1 is not None
        assert page1.page_num == 1
        assert result.get_page(999) is None

    def test_iterable(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.screenshot(invoice_pdf)
        pages = list(result)
        assert len(pages) == len(result)


class TestScreenshotErrors:
    """Test error handling."""

    def test_file_not_found(self, parser: LiteParse):
        with pytest.raises(FileNotFoundError):
            parser.screenshot("/nonexistent/file.pdf")
