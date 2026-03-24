"""E2E tests for LiteParse.batch_parse() — validates Python types match CLI output."""

import tempfile
from pathlib import Path

import pytest

from liteparse import (
    BatchResult,
    LiteParse,
    OutputFormat,
)


class TestBatchParseBasic:
    """Basic batch parsing functionality."""

    def test_batch_parse_returns_batch_result(
        self, parser: LiteParse, invoice_pdf: Path
    ):
        input_dir = invoice_pdf.parent
        with tempfile.TemporaryDirectory() as tmpdir:
            result = parser.batch_parse(
                input_dir,
                tmpdir,
                ocr_enabled=False,
                extension_filter=".pdf",
            )
            assert isinstance(result, BatchResult)
            assert result.output_dir == tmpdir

    def test_batch_parse_creates_output_files(
        self, parser: LiteParse, invoice_pdf: Path
    ):
        input_dir = invoice_pdf.parent
        with tempfile.TemporaryDirectory() as tmpdir:
            parser.batch_parse(
                input_dir,
                tmpdir,
                output_format=OutputFormat.TEXT,
                ocr_enabled=False,
                extension_filter=".pdf",
            )
            output_files = list(Path(tmpdir).rglob("*"))
            # Should have produced at least one output file
            assert len(output_files) > 0

    def test_batch_parse_json_format(self, parser: LiteParse, invoice_pdf: Path):
        input_dir = invoice_pdf.parent
        with tempfile.TemporaryDirectory() as tmpdir:
            parser.batch_parse(
                input_dir,
                tmpdir,
                output_format="json",
                ocr_enabled=False,
                extension_filter=".pdf",
            )
            json_files = list(Path(tmpdir).rglob("*.json"))
            assert len(json_files) > 0

    @pytest.mark.asyncio
    async def test_batch_parse_async(self, parser: LiteParse, invoice_pdf: Path):
        input_dir = invoice_pdf.parent
        with tempfile.TemporaryDirectory() as tmpdir:
            result = await parser.batch_parse_async(
                input_dir,
                tmpdir,
                output_format="json",
                ocr_enabled=False,
                extension_filter=".pdf",
            )
            assert isinstance(result, BatchResult)
            assert result.output_dir == tmpdir


class TestBatchParseErrors:
    """Test error handling."""

    def test_input_dir_not_found(self, parser: LiteParse):
        with tempfile.TemporaryDirectory() as tmpdir:
            with pytest.raises(FileNotFoundError):
                parser.batch_parse("/nonexistent/dir", tmpdir)
