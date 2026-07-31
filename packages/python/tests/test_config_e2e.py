"""E2E tests for LiteParse.get_config() — validates the resolved config mirrors the native config."""

import inspect
from dataclasses import fields

from liteparse import LiteParse
from liteparse._liteparse import PyLiteParseConfig
from liteparse.types import LiteParseConfig

NATIVE_CONFIG_FIELDS = {
    name for name in dir(PyLiteParseConfig) if not name.startswith("_")
}

NON_DEFAULT_OPTIONS = {
    "ocr_enabled": True,
    "ocr_server_url": "http://ocr.example/v1",
    "ocr_server_headers": {"Authorization": "Bearer token"},
    "ocr_language": "fra",
    "tessdata_path": "tessdata",
    "max_pages": 7,
    "target_pages": "2-4",
    "dpi": 222.0,
    "output_format": "markdown",
    "preserve_very_small_text": True,
    "password": "s3cret",
    "quiet": True,
    "num_workers": 3,
    "image_mode": "embed",
    "extract_images": True,
    "image_output_dir": "out/images",
    "extract_links": False,
    "keep_headers_footers": True,
    "extract_annotations": True,
    "extract_form_fields": True,
    "extract_structure_tree": True,
    "extract_xfa_packets": True,
    "extract_content_bounds": True,
    "detect_screenshot_rects": True,
    "render_form_fields": True,
    "ocr_failure_fatal": False,
    "ocr_hedge_delays_ms": [0, 500],
    "emit_word_boxes": True,
    "extract_text_metadata": True,
    "crop_box": (0.0, 0.25, 0.5, 0.125),
    "skip_diagonal_text": True,
    "include_complexity": True,
    "extract_vector_graphics": True,
}


class TestConfigSurfaceParity:
    """The Python config surfaces stay in step with the native config."""

    def test_every_python_config_surface_declares_the_native_fields(self):
        assert {f.name for f in fields(LiteParseConfig)} == NATIVE_CONFIG_FIELDS

        constructor_options = set(inspect.signature(LiteParse.__init__).parameters)
        assert constructor_options - {"self"} == NATIVE_CONFIG_FIELDS

        assert set(NON_DEFAULT_OPTIONS) == NATIVE_CONFIG_FIELDS

    def test_get_config_reports_every_constructor_option(self):
        config = LiteParse(**NON_DEFAULT_OPTIONS).get_config()

        reported = {f.name: getattr(config, f.name) for f in fields(LiteParseConfig)}
        assert reported == NON_DEFAULT_OPTIONS
