from importlib.metadata import PackageNotFoundError, version

from .parser import LiteParse, search_items
from .types import (
    ExtractedImage,
    LiteParseConfig,
    OcrFailure,
    PageComplexityStats,
    ParseResult,
    ParsedPage,
    TextItem,
    WordBox,
    ScreenshotResult,
    ParseError,
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
    "OcrFailure",
    "ExtractedImage",
    "ParseError",
    "search_items",
]
