"""Shared fixtures for e2e tests."""

from pathlib import Path

import pytest

from liteparse import LiteParse

# Resolve test-docs relative to the repo root
REPO_ROOT = Path(__file__).resolve().parents[3]
TEST_DOCS = REPO_ROOT / "test-docs"
EXPECTED_DATASET = REPO_ROOT / "expected-dataset" / "data"


@pytest.fixture
def parser() -> LiteParse:
    """LiteParse instance (auto-detects CLI)."""
    return LiteParse()


@pytest.fixture
def invoice_pdf() -> Path:
    """Path to a small single-page invoice PDF."""
    p = TEST_DOCS / "invoice.pdf"
    if not p.exists():
        pytest.skip(f"Test document not found: {p}")
    return p


@pytest.fixture
def two_page_pdf() -> Path:
    """Path to a 2-page PDF."""
    p = EXPECTED_DATASET / "2pages.pdf"
    if not p.exists():
        pytest.skip(f"Test document not found: {p}")
    return p


@pytest.fixture
def empty_pdf() -> Path:
    """Path to an empty PDF."""
    p = EXPECTED_DATASET / "empty.pdf"
    if not p.exists():
        pytest.skip(f"Test document not found: {p}")
    return p
