"""E2E tests for LiteParse.get_config() — validates the resolved config mirrors the native config."""

from liteparse import LiteParse


class TestGetConfig:
    """Constructor options are reflected in the resolved config."""

    def test_render_form_fields_defaults_to_false(self):
        config = LiteParse(quiet=True).get_config()
        assert config.render_form_fields is False

    def test_render_form_fields_reflects_constructor_option(self):
        config = LiteParse(render_form_fields=True, quiet=True).get_config()
        assert config.render_form_fields is True
