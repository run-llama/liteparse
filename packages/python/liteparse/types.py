"""Python-friendly type wrappers around the native Rust bindings."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, Iterator, List, Optional, Tuple


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
class AnnotationRect:
    """Annotation rectangle in top-left, 72-DPI viewport coordinates."""
    x: float
    y: float
    width: float
    height: float


@dataclass
class DocumentAnnotation:
    """One PDF annotation extracted from a page."""
    subtype: str
    contents: Optional[str] = None
    created: Optional[str] = None
    modified: Optional[str] = None
    title: Optional[str] = None
    rect: Optional[AnnotationRect] = None
    quadpoint_rects: List[AnnotationRect] = field(default_factory=list)
    uri: Optional[str] = None


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
    #: Present only when parsing with ``extract_annotations=True``.
    annotations: Optional[List[DocumentAnnotation]] = None


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
class LayoutComplexityStats:
    """Layout-difficulty signals for one page (columns, tables, dense
    graphics), computed from the real grid-projection pass. Orthogonal to
    ``needs_ocr``: none of these imply OCR — they signal that the text-only
    path may mangle reading order or structure."""
    #: Side-by-side text columns found by the layout pass (1 = single column).
    column_count: int
    #: Ruled-table grids detected.
    ruled_table_count: int
    #: Combined ruled-table area over page area, clamped to 1.
    ruled_table_coverage: float
    #: Borderless table runs found by track-aligned text detection
    #: (description lists excluded). Ruled tables can appear here too — do not
    #: sum with ``ruled_table_count``.
    text_table_run_count: int
    #: Figure regions clustered from vector graphics.
    figure_count: int
    #: Combined figure area over page area, clamped to 1.
    figure_coverage: float
    #: Whether any layout reason fired.
    is_complex: bool
    #: Layout reasons (e.g. ``"multi-column"``, ``"table-likely"``,
    #: ``"dense-graphics"``); new reasons may be added over time.
    reasons: list[str]


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
    #: Layout-difficulty signals; see :class:`LayoutComplexityStats`.
    layout: Optional[LayoutComplexityStats] = None


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
    image_mode: str
    extract_links: bool
    extract_annotations: bool
    ocr_failure_fatal: bool
    ocr_hedge_delays_ms: List[int]
    emit_word_boxes: bool
    #: ``(top, right, bottom, left)`` crop fractions, or ``None`` when the whole
    #: page is kept.
    crop_box: Optional[Tuple[float, float, float, float]]
    skip_diagonal_text: bool
    include_complexity: bool


class ParseError(Exception):
    """Exception raised when parsing fails."""
    pass
