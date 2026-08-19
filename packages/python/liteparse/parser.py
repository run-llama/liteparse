"""LiteParse Python wrapper - native Rust bindings via PyO3."""

from pathlib import Path
from typing import Any, Dict, Iterator, List, Optional, Tuple, Union

from liteparse._liteparse import LiteParse as _NativeLiteParse
from liteparse._liteparse import search_items as _native_search_items

from .types import (
    AnnotationRect,
    DocumentAnnotation,
    LayoutBlock,
    LayoutCell,
    FormField,
    StructureTree,
    StructureTreeElement,
    ExtractedImage,
    ImageRect,
    LayoutComplexityStats,
    LiteParseConfig,
    PageComplexityStats,
    ParsedPage,
    ParseBatch,
    ParseError,
    PageError,
    ParseResult,
    DocumentMetadata,
    ScreenshotRect,
    ScreenshotResult,
    ScreenshotFormat,
    ScreenshotOptions,
    TextItem,
    XfaPacket,
    WordBox,
    VectorGraphics,
    VectorLine,
    VectorShape,
)


def _convert_rect(rect: Any) -> Optional[AnnotationRect]:
    if rect is None:
        return None
    return AnnotationRect(
        x=rect.x, y=rect.y, width=rect.width, height=rect.height
    )


def _convert_cell(cell: Any) -> LayoutCell:
    return LayoutCell(text=cell.text, bbox=_convert_rect(cell.bbox))


def _convert_block(block: Any) -> LayoutBlock:
    return LayoutBlock(
        kind=block.kind,
        text=block.text,
        level=block.level,
        bold=block.bold,
        italic=block.italic,
        ordered=block.ordered,
        marker=block.marker,
        lines=list(block.lines) if block.lines is not None else None,
        lang=block.lang,
        header=(
            [_convert_cell(c) for c in block.header]
            if block.header is not None
            else None
        ),
        rows=(
            [[_convert_cell(c) for c in row] for row in block.rows]
            if block.rows is not None
            else None
        ),
        id=block.id,
        format=block.format,
        bbox=_convert_rect(block.bbox),
    )


def _convert_annotation(annotation: Any) -> DocumentAnnotation:
    return DocumentAnnotation(
        subtype=annotation.subtype,
        contents=annotation.contents,
        created=annotation.created,
        modified=annotation.modified,
        title=annotation.title,
        rect=(
            AnnotationRect(
                x=annotation.rect.x,
                y=annotation.rect.y,
                width=annotation.rect.width,
                height=annotation.rect.height,
            )
            if annotation.rect is not None
            else None
        ),
        quadpoint_rects=[
            AnnotationRect(x=r.x, y=r.y, width=r.width, height=r.height)
            for r in annotation.quadpoint_rects
        ],
        uri=annotation.uri,
    )


def _convert_structure_element(element: Any) -> StructureTreeElement:
    attributes = {}
    for attribute in element.attributes:
        if attribute.boolean_value is not None:
            attributes[attribute.name] = attribute.boolean_value
        elif attribute.number_value is not None:
            attributes[attribute.name] = attribute.number_value
        elif attribute.string_value is not None:
            attributes[attribute.name] = attribute.string_value
    return StructureTreeElement(
        type=element.element_type,
        id=element.id,
        actual_text=element.actual_text,
        alt_text=element.alt_text,
        title=element.title,
        attributes=attributes,
        marked_content_ids=list(element.marked_content_ids),
        children=[_convert_structure_element(child) for child in element.children],
        annotations=[_convert_annotation(a) for a in element.annotations],
    )


def _convert_complexity(s: Any) -> PageComplexityStats:
    """Convert a native PyPageComplexityStats to our Python dataclass."""
    return PageComplexityStats(
        page_number=s.page_number,
        text_length=s.text_length,
        text_coverage=s.text_coverage,
        has_substantial_images=s.has_substantial_images,
        image_block_count=s.image_block_count,
        image_coverage=s.image_coverage,
        largest_image_coverage=s.largest_image_coverage,
        full_page_image=s.full_page_image,
        uncovered_vector_area=s.uncovered_vector_area,
        is_garbled=s.is_garbled,
        page_area=s.page_area,
        needs_ocr=s.needs_ocr,
        reasons=list(s.reasons),
        layout=(
            LayoutComplexityStats(
                column_count=s.layout.column_count,
                ruled_table_count=s.layout.ruled_table_count,
                ruled_table_coverage=s.layout.ruled_table_coverage,
                text_table_run_count=s.layout.text_table_run_count,
                figure_count=s.layout.figure_count,
                figure_coverage=s.layout.figure_coverage,
                is_complex=s.layout.is_complex,
                reasons=list(s.layout.reasons),
            )
            if getattr(s, "layout", None) is not None
            else None
        ),
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
                font_height=getattr(item, "font_height", None),
                font_ascent=getattr(item, "font_ascent", None),
                font_descent=getattr(item, "font_descent", None),
                font_weight=getattr(item, "font_weight", None),
                text_width=getattr(item, "text_width", None),
                font_is_buggy=getattr(item, "font_is_buggy", False),
                mcid=getattr(item, "mcid", None),
                fill_color=getattr(item, "fill_color", None),
                stroke_color=getattr(item, "stroke_color", None),
                char_codes=list(getattr(item, "char_codes", [])),
                trailing_space_generated=getattr(item, "trailing_space_generated", False),
                confidence=item.confidence,
                rotation=getattr(item, "rotation", 0.0),
                words=[
                    WordBox(
                        text=w.text,
                        x=w.x,
                        y=w.y,
                        width=w.width,
                        height=w.height,
                    )
                    for w in getattr(item, "words", [])
                ],
            )
            for item in native_page.text_items
        ]
        native_complexity = getattr(native_page, "complexity", None)
        native_vectors = getattr(native_page, "vector_graphics", None)
        native_annotations = getattr(native_page, "annotations", None)
        native_form_fields = getattr(native_page, "form_fields", None)
        native_structure_tree = getattr(native_page, "structure_tree", None)
        native_blocks = getattr(native_page, "blocks", None)
        pages.append(
            ParsedPage(
                page_num=native_page.page_num,
                width=native_page.width,
                height=native_page.height,
                content_bounds=(
                    (
                        native_content_bounds.x,
                        native_content_bounds.y,
                        native_content_bounds.width,
                        native_content_bounds.height,
                    )
                    if (
                        native_content_bounds := getattr(
                            native_page, "content_bounds", None
                        )
                    )
                    is not None
                    else None
                ),
                text=native_page.text,
                markdown=native_page.markdown,
                text_items=text_items,
                complexity=(
                    _convert_complexity(native_complexity)
                    if native_complexity is not None
                    else None
                ),
                vector_graphics=(
                    VectorGraphics(
                        shapes=[
                            VectorShape(
                                bbox=(s.bbox.x, s.bbox.y, s.bbox.width, s.bbox.height),
                                stroke=s.stroke,
                                stroke_color=s.stroke_color,
                                fill=s.fill,
                                fill_color=s.fill_color,
                                has_curve=s.has_curve,
                            )
                            for s in native_vectors.shapes
                        ],
                        lines=[
                            VectorLine(
                                x1=l.x1,
                                y1=l.y1,
                                x2=l.x2,
                                y2=l.y2,
                                stroke=l.stroke,
                                stroke_width=l.stroke_width,
                                stroke_color=l.stroke_color,
                                fill=l.fill,
                                fill_color=l.fill_color,
                            )
                            for l in native_vectors.lines
                        ],
                    )
                    if native_vectors is not None
                    else None
                ),
                annotations=(
                    [
                        DocumentAnnotation(
                            subtype=annotation.subtype,
                            contents=annotation.contents,
                            created=annotation.created,
                            modified=annotation.modified,
                            title=annotation.title,
                            rect=(
                                AnnotationRect(
                                    x=annotation.rect.x,
                                    y=annotation.rect.y,
                                    width=annotation.rect.width,
                                    height=annotation.rect.height,
                                )
                                if annotation.rect is not None
                                else None
                            ),
                            quadpoint_rects=[
                                AnnotationRect(
                                    x=rect.x,
                                    y=rect.y,
                                    width=rect.width,
                                    height=rect.height,
                                )
                                for rect in annotation.quadpoint_rects
                            ],
                            uri=annotation.uri,
                        )
                        for annotation in native_annotations
                    ]
                    if native_annotations is not None
                    else None
                ),
                form_fields=(
                    [
                        FormField(
                            id=form.id,
                            type=form.field_type,
                            page=form.page,
                            annotation_index=form.annotation_index,
                            widget_index=form.widget_index,
                            object_number=form.object_number,
                            name=form.name,
                            alternate_name=form.alternate_name,
                            value=form.value,
                            export_value=form.export_value,
                            field_flags=form.field_flags,
                            control_count=form.control_count,
                            control_index=form.control_index,
                            checked=form.checked,
                            rect=(AnnotationRect(form.rect.x, form.rect.y, form.rect.width, form.rect.height) if form.rect is not None else None),
                            options=list(form.options),
                            selected_options=list(form.selected_options),
                        )
                        for form in native_form_fields
                    ]
                    if native_form_fields is not None
                    else None
                ),
                structure_tree=(
                    StructureTree(
                        roots=[
                            _convert_structure_element(root)
                            for root in native_structure_tree.roots
                        ]
                    )
                    if native_structure_tree is not None
                    else None
                ),
                blocks=(
                    [_convert_block(block) for block in native_blocks]
                    if native_blocks is not None
                    else None
                ),
            )
        )
    images = [
        ExtractedImage(
            id=img.id,
            name=img.name,
            path=img.path,
            page=img.page,
            bbox=ImageRect(
                x=img.bbox.x,
                y=img.bbox.y,
                width=img.bbox.width,
                height=img.bbox.height,
            ),
            width=img.width,
            height=img.height,
            rotation=img.rotation,
            format=img.format,
            bytes=img.bytes,
            duplicate_of=img.duplicate_of,
        )
        for img in getattr(native_result, "images", [])
    ]
    native_xfa_packets = getattr(native_result, "xfa_packets", None)
    native_doc_meta = getattr(native_result, "doc_meta", None)
    doc_meta = (
        None
        if native_doc_meta is None
        else DocumentMetadata(
            creation_date=getattr(native_doc_meta, "creation_date", None),
            mod_date=getattr(native_doc_meta, "mod_date", None),
            file_version=getattr(native_doc_meta, "file_version", None),
            is_encrypted=getattr(native_doc_meta, "is_encrypted", None),
            security_handler_revision=getattr(
                native_doc_meta, "security_handler_revision", None
            ),
            permissions=getattr(native_doc_meta, "permissions", None),
            eof_section_count=getattr(native_doc_meta, "eof_section_count", None),
            startxref_count=getattr(native_doc_meta, "startxref_count", None),
            trailer_id_pair_differs=getattr(
                native_doc_meta, "trailer_id_pair_differs", None
            ),
            raw_file_size=getattr(native_doc_meta, "raw_file_size", None),
            xmp=getattr(native_doc_meta, "xmp", None),
            xmp_truncated=getattr(native_doc_meta, "xmp_truncated", None),
            signature_count=getattr(native_doc_meta, "signature_count", None),
            signature_byte_range_reaches_eof=getattr(
                native_doc_meta, "signature_byte_range_reaches_eof", None
            ),
        )
    )
    return ParseResult(
        pages=pages,
        text=native_result.text,
        total_pages=getattr(native_result, "total_pages", len(pages)),
        images=images,
        screenshots=[
            ScreenshotResult(
                page_num=screenshot.page_num,
                width=screenshot.width,
                height=screenshot.height,
                image_bytes=screenshot.image_bytes,
                format=screenshot.format,
                stride=screenshot.stride,
                is_solid_fill=getattr(screenshot, "is_solid_fill", False),
                rects=[
                    ScreenshotRect(
                        x=rect.x,
                        y=rect.y,
                        width=rect.width,
                        height=rect.height,
                        color=rect.color,
                        is_line=rect.is_line,
                    )
                    for rect in getattr(screenshot, "rects", [])
                ],
            )
            for screenshot in getattr(native_result, "screenshots", [])
        ],
        image_error_count=getattr(native_result, "image_error_count", 0),
        page_errors=[
            PageError(page_num=error.page_num, message=error.message)
            for error in getattr(native_result, "page_errors", [])
        ],
        form_type=getattr(native_result, "form_type", None),
        creator=getattr(native_result, "creator", None),
        producer=getattr(native_result, "producer", None),
        doc_meta=doc_meta,
        xfa_packets=(
            [
                XfaPacket(
                    index=packet.index,
                    name=packet.name,
                    content_length=packet.content_length,
                    content=packet.content,
                )
                for packet in native_xfa_packets
            ]
            if native_xfa_packets is not None
            else None
        ),
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
        extract_screenshots: Optional[bool] = None,
        screenshot: Optional[ScreenshotOptions] = None,
        continue_on_page_error: Optional[bool] = None,
        dpi: Optional[float] = None,
        output_format: Optional[str] = None,
        preserve_very_small_text: Optional[bool] = None,
        password: Optional[str] = None,
        quiet: Optional[bool] = None,
        num_workers: Optional[int] = None,
        image_mode: Optional[str] = None,
        extract_images: Optional[bool] = None,
        image_output_dir: Optional[Union[str, Path]] = None,
        extract_links: Optional[bool] = None,
        keep_headers_footers: Optional[bool] = None,
        extract_annotations: Optional[bool] = None,
        extract_form_fields: Optional[bool] = None,
        extract_structure_tree: Optional[bool] = None,
        extract_blocks: Optional[bool] = None,
        extract_xfa_packets: Optional[bool] = None,
        extract_document_metadata: Optional[bool] = None,
        extract_content_bounds: Optional[bool] = None,
        detect_screenshot_rects: Optional[bool] = None,
        render_form_fields: Optional[bool] = None,
        ocr_failure_fatal: Optional[bool] = None,
        ocr_hedge_delays_ms: Optional[List[int]] = None,
        emit_word_boxes: Optional[bool] = None,
        extract_text_metadata: Optional[bool] = None,
        crop_box: Optional[Tuple[float, float, float, float]] = None,
        skip_diagonal_text: Optional[bool] = None,
        include_complexity: Optional[bool] = None,
        extract_vector_graphics: Optional[bool] = None,
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
            extract_screenshots: Render parsed pages and return them in
                ``ParseResult.screenshots``. Default False.
            screenshot: Output format and format-specific encoder options.
            continue_on_page_error: Skip page-level PDF extraction failures and
                return them in ``ParseResult.page_errors``. Document-level
                failures remain fatal. Default False.
            dpi: DPI for rendering (affects OCR quality)
            output_format: Output format: "json", "text", or "markdown" (default: "json")
            preserve_very_small_text: Whether to preserve very small text
            password: Password for encrypted/protected documents
            quiet: Suppress progress output
            num_workers: Number of concurrent OCR workers (default: CPU cores - 1)
            extract_links: Render hyperlink annotations as ``[text](url)`` in
                markdown output (default: True). Set False for plain anchor text.
            keep_headers_footers: Keep running headers/footers in markdown
                output instead of stripping repeated page-band lines and page
                chrome (default: False).
            image_output_dir: Directory where extracted embedded images are
                written. Requires ``extract_images=True`` and returns file
                names/paths in each ``ExtractedImage``.
            extract_images: Extract embedded image bytes and metadata. Defaults
                to False. This is the only option that enables extraction;
                ``image_mode`` controls Markdown presentation independently.
            extract_annotations: Include all PDF annotations as page-scoped
                structured data (default: False).
            extract_form_fields: Include AcroForm widget fields and values as
                page-scoped structured data (default: False).
            extract_blocks: Emit each page's classified layout blocks with
                bounding boxes. This is the same decomposition the Markdown
                renderer consumes, exposed as data; enabling it never changes
                the rendered Markdown.
            extract_structure_tree: Include the tagged-PDF logical structure
                tree as page-scoped structured data (default: False).
            ocr_failure_fatal: Whether a systemic OCR failure (every OCR task
                failed and at least one was a text-sparse page) aborts the whole
                parse (default: True). Set False to keep already-recovered native
                text and return partial results instead of raising — for callers
                that prefer a degraded document over a hard failure.
            ocr_hedge_delays_ms: Request-hedging schedule for HTTP OCR, in
                milliseconds. Empty or single-element means no hedging (one
                request per attempt — the default). With multiple delays (e.g.
                ``[0, 5000, 10000]``) each attempt fires a duplicate request at
                every delay and takes the first to succeed, cancelling the rest
                — trades extra OCR-server load for lower tail latency.
            emit_word_boxes: Emit per-word sub-boxes on each text item
                (``TextItem.words``). Default False. Word boxes roughly double
                the text-item payload, so enable only for word-level bbox
                attribution.
            extract_text_metadata: Include rich PDF text metadata on returned
                text items (MCID, glyph width, font metrics/weight/buggy state,
                colors, raw character codes, and ``trailing_space_generated``). Default False.
            crop_box: Restrict output to a page sub-region, as a
                ``(top, right, bottom, left)`` tuple where each value is the
                fraction cropped from that side (top-left origin, each in
                ``[0, 1]``); e.g. ``(0, 0, 0, 0.5)`` keeps the right half. A
                text item survives only if it lies entirely inside the
                remaining rectangle. None (default) keeps the whole page.
            skip_diagonal_text: Drop diagonal text — items whose rotation is
                more than 2° off the nearest right angle (0/90/180/270).
                Default False. Use to exclude rotated watermarks/stamps.
            include_complexity: Compute per-page complexity signals during
                :meth:`parse` and attach them to each page as
                ``ParsedPage.complexity`` (the same :meth:`is_complex` returns).
                Default False; enabling it runs an extra vector-text detection
                pass.
            extract_vector_graphics: Expose page-scoped vector shapes and
                merged horizontal/vertical line segments. Default False.
        """
        kwargs: Dict[str, Any] = {}
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
        if extract_screenshots is not None:
            kwargs["extract_screenshots"] = extract_screenshots
        if screenshot is not None:
            kwargs["screenshot"] = screenshot
        if continue_on_page_error is not None:
            kwargs["continue_on_page_error"] = continue_on_page_error
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
        if extract_images is not None:
            kwargs["extract_images"] = extract_images
        if image_output_dir is not None:
            kwargs["image_output_dir"] = str(image_output_dir)
        if extract_links is not None:
            kwargs["extract_links"] = extract_links
        if keep_headers_footers is not None:
            kwargs["keep_headers_footers"] = keep_headers_footers
        if extract_annotations is not None:
            kwargs["extract_annotations"] = extract_annotations
        if extract_form_fields is not None:
            kwargs["extract_form_fields"] = extract_form_fields
        if extract_structure_tree is not None:
            kwargs["extract_structure_tree"] = extract_structure_tree
        if extract_blocks is not None:
            kwargs["extract_blocks"] = extract_blocks
        if extract_xfa_packets is not None:
            kwargs["extract_xfa_packets"] = extract_xfa_packets
        if extract_document_metadata is not None:
            kwargs["extract_document_metadata"] = extract_document_metadata
        if extract_content_bounds is not None:
            kwargs["extract_content_bounds"] = extract_content_bounds
        if detect_screenshot_rects is not None:
            kwargs["detect_screenshot_rects"] = detect_screenshot_rects
        if render_form_fields is not None:
            kwargs["render_form_fields"] = render_form_fields
        if ocr_failure_fatal is not None:
            kwargs["ocr_failure_fatal"] = ocr_failure_fatal
        if ocr_hedge_delays_ms is not None:
            kwargs["ocr_hedge_delays_ms"] = ocr_hedge_delays_ms
        if emit_word_boxes is not None:
            kwargs["emit_word_boxes"] = emit_word_boxes
        if extract_text_metadata is not None:
            kwargs["extract_text_metadata"] = extract_text_metadata
        if crop_box is not None:
            kwargs["crop_box"] = crop_box
        if skip_diagonal_text is not None:
            kwargs["skip_diagonal_text"] = skip_diagonal_text
        if include_complexity is not None:
            kwargs["include_complexity"] = include_complexity
        if extract_vector_graphics is not None:
            kwargs["extract_vector_graphics"] = extract_vector_graphics

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

    def parse_batches(
        self,
        file_data: Union[str, Path, bytes],
        batch_size: Optional[int] = None,
    ) -> Iterator[ParseBatch]:
        """
        Parse a document in bounded-memory page batches.

        Each yielded batch is an ordinary :class:`ParseResult` covering
        ``batch.start_page`` through ``batch.end_page``, and becomes
        collectible as soon as you advance the iterator — so a loop that does
        not retain batches never holds more than one batch of pages in memory.
        A non-PDF source is converted once, not once per batch.

        Cross-page passes see only the pages in their own batch, so repeated
        header/footer removal and image deduplication are batch-local and the
        output can differ from :meth:`parse`. Prefer :meth:`parse` unless the
        size of the materialized result is the problem.

        Args:
            file_data: Path to the document file, or raw PDF bytes.
            batch_size: Pages materialized per batch (default 25).

        Yields:
            ParseBatch for each page range, in document order.

        Raises:
            ParseError: If parsing fails, or if the parser was constructed
                with ``target_pages`` (ambiguous with generated batch ranges).
                Open errors are raised here, when ``parse_batches`` is called;
                per-batch parse errors are raised from the iterator.
            FileNotFoundError: If the file doesn't exist.
        """
        # Validate and open eagerly — this is not a generator function, so a
        # missing file or a target_pages conflict raises here rather than on
        # the first iteration of the returned iterator.
        try:
            if isinstance(file_data, bytes):
                session = self._native.open_batch_session_bytes(
                    file_data, batch_size
                )
            else:
                file_path = Path(file_data)
                if not file_path.exists():
                    raise FileNotFoundError(f"File not found: {file_path}")
                session = self._native.open_batch_session(
                    str(file_path.absolute()), batch_size
                )
        except FileNotFoundError:
            raise
        except Exception as e:
            raise ParseError(str(e)) from e

        return self._iter_batches(session)

    @staticmethod
    def _iter_batches(session: Any) -> Iterator[ParseBatch]:
        total_pages = session.total_pages
        while True:
            try:
                batch = session.next_batch()
            except Exception as e:
                raise ParseError(str(e)) from e
            if batch is None:
                return
            yield ParseBatch(
                start_page=batch.start_page,
                end_page=batch.end_page,
                total_pages=total_pages,
                result=_convert_native_result(batch.result),
            )

    def is_complex(
        self,
        file_data: Union[str, Path, bytes],
    ) -> List[PageComplexityStats]:
        """
        Determine per-page complexity without running a full parse.

        Returns one entry per page with signals (text coverage, images, garbled
        text, vector area) and a ``needs_ocr`` verdict — a cheap pre-OCR check to
        decide whether a document needs advanced parsing.

        Args:
            file_data: Path to the document file, or raw PDF bytes.

        Returns:
            List of PageComplexityStats, one per page.

        Raises:
            ParseError: If the check fails.
            FileNotFoundError: If the file doesn't exist.
        """
        try:
            if isinstance(file_data, bytes):
                native_stats = self._native.is_complex_bytes(file_data)
            else:
                file_path = Path(file_data)
                if not file_path.exists():
                    raise FileNotFoundError(f"File not found: {file_path}")
                native_stats = self._native.is_complex(str(file_path.absolute()))
            return [_convert_complexity(s) for s in native_stats]
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
            List of ScreenshotResult values in the configured format.

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
                    format=r.format,
                    stride=r.stride,
                    is_solid_fill=getattr(r, "is_solid_fill", False),
                    rects=[
                        ScreenshotRect(
                            x=rect.x,
                            y=rect.y,
                            width=rect.width,
                            height=rect.height,
                            color=rect.color,
                            is_line=rect.is_line,
                        )
                        for rect in getattr(r, "rects", [])
                    ],
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
            extract_screenshots=cfg.extract_screenshots,
            screenshot={
                "format": cfg.screenshot.format,
                "png": {"compression": cfg.screenshot.png.compression},
            },
            continue_on_page_error=cfg.continue_on_page_error,
            dpi=cfg.dpi,
            output_format=cfg.output_format,
            preserve_very_small_text=cfg.preserve_very_small_text,
            password=cfg.password,
            quiet=cfg.quiet,
            num_workers=cfg.num_workers,
            image_mode=cfg.image_mode,
            image_output_dir=cfg.image_output_dir,
            extract_links=cfg.extract_links,
            keep_headers_footers=cfg.keep_headers_footers,
            extract_annotations=cfg.extract_annotations,
            extract_form_fields=cfg.extract_form_fields,
            extract_structure_tree=cfg.extract_structure_tree,
            extract_blocks=cfg.extract_blocks,
            ocr_failure_fatal=cfg.ocr_failure_fatal,
            ocr_hedge_delays_ms=list(cfg.ocr_hedge_delays_ms),
            emit_word_boxes=cfg.emit_word_boxes,
            crop_box=cfg.crop_box,
            skip_diagonal_text=cfg.skip_diagonal_text,
            include_complexity=cfg.include_complexity,
            extract_text_metadata=cfg.extract_text_metadata,
            extract_images=cfg.extract_images,
            extract_vector_graphics=cfg.extract_vector_graphics,
            extract_xfa_packets=cfg.extract_xfa_packets,
            extract_document_metadata=cfg.extract_document_metadata,
            extract_content_bounds=cfg.extract_content_bounds,
            detect_screenshot_rects=cfg.detect_screenshot_rects,
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
