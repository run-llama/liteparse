"""Python-friendly type wrappers around the native Rust bindings."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, Iterator, List, Optional


@dataclass
class WordBox:
    """One word's bounding box within a :class:`TextItem`, in the same viewport
    space (top-left origin, 72 DPI). ``text`` excludes inter-word spaces."""
    text: str
    x: float
    y: float
    width: float
    height: float


@dataclass
class TextItem:
    """Individual text item extracted from a document."""
    text: str
    x: float
    y: float
    width: float
    height: float
    font_name: Optional[str] = None
    font_size: Optional[float] = None
    confidence: Optional[float] = None
    rotation: float = 0.0
    #: Per-word sub-boxes. Empty unless the parser was configured with
    #: ``emit_word_boxes=True``.
    words: List[WordBox] = field(default_factory=list)


@dataclass
class ParsedPage:
    """A parsed page from a document."""
    page_num: int
    width: float
    height: float
    text: str
    markdown: str = ""
    text_items: List[TextItem] = field(default_factory=list)
    #: Per-page complexity signals (the same :meth:`LiteParse.is_complex`
    #: returns). Populated only when parsing with ``include_complexity=True``;
    #: ``None`` otherwise.
    complexity: Optional[PageComplexityStats] = None


@dataclass
class ExtractedImage:
    """An embedded raster image extracted from a page.

    Populated only when the parser was configured with ``image_mode="embed"``.
    The ``id`` matches the reference used in the markdown output
    (e.g. ``![](image_p1_0.png)`` → ``id="p1_0"``).
    """
    id: str
    page: int
    format: str
    bytes: bytes


@dataclass
class ParseResult:
    """Result of parsing a document."""
    pages: List[ParsedPage]
    text: str
    images: List[ExtractedImage] = field(default_factory=list)
    #: 1-based page numbers whose OCR task failed while parsing continued.
    failed_ocr_pages: List[int] = field(default_factory=list)
    #: OCR failures with page numbers and engine error messages.
    ocr_failures: List[OcrFailure] = field(default_factory=list)

    @property
    def num_pages(self) -> int:
        return len(self.pages)

    def get_page(self, page_num: int) -> Optional[ParsedPage]:
        """Get a specific page by number (1-indexed)."""
        for page in self.pages:
            if page.page_num == page_num:
                return page
        return None


@dataclass
class ScreenshotResult:
    """Result of a single page screenshot."""
    page_num: int
    width: int
    height: int
    image_bytes: bytes


@dataclass
class PageComplexityStats:
    """Per-page complexity signals used to decide whether a document needs OCR."""
    page_number: int
    text_length: int
    text_coverage: float
    has_substantial_images: bool
    image_block_count: int
    image_coverage: float
    largest_image_coverage: float
    full_page_image: bool
    uncovered_vector_area: Optional[float]
    is_garbled: bool
    page_area: float
    needs_ocr: bool
    reasons: list[str]


@dataclass
class OcrFailure:
    """A failed OCR page and the error reported by the OCR engine."""
    page_number: int
    error: str


@dataclass
class LiteParseConfig:
    """Resolved parser configuration."""
    ocr_language: str
    ocr_enabled: bool
    ocr_server_url: Optional[str]
    ocr_server_headers: Optional[Dict[str, str]]
    tessdata_path: Optional[str]
    max_pages: int
    target_pages: Optional[str]
    dpi: float
    output_format: str
    preserve_very_small_text: bool
    password: Optional[str]
    quiet: bool
    num_workers: int


class ParseError(Exception):
    """Exception raised when parsing fails."""
    pass
