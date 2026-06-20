"""LiteParse Python wrapper - native Rust bindings via PyO3."""

from pathlib import Path
from typing import Any, Dict, List, Optional, Union

from liteparse._liteparse import LiteParse as _NativeLiteParse
from liteparse._liteparse import search_items as _native_search_items

from .types import (
    ExtractedImage,
    LiteParseConfig,
    ParsedPage,
    ParseError,
    ParseResult,
    ScreenshotResult,
    TextItem,
)


def _convert_native_result(native_result: Any) -> ParseResult:
    """Convert a native PyParseResult to our Python ParseResult."""
    pages: List[ParsedPage] = []
    for native_page in native_result.pages:
        text_items = [
            TextItem(
                text=item.text,
                x=item.x,
                y=item.y,
                width=item.width,
                height=item.height,
                font_name=item.font_name,
                font_size=item.font_size,
                confidence=item.confidence,
            )
            for item in native_page.text_items
        ]
        pages.append(
            ParsedPage(
                page_num=native_page.page_num,
                width=native_page.width,
                height=native_page.height,
                text=native_page.text,
                text_items=text_items,
            )
        )
    images = [
        ExtractedImage(
            id=img.id,
            page=img.page,
            format=img.format,
            bytes=img.bytes,
        )
        for img in getattr(native_result, "images", [])
    ]
    return ParseResult(
        pages=pages,
        text=native_result.text,
        images=images,
    )


class LiteParse:
    """
    Python wrapper for the LiteParse document parser.

    Uses native Rust bindings for fast, in-process PDF parsing.

    Example:
        >>> from liteparse import LiteParse
        >>> parser = LiteParse()
        >>> result = parser.parse("document.pdf")
        >>> print(result.text)
    """

    def __init__(
        self,
        *,
        ocr_enabled: Optional[bool] = None,
        ocr_server_url: Optional[str] = None,
        ocr_server_headers: Optional[Dict[str, str]] = None,
        ocr_language: Optional[str] = None,
        tessdata_path: Optional[str] = None,
        max_pages: Optional[int] = None,
        target_pages: Optional[str] = None,
        dpi: Optional[float] = None,
        output_format: Optional[str] = None,
        preserve_very_small_text: Optional[bool] = None,
        password: Optional[str] = None,
        quiet: Optional[bool] = None,
        num_workers: Optional[int] = None,
        image_mode: Optional[str] = None,
        inline_images: Optional[bool] = None,
        extract_links: Optional[bool] = None,
    ):
        """
        Initialize LiteParse parser.

        Args:
            ocr_enabled: Whether to enable OCR for scanned documents (default: True)
            ocr_server_url: URL of HTTP OCR server (uses Tesseract if not provided)
            ocr_server_headers: Extra HTTP headers sent with every request to
                ``ocr_server_url`` (e.g. ``{"Authorization": "Bearer <token>"}``)
            ocr_language: Language code for OCR (e.g., "eng", "fra")
            tessdata_path: Path to tessdata directory for Tesseract
            max_pages: Maximum number of pages to parse
            target_pages: Specific pages to parse (e.g., "1-5,10,15-20")
            dpi: DPI for rendering (affects OCR quality)
            output_format: Output format: "json", "text", or "markdown" (default: "json")
            preserve_very_small_text: Whether to preserve very small text
            password: Password for encrypted/protected documents
            quiet: Suppress progress output
            num_workers: Number of concurrent OCR workers (default: CPU cores - 1)
            inline_images: Inline embedded PDF images into markdown text as base64 data URI images
            extract_links: Render hyperlink annotations as ``[text](url)`` in
                markdown output (default: True). Set False for plain anchor text.
        """
        kwargs = {}
        if ocr_enabled is not None:
            kwargs["ocr_enabled"] = ocr_enabled
        if ocr_server_url is not None:
            kwargs["ocr_server_url"] = ocr_server_url
        if ocr_server_headers is not None:
            kwargs["ocr_server_headers"] = ocr_server_headers
        if ocr_language is not None:
            kwargs["ocr_language"] = ocr_language
        if tessdata_path is not None:
            kwargs["tessdata_path"] = tessdata_path
        if max_pages is not None:
            kwargs["max_pages"] = max_pages
        if target_pages is not None:
            kwargs["target_pages"] = target_pages
        if dpi is not None:
            kwargs["dpi"] = dpi
        if output_format is not None:
            kwargs["output_format"] = output_format
        if preserve_very_small_text is not None:
            kwargs["preserve_very_small_text"] = preserve_very_small_text
        if password is not None:
            kwargs["password"] = password
        if quiet is not None:
            kwargs["quiet"] = quiet
        if num_workers is not None:
            kwargs["num_workers"] = num_workers
        if image_mode is not None:
            kwargs["image_mode"] = image_mode
        if inline_images is not None:
            kwargs["inline_images"] = inline_images
        if extract_links is not None:
            kwargs["extract_links"] = extract_links

        self._native = _NativeLiteParse(**kwargs)

    def parse(
        self,
        file_data: Union[str, Path, bytes],
    ) -> ParseResult:
        """
        Parse a document file.

        Args:
            file_data: Path to the document file, or raw PDF bytes.

        Returns:
            ParseResult containing the parsed document data.

        Raises:
            ParseError: If parsing fails.
            FileNotFoundError: If the file doesn't exist.
        """
        try:
            if isinstance(file_data, bytes):
                native_result = self._native.parse_bytes(file_data)
            else:
                file_path = Path(file_data)
                if not file_path.exists():
                    raise FileNotFoundError(f"File not found: {file_path}")
                native_result = self._native.parse(str(file_path.absolute()))
            return _convert_native_result(native_result)
        except FileNotFoundError:
            raise
        except Exception as e:
            raise ParseError(str(e)) from e

    def screenshot(
        self,
        file_path: Union[str, Path],
        *,
        page_numbers: Optional[List[int]] = None,
    ) -> List[ScreenshotResult]:
        """
        Generate screenshots of document pages.

        Supports PDFs natively. Non-PDF formats (DOCX, XLSX, images, etc.) are
        automatically converted to PDF before rendering when the required system
        tools are installed. Plain-text formats cannot be rendered.

        Args:
            file_path: Path to the document file (PDF, DOCX, images, etc.).
            page_numbers: Specific page numbers to screenshot (1-indexed).
                          If None, screenshots all pages.

        Returns:
            List of ScreenshotResult with PNG image bytes.

        Raises:
            FileNotFoundError: If the file doesn't exist.
            ParseError: If screenshot generation fails.
        """
        file_path = Path(file_path)
        if not file_path.exists():
            raise FileNotFoundError(f"File not found: {file_path}")

        try:
            native_results = self._native.screenshot(
                str(file_path.absolute()),
                page_numbers,
            )
            return [
                ScreenshotResult(
                    page_num=r.page_num,
                    width=r.width,
                    height=r.height,
                    image_bytes=r.image_bytes,
                )
                for r in native_results
            ]
        except Exception as e:
            raise ParseError(str(e)) from e

    def get_config(self) -> LiteParseConfig:
        """Return the resolved configuration."""
        cfg = self._native.config
        return LiteParseConfig(
            ocr_language=cfg.ocr_language,
            ocr_enabled=cfg.ocr_enabled,
            ocr_server_url=cfg.ocr_server_url,
            ocr_server_headers=cfg.ocr_server_headers,
            tessdata_path=cfg.tessdata_path,
            max_pages=cfg.max_pages,
            target_pages=cfg.target_pages,
            dpi=cfg.dpi,
            output_format=cfg.output_format,
            preserve_very_small_text=cfg.preserve_very_small_text,
            password=cfg.password,
            quiet=cfg.quiet,
            num_workers=cfg.num_workers,
            inline_images=cfg.inline_images,
        )

    def __repr__(self) -> str:
        return "LiteParse()"


def search_items(
    items: List[TextItem],
    phrase: str,
    *,
    case_sensitive: bool = False,
) -> List[TextItem]:
    """
    Search text items for phrase matches, returning merged items with combined bounding boxes.

    A phrase may span multiple adjacent text items. This function concatenates
    consecutive items, finds matches, and returns synthetic merged TextItem
    objects with combined bounding boxes.

    Args:
        items: List of TextItem objects to search through.
        phrase: The phrase to search for.
        case_sensitive: Whether the search should be case-sensitive (default: False).

    Returns:
        List of TextItem objects representing the matched regions.
    """
    native_results = _native_search_items(items, phrase, case_sensitive=case_sensitive)
    return [
        TextItem(
            text=item.text,
            x=item.x,
            y=item.y,
            width=item.width,
            height=item.height,
            font_name=item.font_name,
            font_size=item.font_size,
            confidence=item.confidence,
        )
        for item in native_results
    ]
