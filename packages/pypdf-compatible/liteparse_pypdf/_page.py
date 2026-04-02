"""pypdf-compatible PageObject backed by LiteParse + pypdf fallback."""

from __future__ import annotations

from decimal import Decimal
from typing import Any, Callable, Dict, List, Optional, Tuple

import pypdf
import pypdf.generic

from liteparse.types import ParsedPage, TextItem


class PageObject:
    """
    Drop-in replacement for ``pypdf.PageObject``.

    ``extract_text()`` uses the LiteParse result for higher quality spatial
    text extraction.  All other properties and methods are delegated to the
    underlying ``pypdf.PageObject`` when available.
    """

    def __init__(
        self,
        parsed_page: ParsedPage,
        pypdf_page: Optional[pypdf.PageObject] = None,
    ) -> None:
        self._page = parsed_page
        self._pypdf_page = pypdf_page

    # ------------------------------------------------------------------
    # pypdf-compatible properties (delegated to pypdf when available)
    # ------------------------------------------------------------------

    @property
    def page_number(self) -> int:
        """0-based page index (pypdf convention)."""
        if self._pypdf_page is not None:
            return self._pypdf_page.page_number  # type: ignore[return-value]
        return self._page.pageNum - 1

    @property
    def mediabox(self) -> Any:
        """Page media box."""
        if self._pypdf_page is not None:
            return self._pypdf_page.mediabox
        return _RectangleObject(0, 0, self._page.width, self._page.height)

    @property
    def cropbox(self) -> Any:
        """Page crop box."""
        if self._pypdf_page is not None:
            return self._pypdf_page.cropbox
        return self.mediabox

    @property
    def trimbox(self) -> Any:
        """Page trim box."""
        if self._pypdf_page is not None:
            return self._pypdf_page.trimbox
        return self.mediabox

    @property
    def artbox(self) -> Any:
        """Page art box — delegated to pypdf."""
        if self._pypdf_page is not None:
            return self._pypdf_page.artbox
        return self.mediabox

    @property
    def bleedbox(self) -> Any:
        """Page bleed box — delegated to pypdf."""
        if self._pypdf_page is not None:
            return self._pypdf_page.bleedbox
        return self.mediabox

    @property
    def width(self) -> Decimal:
        """Page width in PDF points."""
        if self._pypdf_page is not None:
            return Decimal(str(self._pypdf_page.mediabox.width))
        return Decimal(str(self._page.width))

    @property
    def height(self) -> Decimal:
        """Page height in PDF points."""
        if self._pypdf_page is not None:
            return Decimal(str(self._pypdf_page.mediabox.height))
        return Decimal(str(self._page.height))

    @property
    def rotation(self) -> int:
        """Page rotation in degrees."""
        if self._pypdf_page is not None:
            return self._pypdf_page.rotation
        return 0

    @property
    def user_unit(self) -> float:
        """UserUnit scaling factor."""
        if self._pypdf_page is not None:
            return self._pypdf_page.user_unit
        return 1.0

    @property
    def images(self) -> Any:
        """Images on the page — delegated to pypdf."""
        if self._pypdf_page is not None:
            return self._pypdf_page.images
        return []

    @property
    def annotations(self) -> Optional[Any]:
        """Page annotations — delegated to pypdf."""
        if self._pypdf_page is not None:
            return self._pypdf_page.annotations
        return None

    # ------------------------------------------------------------------
    # Text extraction — always uses LiteParse
    # ------------------------------------------------------------------

    def extract_text(
        self,
        *args: Any,
        extraction_mode: str = "plain",
        visitor_text: Optional[Callable[..., None]] = None,
        **kwargs: Any,
    ) -> str:
        """
        Extract text from the page using LiteParse's spatial extraction.

        This always uses LiteParse regardless of ``extraction_mode``.
        The ``visitor_text`` callback is supported with the signature
        ``(text, None, None, None, None) -> None`` — transformation matrices
        are not available from LiteParse.

        Args:
            extraction_mode: Accepted for compatibility; both "plain" and
                "layout" map to LiteParse's spatial extraction.
            visitor_text: Optional callback invoked per text item.

        Returns:
            The page text as a string.
        """
        if visitor_text is not None:
            for item in self._page.textItems:
                visitor_text(item.str, None, None, None, None)

        return self._page.text

    def extractText(self, *args: Any, **kwargs: Any) -> str:  # noqa: N802
        """Legacy PyPDF2 method name."""
        return self.extract_text(*args, **kwargs)

    # ------------------------------------------------------------------
    # Page manipulation — delegated to pypdf
    # ------------------------------------------------------------------

    def add_transformation(self, ctm: Any) -> None:
        """Apply a transformation matrix — delegated to pypdf."""
        if self._pypdf_page is not None:
            self._pypdf_page.add_transformation(ctm)
        else:
            raise NotImplementedError("No underlying pypdf page available")

    def merge_page(self, page2: Any, over: bool = True) -> None:
        """Merge another page on top/below — delegated to pypdf."""
        if self._pypdf_page is not None:
            other = page2._pypdf_page if isinstance(page2, PageObject) else page2
            self._pypdf_page.merge_page(other, over=over)
        else:
            raise NotImplementedError("No underlying pypdf page available")

    def scale(self, sx: float, sy: float) -> None:
        """Scale page — delegated to pypdf."""
        if self._pypdf_page is not None:
            self._pypdf_page.scale(sx, sy)
        else:
            raise NotImplementedError("No underlying pypdf page available")

    def scale_by(self, factor: float) -> None:
        """Scale page uniformly — delegated to pypdf."""
        if self._pypdf_page is not None:
            self._pypdf_page.scale_by(factor)
        else:
            raise NotImplementedError("No underlying pypdf page available")

    def scale_to(self, width: float, height: float) -> None:
        """Scale page to exact size — delegated to pypdf."""
        if self._pypdf_page is not None:
            self._pypdf_page.scale_to(width, height)
        else:
            raise NotImplementedError("No underlying pypdf page available")

    def rotate(self, angle: int) -> "PageObject":
        """Rotate page — delegated to pypdf."""
        if self._pypdf_page is not None:
            self._pypdf_page.rotate(angle)
            return self
        raise NotImplementedError("No underlying pypdf page available")

    def transfer_rotation_to_content(self) -> None:
        """Transfer rotation to content stream — delegated to pypdf."""
        if self._pypdf_page is not None:
            self._pypdf_page.transfer_rotation_to_content()
        else:
            raise NotImplementedError("No underlying pypdf page available")

    # ------------------------------------------------------------------
    # Dict-like access to PDF page dictionary — delegated to pypdf
    # ------------------------------------------------------------------

    def __getitem__(self, key: str) -> Any:
        if self._pypdf_page is not None:
            return self._pypdf_page[key]
        raise KeyError(key)

    def get(self, key: str, default: Any = None) -> Any:
        if self._pypdf_page is not None:
            return self._pypdf_page.get(key, default)
        return default

    # ------------------------------------------------------------------
    # LiteParse extras (not in pypdf, but useful)
    # ------------------------------------------------------------------

    @property
    def text_items(self) -> List[TextItem]:
        """Access the underlying LiteParse text items with coordinates."""
        return self._page.textItems

    @property
    def bounding_boxes(self) -> list:  # type: ignore[type-arg]
        """Access the underlying LiteParse bounding boxes."""
        return self._page.boundingBoxes

    # ------------------------------------------------------------------
    # Dunder helpers
    # ------------------------------------------------------------------

    def __repr__(self) -> str:
        return f"PageObject(page_number={self.page_number})"


class _RectangleObject:
    """Minimal rectangle fallback when no pypdf page is available."""

    def __init__(self, x1: float, y1: float, x2: float, y2: float) -> None:
        self._coords = (
            Decimal(str(x1)),
            Decimal(str(y1)),
            Decimal(str(x2)),
            Decimal(str(y2)),
        )

    @property
    def left(self) -> Decimal:
        return self._coords[0]

    @property
    def bottom(self) -> Decimal:
        return self._coords[1]

    @property
    def right(self) -> Decimal:
        return self._coords[2]

    @property
    def top(self) -> Decimal:
        return self._coords[3]

    @property
    def width(self) -> Decimal:
        return self._coords[2] - self._coords[0]

    @property
    def height(self) -> Decimal:
        return self._coords[3] - self._coords[1]

    def __getitem__(self, index: int) -> Decimal:
        return self._coords[index]

    def __len__(self) -> int:
        return 4

    def __repr__(self) -> str:
        return f"RectangleObject([{self.left}, {self.bottom}, {self.right}, {self.top}])"
