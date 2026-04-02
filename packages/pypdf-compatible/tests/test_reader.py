"""Tests for the pypdf-compatible wrapper."""

from __future__ import annotations

from decimal import Decimal
from unittest.mock import MagicMock, PropertyMock, patch

import pypdf
import pytest

from liteparse.types import BoundingBox, ParsedPage, ParseResult, TextItem

from liteparse_pypdf import (
    PdfReader,
    PageObject,
    PdfWriter,
    DocumentInformation,
    PdfReadError,
)
from liteparse_pypdf._page import _RectangleObject


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


def _make_text_item(text: str = "Hello", page: int = 1) -> TextItem:
    return TextItem(
        str=text, x=72.0, y=700.0,
        width=100.0, height=12.0, w=100.0, h=12.0, r=0,
        fontName="Helvetica", fontSize=12.0,
    )


def _make_parsed_page(page_num: int = 1) -> ParsedPage:
    return ParsedPage(
        pageNum=page_num,
        width=612.0, height=792.0,
        text=f"Page {page_num} text content",
        textItems=[_make_text_item(f"Hello from page {page_num}", page_num)],
        boundingBoxes=[BoundingBox(x1=72.0, y1=688.0, x2=172.0, y2=700.0)],
    )


def _make_parse_result(num_pages: int = 2) -> ParseResult:
    pages = [_make_parsed_page(i) for i in range(1, num_pages + 1)]
    return ParseResult(
        pages=pages,
        text="\n\n".join(p.text for p in pages),
    )


def _make_mock_pypdf_page(page_number: int = 0) -> MagicMock:
    """Create a mock pypdf.PageObject."""
    page = MagicMock(spec=pypdf.PageObject)
    page.page_number = page_number
    page.rotation = 0
    page.user_unit = 1.0
    page.annotations = None
    page.images = []

    # mediabox / cropbox / trimbox / artbox / bleedbox
    box = MagicMock()
    box.width = Decimal("612")
    box.height = Decimal("792")
    box.left = Decimal("0")
    box.bottom = Decimal("0")
    box.right = Decimal("612")
    box.top = Decimal("792")
    page.mediabox = box
    page.cropbox = box
    page.trimbox = box
    page.artbox = box
    page.bleedbox = box

    return page


def _make_mock_pypdf_reader(num_pages: int = 2) -> MagicMock:
    """Create a mock pypdf.PdfReader."""
    reader = MagicMock()
    reader.pages = [_make_mock_pypdf_page(i) for i in range(num_pages)]
    reader.metadata = MagicMock(spec=pypdf.DocumentInformation)
    reader.metadata.title = "Test Document"
    reader.metadata.author = "Test Author"
    reader.is_encrypted = False
    reader.pdf_header = "%PDF-1.7"
    reader.outline = []
    reader.named_destinations = {}
    reader.page_labels = [str(i + 1) for i in range(num_pages)]
    reader.page_layout = None
    reader.page_mode = None
    reader.attachments = {}
    reader.xfa = None
    reader.get_fields.return_value = None
    reader.get_form_text_fields.return_value = None
    return reader


# ---------------------------------------------------------------------------
# PdfReader tests
# ---------------------------------------------------------------------------


class TestPdfReader:
    """Tests for the PdfReader wrapper."""

    @patch("liteparse_pypdf._reader.pypdf.PdfReader")
    @patch("liteparse_pypdf._reader.LiteParse")
    def test_pages_length(self, MockLP: MagicMock, MockPypdf: MagicMock) -> None:
        MockLP.return_value.parse.return_value = _make_parse_result(3)
        MockPypdf.return_value = _make_mock_pypdf_reader(3)

        reader = PdfReader("fake.pdf")
        assert len(reader.pages) == 3
        assert len(reader) == 3
        assert reader.get_num_pages() == 3
        assert reader.numPages == 3

    @patch("liteparse_pypdf._reader.pypdf.PdfReader")
    @patch("liteparse_pypdf._reader.LiteParse")
    def test_page_access(self, MockLP: MagicMock, MockPypdf: MagicMock) -> None:
        MockLP.return_value.parse.return_value = _make_parse_result(2)
        MockPypdf.return_value = _make_mock_pypdf_reader(2)

        reader = PdfReader("fake.pdf")

        page = reader.pages[0]
        assert isinstance(page, PageObject)
        assert page.page_number == 0

        page1 = reader.get_page(1)
        assert page1.page_number == 1

        legacy = reader.getPage(0)
        assert legacy.page_number == 0

    @patch("liteparse_pypdf._reader.pypdf.PdfReader")
    @patch("liteparse_pypdf._reader.LiteParse")
    def test_metadata_from_pypdf(self, MockLP: MagicMock, MockPypdf: MagicMock) -> None:
        MockLP.return_value.parse.return_value = _make_parse_result(1)
        mock_reader = _make_mock_pypdf_reader(1)
        MockPypdf.return_value = mock_reader

        reader = PdfReader("fake.pdf")
        assert reader.metadata is mock_reader.metadata
        assert reader.metadata.title == "Test Document"
        assert reader.metadata.author == "Test Author"

    @patch("liteparse_pypdf._reader.pypdf.PdfReader")
    @patch("liteparse_pypdf._reader.LiteParse")
    def test_is_encrypted_from_pypdf(self, MockLP: MagicMock, MockPypdf: MagicMock) -> None:
        MockLP.return_value.parse.return_value = _make_parse_result(1)
        mock_reader = _make_mock_pypdf_reader(1)
        mock_reader.is_encrypted = True
        MockPypdf.return_value = mock_reader

        reader = PdfReader("fake.pdf")
        assert reader.is_encrypted is True

    @patch("liteparse_pypdf._reader.pypdf.PdfReader")
    @patch("liteparse_pypdf._reader.LiteParse")
    def test_outline(self, MockLP: MagicMock, MockPypdf: MagicMock) -> None:
        MockLP.return_value.parse.return_value = _make_parse_result(1)
        mock_reader = _make_mock_pypdf_reader(1)
        mock_reader.outline = [{"title": "Chapter 1"}]
        MockPypdf.return_value = mock_reader

        reader = PdfReader("fake.pdf")
        assert reader.outline == [{"title": "Chapter 1"}]

    @patch("liteparse_pypdf._reader.pypdf.PdfReader")
    @patch("liteparse_pypdf._reader.LiteParse")
    def test_page_labels(self, MockLP: MagicMock, MockPypdf: MagicMock) -> None:
        MockLP.return_value.parse.return_value = _make_parse_result(2)
        MockPypdf.return_value = _make_mock_pypdf_reader(2)

        reader = PdfReader("fake.pdf")
        assert reader.page_labels == ["1", "2"]

    @patch("liteparse_pypdf._reader.pypdf.PdfReader")
    @patch("liteparse_pypdf._reader.LiteParse")
    def test_attachments(self, MockLP: MagicMock, MockPypdf: MagicMock) -> None:
        MockLP.return_value.parse.return_value = _make_parse_result(1)
        mock_reader = _make_mock_pypdf_reader(1)
        mock_reader.attachments = {"file.txt": [b"content"]}
        MockPypdf.return_value = mock_reader

        reader = PdfReader("fake.pdf")
        assert reader.attachments == {"file.txt": [b"content"]}

    @patch("liteparse_pypdf._reader.pypdf.PdfReader")
    @patch("liteparse_pypdf._reader.LiteParse")
    def test_form_fields(self, MockLP: MagicMock, MockPypdf: MagicMock) -> None:
        MockLP.return_value.parse.return_value = _make_parse_result(1)
        mock_reader = _make_mock_pypdf_reader(1)
        mock_reader.get_fields.return_value = {"name": {"value": "John"}}
        mock_reader.get_form_text_fields.return_value = {"name": "John"}
        MockPypdf.return_value = mock_reader

        reader = PdfReader("fake.pdf")
        assert reader.get_fields() == {"name": {"value": "John"}}
        assert reader.get_form_text_fields() == {"name": "John"}

    @patch("liteparse_pypdf._reader.pypdf.PdfReader")
    @patch("liteparse_pypdf._reader.LiteParse")
    def test_context_manager(self, MockLP: MagicMock, MockPypdf: MagicMock) -> None:
        MockLP.return_value.parse.return_value = _make_parse_result(1)
        MockPypdf.return_value = _make_mock_pypdf_reader(1)

        with PdfReader("fake.pdf") as reader:
            assert len(reader.pages) == 1

    @patch("liteparse_pypdf._reader.pypdf.PdfReader")
    @patch("liteparse_pypdf._reader.LiteParse")
    def test_bytes_input(self, MockLP: MagicMock, MockPypdf: MagicMock) -> None:
        MockLP.return_value.parse.return_value = _make_parse_result(1)
        MockPypdf.return_value = _make_mock_pypdf_reader(1)

        reader = PdfReader(b"%PDF-1.7 fake content")
        assert len(reader.pages) == 1
        reader.close()


# ---------------------------------------------------------------------------
# PageObject tests
# ---------------------------------------------------------------------------


class TestPageObject:
    """Tests for the PageObject wrapper."""

    def _make_page(self, with_pypdf: bool = True) -> PageObject:
        parsed = _make_parsed_page(1)
        pypdf_page = _make_mock_pypdf_page(0) if with_pypdf else None
        return PageObject(parsed, pypdf_page=pypdf_page)

    def test_extract_text_uses_liteparse(self) -> None:
        """extract_text() always returns LiteParse text, not pypdf."""
        page = self._make_page(with_pypdf=True)
        assert page.extract_text() == "Page 1 text content"
        assert page.extractText() == "Page 1 text content"

    def test_extract_text_without_pypdf(self) -> None:
        page = self._make_page(with_pypdf=False)
        assert page.extract_text() == "Page 1 text content"

    def test_dimensions_from_pypdf(self) -> None:
        page = self._make_page(with_pypdf=True)
        assert page.width == Decimal("612")
        assert page.height == Decimal("792")

    def test_dimensions_fallback(self) -> None:
        page = self._make_page(with_pypdf=False)
        assert page.width == Decimal("612.0")
        assert page.height == Decimal("792.0")

    def test_page_number_from_pypdf(self) -> None:
        page = self._make_page(with_pypdf=True)
        assert page.page_number == 0

    def test_page_number_fallback(self) -> None:
        page = self._make_page(with_pypdf=False)
        assert page.page_number == 0  # pageNum=1 → 0-indexed

    def test_mediabox_from_pypdf(self) -> None:
        page = self._make_page(with_pypdf=True)
        box = page.mediabox
        assert box.width == Decimal("612")

    def test_mediabox_fallback(self) -> None:
        page = self._make_page(with_pypdf=False)
        box = page.mediabox
        assert isinstance(box, _RectangleObject)
        assert box.width == Decimal("612.0")

    def test_rotation_from_pypdf(self) -> None:
        page = self._make_page(with_pypdf=True)
        assert page.rotation == 0

    def test_rotation_fallback(self) -> None:
        page = self._make_page(with_pypdf=False)
        assert page.rotation == 0

    def test_images_from_pypdf(self) -> None:
        page = self._make_page(with_pypdf=True)
        assert page.images == []

    def test_images_fallback(self) -> None:
        page = self._make_page(with_pypdf=False)
        assert page.images == []

    def test_annotations_from_pypdf(self) -> None:
        page = self._make_page(with_pypdf=True)
        assert page.annotations is None

    def test_visitor_text(self) -> None:
        page = self._make_page()
        collected: list[str] = []

        def visitor(text: str, *args: object) -> None:
            collected.append(text)

        page.extract_text(visitor_text=visitor)
        assert collected == ["Hello from page 1"]

    def test_text_items_extra(self) -> None:
        page = self._make_page()
        assert len(page.text_items) == 1
        assert page.text_items[0].str == "Hello from page 1"

    def test_dict_access_from_pypdf(self) -> None:
        page = self._make_page(with_pypdf=True)
        # Mock the __getitem__ on the pypdf page
        page._pypdf_page.__getitem__.return_value = "some_value"
        assert page["/Resources"] == "some_value"

    def test_dict_access_fallback(self) -> None:
        page = self._make_page(with_pypdf=False)
        with pytest.raises(KeyError):
            page["/Resources"]

    def test_get_with_default(self) -> None:
        page = self._make_page(with_pypdf=False)
        assert page.get("/Resources", "default") == "default"

    def test_scale_delegates_to_pypdf(self) -> None:
        page = self._make_page(with_pypdf=True)
        page.scale(2.0, 2.0)
        page._pypdf_page.scale.assert_called_once_with(2.0, 2.0)

    def test_scale_raises_without_pypdf(self) -> None:
        page = self._make_page(with_pypdf=False)
        with pytest.raises(NotImplementedError):
            page.scale(2.0, 2.0)

    def test_rotate_delegates_to_pypdf(self) -> None:
        page = self._make_page(with_pypdf=True)
        result = page.rotate(90)
        page._pypdf_page.rotate.assert_called_once_with(90)
        assert result is page


# ---------------------------------------------------------------------------
# RectangleObject fallback tests
# ---------------------------------------------------------------------------


class TestRectangleObject:
    def test_properties(self) -> None:
        rect = _RectangleObject(10, 20, 110, 120)
        assert rect.left == Decimal("10")
        assert rect.bottom == Decimal("20")
        assert rect.right == Decimal("110")
        assert rect.top == Decimal("120")
        assert rect.width == Decimal("100")
        assert rect.height == Decimal("100")

    def test_indexing(self) -> None:
        rect = _RectangleObject(0, 0, 100, 200)
        assert rect[2] == Decimal("100")
        assert rect[3] == Decimal("200")


# ---------------------------------------------------------------------------
# Re-export tests
# ---------------------------------------------------------------------------


class TestReExports:
    """Verify that commonly used pypdf classes are available."""

    def test_pdf_writer_is_pypdf(self) -> None:
        assert PdfWriter is pypdf.PdfWriter

    def test_document_information_is_pypdf(self) -> None:
        assert DocumentInformation is pypdf.DocumentInformation

    def test_pdf_read_error_is_pypdf(self) -> None:
        from pypdf.errors import PdfReadError as OrigError
        assert PdfReadError is OrigError
