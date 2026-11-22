"""Optional model packages (``lit models``): LiteParse's own OCR and table models.

    from liteparse import models
    models.pull("ocria-nano")          # once; prints the model licence notice
    LiteParse(ocr_backend="ocria-nano").parse("scan.pdf")
"""

from dataclasses import dataclass
from typing import List, Optional

from liteparse import _liteparse as _native


@dataclass
class InstalledModel:
    name: str
    version: Optional[str]
    path: str
    size_bytes: int
    #: Every file of the package is present (else pull again).
    complete: bool


def pull(spec: str = "ocria-nano", *, from_dir: Optional[str] = None, quiet: bool = False) -> str:
    """Download and verify a model package (``"ocria-nano"`` or ``"ocria-nano@<version>"``).

    ``from_dir`` installs from a local directory holding the package files instead.
    Returns the installed directory."""
    return _native.models_pull(spec, from_dir=from_dir, quiet=quiet)


def list() -> List[InstalledModel]:  # noqa: A001 - mirrors `lit models list`
    """Installed model packages."""
    return [InstalledModel(*m) for m in _native.models_list()]


def rm(name: str) -> Optional[str]:
    """Remove an installed model package; returns its directory, or None if not installed."""
    return _native.models_rm(name)
