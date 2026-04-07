"""pypdf-compatible PdfReader powered by LiteParse.

Text extraction uses LiteParse for higher quality spatial reconstruction.
All other PDF features (metadata, images, annotations, outlines, form fields,
etc.) are delegated to the real pypdf library.
"""

from __future__ import annotations

import io
import tempfile
from pathlib import Path
from typing import IO, Any, Dict, List, Optional, Union

import pypdf

from liteparse import LiteParse, ParseResult, ParsedPage
from ._page import PageObject


class PdfReader:
    """
    Drop-in replacement for ``pypdf.PdfReader`` backed by LiteParse.

    Text extraction (``page.extract_text()``) is powered by LiteParse's
    spatial reconstruction engine for higher quality output.  Everything else
    — metadata, images, annotations, outlines, form fields, etc. — is
    delegated to the real ``pypdf.PdfReader``.

    Usage::

        # Before
        from pypdf import PdfReader

        # After
        from liteparse_pypdf import PdfReader

    Args:
        stream: A file path (str/Path), file-like object, or bytes.
        password: Password for encrypted PDFs.
        strict: Passed through to pypdf.
    """

    def __init__(
        self,
        stream: Union[str, Path, IO[bytes], bytes],
        password: Optional[str] = None,
        strict: bool = False,
        *,
        # LiteParse-specific options
        ocr_enabled: bool = True,
        ocr_server_url: Optional[str] = None,
        ocr_language: str = "en",
        dpi: int = 150,
        precise_bounding_box: bool = True,
    ) -> None:
        self._password = password
        self._ocr_enabled = ocr_enabled
        self._ocr_server_url = ocr_server_url
        self._ocr_language = ocr_language
        self._dpi = dpi
        self._precise_bounding_box = precise_bounding_box

        # Resolve input to a file path for liteparse and keep the original
        # stream available for pypdf.
        self._tmp_file: Optional[tempfile.NamedTemporaryFile] = None  # type: ignore[type-arg]
        file_path, pypdf_stream = self._resolve_input(stream)

        # --- pypdf fallback reader (for metadata, images, annotations, …) ---
        self._pypdf_reader = pypdf.PdfReader(
            pypdf_stream, password=password, strict=strict
        )

        # --- LiteParse parse (for text extraction) ---
        parser = LiteParse()
        self._result: ParseResult = parser.parse(
            file_path,
            ocr_enabled=self._ocr_enabled,
            ocr_server_url=self._ocr_server_url,
            ocr_language=self._ocr_language,
            dpi=self._dpi,
            precise_bounding_box=self._precise_bounding_box,
        )

        # Build page objects combining liteparse text + pypdf page data
        self._pages: List[PageObject] = []
        for idx, lp_page in enumerate(self._result.pages):
            pypdf_page = (
                self._pypdf_reader.pages[idx]
                if idx < len(self._pypdf_reader.pages)
                else None
            )
            self._pages.append(PageObject(lp_page, pypdf_page=pypdf_page))

    # ------------------------------------------------------------------
    # pypdf-compatible public API
    # ------------------------------------------------------------------

    @property
    def pages(self) -> List[PageObject]:
        """List of :class:`PageObject` instances (0-indexed)."""
        return self._pages

    @property
    def metadata(self) -> Optional[pypdf.DocumentInformation]:
        """Document metadata — delegated to pypdf."""
        return self._pypdf_reader.metadata

    @property
    def is_encrypted(self) -> bool:
        """Whether the PDF is encrypted."""
        return self._pypdf_reader.is_encrypted

    @property
    def pdf_header(self) -> str:
        """PDF version header string."""
        return self._pypdf_reader.pdf_header

    @property
    def outline(self) -> List[Any]:
        """Document outline (bookmarks) — delegated to pypdf."""
        return self._pypdf_reader.outline  # type: ignore[return-value]

    @property
    def named_destinations(self) -> Dict[str, Any]:
        """Named destinations — delegated to pypdf."""
        return self._pypdf_reader.named_destinations  # type: ignore[return-value]

    @property
    def page_labels(self) -> List[str]:
        """Page labels — delegated to pypdf."""
        return self._pypdf_reader.page_labels  # type: ignore[return-value]

    @property
    def page_layout(self) -> Optional[str]:
        """Page layout setting — delegated to pypdf."""
        return self._pypdf_reader.page_layout

    @property
    def page_mode(self) -> Optional[str]:
        """Page mode setting — delegated to pypdf."""
        return self._pypdf_reader.page_mode

    @property
    def attachments(self) -> Dict[str, List[bytes]]:
        """File attachments — delegated to pypdf."""
        return self._pypdf_reader.attachments

    @property
    def xfa(self) -> Optional[Dict[str, Any]]:
        """XFA form data — delegated to pypdf."""
        return self._pypdf_reader.xfa

    def get_fields(
        self, tree: Any = None, retval: Any = None, fileobj: Any = None
    ) -> Optional[Dict[str, Any]]:
        """Extract form field data — delegated to pypdf."""
        return self._pypdf_reader.get_fields(tree, retval, fileobj)

    def get_form_text_fields(self) -> Optional[Dict[str, Any]]:
        """Extract text form fields — delegated to pypdf."""
        return self._pypdf_reader.get_form_text_fields()

    def get_num_pages(self) -> int:
        """Return number of pages (legacy helper, prefer ``len(reader.pages)``)."""
        return len(self._pages)

    @property
    def numPages(self) -> int:  # noqa: N802
        """Number of pages (PyPDF2 compatibility alias)."""
        return len(self._pages)

    def getPage(self, page_number: int) -> PageObject:  # noqa: N802
        """Get page by index (PyPDF2 compatibility alias)."""
        return self._pages[page_number]

    def get_page(self, page_number: int) -> PageObject:
        """Get page by 0-based index."""
        return self._pages[page_number]

    # ------------------------------------------------------------------
    # Internals
    # ------------------------------------------------------------------

    def _resolve_input(
        self, stream: Union[str, Path, IO[bytes], bytes]
    ) -> tuple[Path, Union[str, bytes]]:
        """Convert various input types to (file_path, pypdf_stream)."""
        if isinstance(stream, (str, Path)):
            p = Path(stream)
            return p, str(p)

        # bytes or file-like → write to temp file for liteparse
        if isinstance(stream, bytes):
            data = stream
        elif isinstance(stream, io.IOBase) or hasattr(stream, "read"):
            data = stream.read()  # type: ignore[union-attr]
        else:
            raise TypeError(f"Unsupported stream type: {type(stream)}")

        tmp = tempfile.NamedTemporaryFile(suffix=".pdf", delete=False)
        tmp.write(data)
        tmp.flush()
        self._tmp_file = tmp
        return Path(tmp.name), tmp.name

    def close(self) -> None:
        """Clean up temporary files."""
        if self._tmp_file is not None:
            try:
                Path(self._tmp_file.name).unlink(missing_ok=True)
            except OSError:
                pass
            self._tmp_file = None

    def __del__(self) -> None:
        self.close()

    def __enter__(self) -> "PdfReader":
        return self

    def __exit__(self, *args: Any) -> None:
        self.close()

    def __len__(self) -> int:
        return len(self._pages)

    def __repr__(self) -> str:
        return f"PdfReader(pages={len(self._pages)})"
