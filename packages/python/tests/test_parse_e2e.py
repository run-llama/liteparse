"""E2E tests for LiteParse.parse() — validates Python types match CLI output."""

from pathlib import Path

import pytest

from liteparse import (
    BoundingBox,
    LiteParse,
    ParsedPage,
    ParseError,
    ParseResult,
    TextItem,
)


class TestParseBasic:
    """Basic parse functionality and output structure."""

    def test_parse_returns_parse_result(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.parse(invoice_pdf, ocr_enabled=False)
        assert isinstance(result, ParseResult)

    def test_parse_result_has_pages(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.parse(invoice_pdf, ocr_enabled=False)
        assert len(result.pages) > 0
        assert result.num_pages == len(result.pages)

    def test_parse_result_has_text(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.parse(invoice_pdf, ocr_enabled=False)
        assert isinstance(result.text, str)
        assert len(result.text) > 0

    def test_parse_result_has_json(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.parse(invoice_pdf, ocr_enabled=False)
        assert result.json is not None
        assert "pages" in result.json

    def test_parse_bytest_input(self, parser: LiteParse, invoice_pdf: Path):
        file_bytes = invoice_pdf.read_bytes()
        result = parser.parse(file_bytes=file_bytes)
        assert result.json is not None
        assert "pages" in result.json

    @pytest.mark.asyncio
    async def test_parse_async(self, parser: LiteParse, invoice_pdf: Path):
        result = await parser.parse_async(invoice_pdf)
        assert result.json is not None
        assert "pages" in result.json

    @pytest.mark.asyncio
    async def test_parse_async_bytes_input(self, parser: LiteParse, invoice_pdf: Path):
        file_bytes = invoice_pdf.read_bytes()
        result = await parser.parse_async(file_bytes=file_bytes)
        assert result.json is not None
        assert "pages" in result.json


class TestParsedPageStructure:
    """Validate ParsedPage fields match CLI output."""

    def test_page_fields(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.parse(invoice_pdf, ocr_enabled=False)
        page = result.pages[0]
        assert isinstance(page, ParsedPage)
        assert isinstance(page.pageNum, int)
        assert page.pageNum >= 1
        assert isinstance(page.width, (int, float))
        assert isinstance(page.height, (int, float))
        assert page.width > 0
        assert page.height > 0
        assert isinstance(page.text, str)

    def test_page_has_text_items(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.parse(invoice_pdf, ocr_enabled=False)
        page = result.pages[0]
        assert len(page.textItems) > 0

    def test_text_item_fields(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.parse(invoice_pdf, ocr_enabled=False)
        item = result.pages[0].textItems[0]
        assert isinstance(item, TextItem)
        assert isinstance(item.text, str)
        assert isinstance(item.x, (int, float))
        assert isinstance(item.y, (int, float))
        assert isinstance(item.width, (int, float))
        assert isinstance(item.height, (int, float))
        # fontName and fontSize may be None or present
        if item.fontName is not None:
            assert isinstance(item.fontName, str)
        if item.fontSize is not None:
            assert isinstance(item.fontSize, (int, float))

    def test_bounding_box_fields(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.parse(invoice_pdf, ocr_enabled=False, precise_bounding_box=True)
        page = result.pages[0]
        assert len(page.boundingBoxes) > 0
        bbox = page.boundingBoxes[0]
        assert isinstance(bbox, BoundingBox)
        assert isinstance(bbox.x1, (int, float))
        assert isinstance(bbox.y1, (int, float))
        assert isinstance(bbox.x2, (int, float))
        assert isinstance(bbox.y2, (int, float))


class TestParseOptions:
    """Test that CLI options are correctly forwarded."""

    def test_target_pages(self, parser: LiteParse, invoice_pdf: Path):
        # invoice.pdf has 2 pages — parse only page 1
        result = parser.parse(invoice_pdf, ocr_enabled=False, target_pages="1")
        assert result.num_pages == 1
        assert result.pages[0].pageNum == 1

    def test_max_pages(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.parse(invoice_pdf, ocr_enabled=False, max_pages=1)
        assert result.num_pages == 1

    def test_no_precise_bbox(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.parse(
            invoice_pdf, ocr_enabled=False, precise_bounding_box=False
        )
        assert isinstance(result, ParseResult)
        # With no precise bbox, boundingBoxes should be empty
        for page in result.pages:
            assert len(page.boundingBoxes) == 0

    def test_get_page_helper(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.parse(invoice_pdf, ocr_enabled=False)
        page1 = result.get_page(1)
        assert page1 is not None
        assert page1.pageNum == 1
        assert result.get_page(999) is None

    def test_multi_page_text_joined(self, parser: LiteParse, invoice_pdf: Path):
        result = parser.parse(invoice_pdf, ocr_enabled=False)
        assert result.num_pages == 2
        # result.text should be the join of all page texts
        expected = "\n\n".join(p.text for p in result.pages)
        assert result.text == expected


class TestParseErrors:
    """Test error handling."""

    def test_file_not_found(self, parser: LiteParse):
        with pytest.raises(FileNotFoundError):
            parser.parse("/nonexistent/file.pdf")

    def test_cli_not_found(self):
        parser = LiteParse(cli_path="/nonexistent/liteparse")
        # subprocess raises FileNotFoundError when the binary doesn't exist
        with pytest.raises((ParseError, FileNotFoundError, OSError)):
            parser.parse(Path(__file__))  # any existing file

    def test_timeout(self, parser: LiteParse, invoice_pdf: Path):
        # Extremely short timeout should fail
        with pytest.raises(TimeoutError):
            parser.parse(invoice_pdf, timeout=0.001)
