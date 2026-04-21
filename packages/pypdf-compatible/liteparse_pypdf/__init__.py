"""liteparse-pypdf-compatible: drop-in pypdf replacement powered by LiteParse.

Text extraction uses LiteParse for higher quality spatial reconstruction.
All other PDF features are delegated to the real pypdf library.
"""

from ._reader import PdfReader
from ._page import PageObject

# Re-export commonly imported pypdf classes so users don't need a
# separate ``import pypdf`` for these.
from pypdf import PdfWriter, DocumentInformation
from pypdf.errors import (
    PdfReadError,
    PdfStreamError,
    PageSizeNotDefinedError,
    FileNotDecryptedError,
    PdfReadWarning,
)

__version__ = "0.1.0"
__all__ = [
    # LiteParse-enhanced
    "PdfReader",
    "PageObject",
    # Delegated to pypdf
    "PdfWriter",
    "DocumentInformation",
    # Errors
    "PdfReadError",
    "PdfStreamError",
    "PageSizeNotDefinedError",
    "FileNotDecryptedError",
    "PdfReadWarning",
]
