from pathlib import Path

import pytest

from liteparse import LiteParse


@pytest.mark.parametrize(
    "options, expected",
    [
        ({}, "quote 'word' and \"word\"; range 1-2 - done"),
        ({"normalize_punctuation": False}, "quote ‘word’ and “word”; range 1–2 — done"),
    ],
)
def test_native_punctuation(options, expected):
    fixture = (
        Path(__file__).resolve().parents[3]
        / "crates/liteparse/tests/fixtures/typographic-punctuation.pdf"
    )
    with LiteParse(ocr_enabled=False, quiet=True, **options) as parser:
        assert parser.get_config().normalize_punctuation == options.get(
            "normalize_punctuation", True
        )
        assert parser.parse(fixture).pages[0].text.strip() == expected
