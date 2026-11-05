"""The public Python API exposes the PDF's document-level outline."""

from pathlib import Path
import json
import subprocess
import sys

from liteparse import LiteParse, OutlineTarget


FIXTURE = Path(__file__).resolve().parents[3] / "integration_tests_data" / "outline.pdf"


def test_outline_is_available_for_path_bytes_and_page_selection():
    parser = LiteParse(ocr_enabled=False, quiet=True)
    result = parser.parse(FIXTURE)
    assert all(isinstance(entry, OutlineTarget) for entry in result.outline)
    assert [(item.level, item.title, item.page_index) for item in result.outline] == [
        (1, "Chapter 1", 0),
        (2, "Section 1.1", 0),
        (1, "Chapter 2", 1),
    ]
    assert all(item.y_pdf is None or isinstance(item.y_pdf, float) for item in result.outline)
    assert parser.parse(FIXTURE.read_bytes()).outline == result.outline

    selected = LiteParse(ocr_enabled=False, quiet=True, target_pages="1").parse(FIXTURE)
    assert len(selected.pages) == 1
    assert selected.outline == result.outline


def test_every_batch_contains_the_document_outline():
    parser = LiteParse(ocr_enabled=False, quiet=True)
    batches = list(parser.parse_batches(FIXTURE, batch_size=1))
    assert len(batches) == 2
    assert batches[0].result.outline == batches[1].result.outline
    assert [item.title for item in batches[0].result.outline] == [
        "Chapter 1", "Section 1.1", "Chapter 2"
    ]


def test_missing_outline_is_an_empty_list():
    parser = LiteParse(ocr_enabled=False, quiet=True)
    assert parser.parse(FIXTURE.with_name("no_outline.pdf")).outline == []


def test_cli_json_includes_the_outline():
    completed = subprocess.run(
        [sys.executable, "-m", "liteparse.cli", "parse", str(FIXTURE),
         "--format", "json", "--no-ocr", "--quiet"],
        capture_output=True, text=True, encoding="utf-8", check=True,
    )
    outline = json.loads(completed.stdout)["outline"]
    assert [(item["level"], item["title"], item["page_index"]) for item in outline] == [
        (1, "Chapter 1", 0), (2, "Section 1.1", 0), (1, "Chapter 2", 1)
    ]
    assert "y_pdf" in outline[0]
