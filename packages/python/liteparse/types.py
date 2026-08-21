"""Python-friendly type wrappers around the native Rust bindings."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, Iterator, List, Optional, Tuple, Union


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
    font_height: Optional[float] = None
    font_ascent: Optional[float] = None
    font_descent: Optional[float] = None
    font_weight: Optional[int] = None
    text_width: Optional[float] = None
    font_is_buggy: bool = False
    mcid: Optional[int] = None
    #: Fill color as an eight-character ARGB hex string.
    fill_color: Optional[str] = None
    #: Stroke color as an eight-character ARGB hex string.
    stroke_color: Optional[str] = None
    #: Raw PDF content-stream character codes for the source glyphs.
    char_codes: List[int] = field(default_factory=list)
    #: True when the trailing source space was synthesized by PDFium.
    trailing_space_generated: bool = False
    #: OCR confidence score (0.0-1.0). ``None`` for native PDF text.
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
class LayoutCell:
    """One table cell: its text and the region of the page it was read from.

    ``bbox`` is ``None`` for cells with no ink behind them -- padding inserted
    to square off a ragged grid, or halves of a merged run split at an
    estimated position rather than an observed boundary.
    """
    text: str
    bbox: Optional[AnnotationRect] = None
    #: Indices into the page's returned ``text_items``, in reading order,
    #: never repeating within one cell; empty for padding cells.
    text_item_indices: List[int] = field(default_factory=list)


@dataclass
class LayoutBlock:
    """A classified block of page content, discriminated by ``kind``.

    ``kind`` is one of ``heading``, ``paragraph``, ``list_item``, ``code``,
    ``table``, ``grid_fallback``, ``rule``, ``figure``. Fields that do not
    apply to a block's kind are ``None``.
    """
    kind: str
    #: Indices into the page's returned ``text_items``, sorted and deduped;
    #: empty for text-less blocks. For a ``table`` block, the union of its
    #: cells' indices.
    text_item_indices: List[int] = field(default_factory=list)
    #: Rendered text for ``heading``, ``paragraph`` and ``list_item``.
    text: Optional[str] = None
    #: Heading level (1-6), or list nesting depth for ``list_item``.
    level: Optional[int] = None
    bold: bool = False
    italic: bool = False
    #: ``list_item`` only; ``marker`` is the marker as it appeared on the page.
    ordered: Optional[bool] = None
    marker: Optional[str] = None
    #: Verbatim source lines for ``code`` and ``grid_fallback``.
    lines: Optional[List[str]] = None
    #: Best-effort language hint for ``code``.
    lang: Optional[str] = None
    #: ``table`` only.
    header: Optional[List[LayoutCell]] = None
    rows: Optional[List[List[LayoutCell]]] = None
    #: ``figure`` only, matching the ``img_{id}.{format}`` Markdown target.
    id: Optional[str] = None
    format: Optional[str] = None
    #: Region this block occupies, in the same top-left 72-DPI viewport space
    #: as ``text_items``. The union of every source line that fed the block.
    bbox: Optional[AnnotationRect] = None


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
class FormField:
    """One AcroForm widget and its resolved field metadata."""
    id: str
    #: Widget type, e.g. ``"text"``, ``"checkbox"``, ``"combobox"``. Named
    #: ``type`` to match the ``type`` key in JSON output and the Node bindings.
    type: str
    page: int
    annotation_index: int
    widget_index: int
    field_flags: int
    object_number: Optional[int] = None
    name: Optional[str] = None
    alternate_name: Optional[str] = None
    value: Optional[str] = None
    export_value: Optional[str] = None
    control_count: Optional[int] = None
    control_index: Optional[int] = None
    checked: Optional[bool] = None
    rect: Optional[AnnotationRect] = None
    options: List[str] = field(default_factory=list)
    selected_options: List[str] = field(default_factory=list)


StructureAttributeValue = Union[bool, float, str]


@dataclass
class StructureTreeElement:
    """One element in a tagged-PDF logical structure tree."""
    #: Structure tag, e.g. ``"P"``, ``"Table"``, ``"H1"``. Named ``type`` to
    #: match the ``type`` key in JSON output and the Node bindings.
    type: str
    id: Optional[str] = None
    actual_text: Optional[str] = None
    alt_text: Optional[str] = None
    title: Optional[str] = None
    attributes: Dict[str, StructureAttributeValue] = field(default_factory=dict)
    marked_content_ids: List[int] = field(default_factory=list)
    children: List[StructureTreeElement] = field(default_factory=list)
    annotations: List[DocumentAnnotation] = field(default_factory=list)


@dataclass
class StructureTree:
    """Complete page-scoped tagged-PDF logical structure."""
    roots: List[StructureTreeElement] = field(default_factory=list)


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
    #: Present only when parsing with ``extract_vector_graphics=True``.
    vector_graphics: Optional[VectorGraphics] = None
    #: Present only when parsing with ``extract_annotations=True``.
    annotations: Optional[List[DocumentAnnotation]] = None
    #: Present only when parsing with ``extract_form_fields=True``.
    form_fields: Optional[List[FormField]] = None
    #: Present only when parsing with ``extract_structure_tree=True``.
    structure_tree: Optional[StructureTree] = None
    #: Classified layout blocks in reading order -- the same blocks, in the
    #: same order, the page's Markdown is built from. Present only when
    #: parsing with ``extract_blocks=True``.
    blocks: Optional[List[LayoutBlock]] = None
    #: Union bbox ``(x, y, width, height)`` of the page's top-level content
    #: objects in viewport coords (visible content extent). Present only when
    #: parsing with ``extract_content_bounds=True``; ``None`` otherwise (and
    #: for empty pages).
    content_bounds: Optional[Tuple[float, float, float, float]] = None


@dataclass
class VectorShape:
    bbox: Tuple[float, float, float, float]
    stroke: bool
    stroke_color: Optional[str]
    fill: bool
    fill_color: Optional[str]
    has_curve: bool


@dataclass
class VectorLine:
    x1: float
    y1: float
    x2: float
    y2: float
    stroke: bool
    stroke_width: Optional[float]
    stroke_color: Optional[str]
    fill: bool
    fill_color: Optional[str]


@dataclass
class VectorGraphics:
    shapes: List[VectorShape]
    lines: List[VectorLine]


@dataclass
class ImageRect:
    """Image placement in viewport coordinates (top-left origin, 72 DPI)."""
    x: float
    y: float
    width: float
    height: float


@dataclass
class ExtractedImage:
    """An embedded raster image extracted from a page.

    Populated only when ``extract_images=True``. ``image_mode`` controls
    Markdown presentation independently.
    The ``id`` matches the reference used in the markdown output
    (e.g. ``![](img_p1_1.png)`` → ``id="p1_1"``).
    """
    id: str
    name: str
    path: Optional[str]
    page: int
    bbox: ImageRect
    width: int
    height: int
    rotation: float
    format: str
    bytes: bytes
    duplicate_of: Optional[str] = None


@dataclass
class XfaPacket:
    """One raw packet from an XFA form document's ``/XFA`` array."""
    index: int
    name: Optional[str]
    content_length: int
    #: Packet content (usually XML), lossily decoded as UTF-8.
    content: Optional[str]


@dataclass
class DocumentMetadata:
    """Document-level provenance metadata from PDFium and the source PDF."""
    creation_date: Optional[str] = None
    mod_date: Optional[str] = None
    #: Encoded PDF version (14 means PDF 1.4).
    file_version: Optional[int] = None
    is_encrypted: Optional[bool] = None
    security_handler_revision: Optional[int] = None
    permissions: Optional[int] = None
    eof_section_count: Optional[int] = None
    startxref_count: Optional[int] = None
    trailer_id_pair_differs: Optional[bool] = None
    raw_file_size: Optional[int] = None
    #: The document catalog's ``/Metadata`` XMP packet, capped at 64 KiB.
    #: ``None`` when the document has none or it is too large to resolve
    #: cheaply.
    xmp: Optional[str] = None
    #: True when the catalog's XMP stream exceeded the 64 KiB cap.
    xmp_truncated: Optional[bool] = None
    signature_count: Optional[int] = None
    signature_byte_range_reaches_eof: Optional[bool] = None


@dataclass
class PageError:
    """A page-level extraction failure skipped during a tolerant parse."""
    page_num: int
    message: str


@dataclass
class ParseResult:
    """Result of parsing a document."""
    pages: List[ParsedPage]
    text: str
    #: Total source-document pages before target/max-page filtering.
    total_pages: int = 0
    images: List[ExtractedImage] = field(default_factory=list)
    screenshots: List["ScreenshotResult"] = field(default_factory=list)
    image_error_count: int = 0
    page_errors: List[PageError] = field(default_factory=list)
    #: PDFium form type, present only when ``extract_form_fields=True``.
    form_type: Optional[int] = None
    #: The document's ``/Info`` ``Creator`` entry, when present.
    creator: Optional[str] = None
    #: The document's ``/Info`` ``Producer`` entry, when present.
    producer: Optional[str] = None
    #: Document-level provenance metadata. Present only when
    #: ``extract_document_metadata=True`` and the input was a real PDF
    #: (not converted from DOCX/XLSX/an image).
    doc_meta: Optional[DocumentMetadata] = None
    #: Raw XFA packets; present only when ``extract_xfa_packets=True``.
    xfa_packets: Optional[List[XfaPacket]] = None

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
class ParseBatch:
    """One batch of pages from :meth:`LiteParse.parse_batches`."""
    #: First source page in this batch (1-indexed).
    start_page: int
    #: Last source page in this batch (1-indexed, inclusive).
    end_page: int
    #: Total source-document pages, before the parser's ``max_pages`` cap.
    total_pages: int
    #: The pages in ``start_page..end_page``, as an ordinary parse result.
    result: ParseResult


@dataclass
class ScreenshotRect:
    """One solid rectangle (or line) detected in a rendered page bitmap,
    in viewport coords (top-left origin, 72 DPI)."""
    x: float
    y: float
    width: float
    height: float
    #: Fill color as ARGB hex string (e.g. ``"ff1a2b3c"``).
    color: str
    #: True when the region is a solid line rather than a filled area.
    is_line: bool


@dataclass
class ScreenshotResult:
    """Result of a single page screenshot."""
    page_num: int
    width: int
    height: int
    image_bytes: bytes
    #: True when every pixel has the same color (blank page after render).
    is_solid_fill: bool = False
    #: Solid rectangles/lines detected in the raster. Populated only when
    #: ``detect_screenshot_rects=True``.
    rects: List[ScreenshotRect] = field(default_factory=list)


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
    #: Length of usable native text (garbled/unmappable items excluded).
    text_length: int
    #: Fraction of the page area covered by native text (0–1).
    text_coverage: float
    has_substantial_images: bool
    #: Number of counted raster images — inline figures only; full-page
    #: backgrounds are excluded (see class docstring).
    image_block_count: int
    #: Summed bbox area of the counted images over page area, clamped to 1.
    #: Overlapping images can inflate the raw sum, so read it as "summed
    #: image-bbox area", not unique covered area. A full-page scan raster
    #: contributes 0 here — see ``full_page_image``.
    image_coverage: float
    #: Area of the single largest counted image over page area, clamped to 1.
    #: Same exclusion as ``image_coverage``: a full-page raster contributes 0.
    largest_image_coverage: float
    #: A single raster covering ≥90% of the page is present. Such full-page
    #: backgrounds are excluded from ``image_coverage`` /
    #: ``largest_image_coverage`` (they're not inline figures), so this flag is
    #: the only signal that distinguishes a scan from a genuinely blank page —
    #: both otherwise report no text and no counted images.
    full_page_image: bool
    #: Filled vector-outline area not covered by native text, in pt².
    #: ``None`` when a cheaper signal already decided the page, so this
    #: expensive walk was skipped.
    uncovered_vector_area: Optional[float]
    is_garbled: bool
    page_area: float
    #: Verdict: whether this page needs more than the cheap text-only path.
    needs_ocr: bool
    #: Every reason the page was flagged (e.g. ``"scanned"``,
    #: ``"sparse-text"``, ``"garbled"``). Empty exactly when ``needs_ocr`` is
    #: False; new reasons may be added over time.
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
    extract_screenshots: bool
    continue_on_page_error: bool
    dpi: float
    output_format: str
    preserve_very_small_text: bool
    password: Optional[str]
    quiet: bool
    num_workers: int
    image_mode: str
    image_output_dir: Optional[str]
    extract_links: bool
    extract_annotations: bool
    extract_form_fields: bool
    extract_structure_tree: bool
    extract_blocks: bool
    ocr_failure_fatal: bool
    ocr_hedge_delays_ms: List[int]
    emit_word_boxes: bool
    #: ``(top, right, bottom, left)`` crop fractions, or ``None`` when the whole
    #: page is kept.
    crop_box: Optional[Tuple[float, float, float, float]]
    skip_diagonal_text: bool
    include_complexity: bool
    extract_text_metadata: bool = False
    #: Keep running headers/footers in markdown output instead of stripping
    #: repeated page-band lines and page chrome.
    keep_headers_footers: bool = False
    extract_images: bool = False
    extract_vector_graphics: bool = False
    extract_xfa_packets: bool = False
    extract_document_metadata: bool = False
    detect_screenshot_rects: bool = False
    extract_content_bounds: bool = False


class ParseError(Exception):
    """Exception raised when parsing fails."""
    pass
