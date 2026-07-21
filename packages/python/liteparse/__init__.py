from importlib.metadata import PackageNotFoundError, version

from .parser import LiteParse, search_items
from .types import (
    ExtractedImage,
    LayoutComplexityStats,
    LiteParseConfig,
    PageComplexityStats,
    ParseResult,
    ParsedPage,
    TextItem,
    WordBox,
    ScreenshotResult,
    ParseError,
    VectorGraphics,
    VectorLine,
    VectorShape,
)

try:
    __version__ = version("liteparse")
except PackageNotFoundError:  # source tree without installed dist metadata
    __version__ = "0.0.0+unknown"
__all__ = [
    "LiteParse",
    "LiteParseConfig",
    "ParseResult",
    "ParsedPage",
    "TextItem",
    "WordBox",
    "ScreenshotResult",
    "PageComplexityStats",
    "LayoutComplexityStats",
    "ExtractedImage",
    "ParseError",
    "search_items",
    "VectorGraphics",
    "VectorLine",
    "VectorShape",
]
