"""E2E tests for pool mode (process-isolated workers with hard timeouts)."""

import math
import zlib
from pathlib import Path

import pytest

from liteparse import LiteParse, ParseError, ParseTimeoutError

REPO_ROOT = Path(__file__).resolve().parents[3]
SAMPLE_PDF = REPO_ROOT / "integration_tests_data" / "sample.pdf"

CFG = dict(quiet=True, ocr_enabled=False, output_format="text")


def _grid_pdf(n_objects: int) -> bytes:
    """One page with a dense grid of tiny text objects.

    Parse time is superlinear in the object count (PDFium's text-page
    assembly), which makes a small file arbitrarily slow to parse — the same
    shape as the rogue documents that stall prod pipelines, used here to
    exercise the timeout path.
    """
    cols = int(math.sqrt(n_objects)) or 1
    parts = [
        f"BT /F1 2 Tf {20 + (i % cols) * 2.5:.1f} "
        f"{770 - (i // cols) * 2.5:.1f} Td (x) Tj ET"
        for i in range(n_objects)
    ]
    stream = zlib.compress("\n".join(parts).encode())
    width = int(40 + cols * 2.5)
    height = int(40 + (n_objects // cols) * 2.5)
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 %d %d] "
        b"/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
        % (width, height),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        b"<< /Length %d /Filter /FlateDecode >>\nstream\n%s\nendstream"
        % (len(stream), stream),
    ]
    out = bytearray(b"%PDF-1.4\n")
    offsets = []
    for i, body in enumerate(objects, start=1):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % i + body + b"\nendobj\n"
    xref_pos = len(out)
    out += b"xref\n0 %d\n" % (len(objects) + 1)
    out += b"0000000000 65535 f \n"
    for off in offsets:
        out += b"%010d 00000 n \n" % off
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (
        len(objects) + 1,
        xref_pos,
    )
    return bytes(out)


@pytest.fixture
def sample_pdf() -> Path:
    if not SAMPLE_PDF.exists():
        pytest.skip(f"Test document not found: {SAMPLE_PDF}")
    return SAMPLE_PDF


def test_pool_matches_in_process(sample_pdf):
    expected = LiteParse(**CFG).parse(sample_pdf)
    with LiteParse(**CFG, pool_size=2) as pooled:
        from_path = pooled.parse(sample_pdf)
        from_bytes = pooled.parse(sample_pdf.read_bytes())
    assert from_path.text == expected.text
    assert from_bytes.text == expected.text
    assert len(from_path.pages) == len(expected.pages)


def test_parse_timeout_requires_pool():
    with pytest.raises(ValueError, match="pool_size"):
        LiteParse(parse_timeout=5)


def test_timeout_kills_worker_and_pool_recovers(sample_pdf):
    slow_doc = _grid_pdf(64000)  # parses in seconds, not before the deadline
    with LiteParse(**CFG, pool_size=1, parse_timeout=0.5) as pooled:
        pooled.warm_up()
        with pytest.raises(ParseTimeoutError) as excinfo:
            pooled.parse(slow_doc)
        assert excinfo.value.timeout == 0.5
        assert "bytes" in excinfo.value.source
        # The replacement worker must serve subsequent parses.
        result = pooled.parse(sample_pdf)
        assert result.text


def test_timeout_names_the_file(tmp_path):
    slow_path = tmp_path / "rogue.pdf"
    slow_path.write_bytes(_grid_pdf(64000))
    # The doc parses in ~0.35s, so a 0.5s budget only ever tripped because
    # worker spawn + `import liteparse` were charged to it. Pick a timeout the
    # parse itself blows through, so the assertion does not depend on how slow
    # process startup happens to be on the machine running the suite.
    with LiteParse(**CFG, pool_size=1, parse_timeout=0.1) as pooled:
        with pytest.raises(ParseTimeoutError) as excinfo:
            pooled.parse(slow_path)
    assert excinfo.value.source == str(slow_path.absolute())


def test_parse_error_propagates_from_worker():
    with LiteParse(**CFG, pool_size=1) as pooled:
        with pytest.raises(ParseError):
            pooled.parse(b"%PDF-1.4 not actually a pdf")


def test_close_shuts_down_workers(sample_pdf):
    pooled = LiteParse(**CFG, pool_size=2)
    pooled.parse(sample_pdf)
    pooled.close()
    workers = list(pooled._pool._workers)
    assert workers == []
    with pytest.raises(ParseError, match="closed"):
        pooled.parse(sample_pdf)
    pooled.close()  # idempotent
