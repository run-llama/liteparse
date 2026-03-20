"""Unit tests for MiniMax LLM provider."""

import os
from unittest.mock import MagicMock, patch

import pytest

from liteparse_eval.providers.llm.minimax import MiniMaxProvider
from liteparse_eval.providers.llm.base import LLMProvider


class TestMiniMaxProviderInit:
    """Tests for MiniMaxProvider initialization."""

    def test_requires_api_key(self):
        """Should raise ValueError when no API key is provided."""
        with patch.dict(os.environ, {}, clear=True):
            env = os.environ.copy()
            env.pop("MINIMAX_API_KEY", None)
            with patch.dict(os.environ, env, clear=True):
                with pytest.raises(ValueError, match="MiniMax API key is required"):
                    MiniMaxProvider()

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_accepts_explicit_api_key(self, mock_openai):
        """Should accept an explicit API key."""
        provider = MiniMaxProvider(api_key="test-key-123")
        mock_openai.assert_called_once()
        call_kwargs = mock_openai.call_args[1]
        assert call_kwargs["api_key"] == "test-key-123"
        assert call_kwargs["base_url"] == "https://api.minimax.io/v1"

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_reads_env_var(self, mock_openai):
        """Should read MINIMAX_API_KEY from environment."""
        with patch.dict(os.environ, {"MINIMAX_API_KEY": "env-key-456"}):
            provider = MiniMaxProvider()
            call_kwargs = mock_openai.call_args[1]
            assert call_kwargs["api_key"] == "env-key-456"

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_default_model(self, mock_openai):
        """Should default to MiniMax-M2.7 model."""
        provider = MiniMaxProvider(api_key="test-key")
        assert provider.model == "MiniMax-M2.7"

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_custom_model(self, mock_openai):
        """Should accept a custom model name."""
        provider = MiniMaxProvider(api_key="test-key", model="MiniMax-M2.5-highspeed")
        assert provider.model == "MiniMax-M2.5-highspeed"

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_custom_base_url(self, mock_openai):
        """Should accept a custom base URL."""
        provider = MiniMaxProvider(api_key="test-key", base_url="https://custom.api.com/v1")
        call_kwargs = mock_openai.call_args[1]
        assert call_kwargs["base_url"] == "https://custom.api.com/v1"

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_is_llm_provider_subclass(self, mock_openai):
        """MiniMaxProvider should be a subclass of LLMProvider."""
        provider = MiniMaxProvider(api_key="test-key")
        assert isinstance(provider, LLMProvider)


class TestMiniMaxProviderAnswerQuestion:
    """Tests for MiniMaxProvider.answer_question()."""

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_answer_question_basic(self, mock_openai):
        """Should return the answer from the API response."""
        mock_client = MagicMock()
        mock_openai.return_value = mock_client

        mock_choice = MagicMock()
        mock_choice.message.content = "The answer is 42."
        mock_response = MagicMock()
        mock_response.choices = [mock_choice]
        mock_client.chat.completions.create.return_value = mock_response

        provider = MiniMaxProvider(api_key="test-key")
        result = provider.answer_question("Some OCR text", "What is the answer?")

        assert result == "The answer is 42."
        mock_client.chat.completions.create.assert_called_once()
        call_kwargs = mock_client.chat.completions.create.call_args[1]
        assert call_kwargs["model"] == "MiniMax-M2.7"
        assert call_kwargs["max_tokens"] == 1024
        assert call_kwargs["temperature"] == 0.1

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_answer_question_strips_think_tags(self, mock_openai):
        """Should strip <think>...</think> tags from the response."""
        mock_client = MagicMock()
        mock_openai.return_value = mock_client

        mock_choice = MagicMock()
        mock_choice.message.content = "<think>Let me analyze this document carefully...</think>\nThe answer is Paris."
        mock_response = MagicMock()
        mock_response.choices = [mock_choice]
        mock_client.chat.completions.create.return_value = mock_response

        provider = MiniMaxProvider(api_key="test-key")
        result = provider.answer_question("Document about France", "What is the capital?")

        assert result == "The answer is Paris."

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_answer_question_empty_choices_raises(self, mock_openai):
        """Should raise ValueError when response has no choices."""
        mock_client = MagicMock()
        mock_openai.return_value = mock_client

        mock_response = MagicMock()
        mock_response.choices = []
        mock_client.chat.completions.create.return_value = mock_response

        provider = MiniMaxProvider(api_key="test-key")
        with pytest.raises(ValueError, match="No content returned"):
            provider.answer_question("text", "question?")

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_answer_question_none_content(self, mock_openai):
        """Should return empty string when content is None."""
        mock_client = MagicMock()
        mock_openai.return_value = mock_client

        mock_choice = MagicMock()
        mock_choice.message.content = None
        mock_response = MagicMock()
        mock_response.choices = [mock_choice]
        mock_client.chat.completions.create.return_value = mock_response

        provider = MiniMaxProvider(api_key="test-key")
        result = provider.answer_question("text", "question?")
        assert result == ""

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_answer_question_prompt_formatting(self, mock_openai):
        """Should properly format the QA prompt."""
        mock_client = MagicMock()
        mock_openai.return_value = mock_client

        mock_choice = MagicMock()
        mock_choice.message.content = "answer"
        mock_response = MagicMock()
        mock_response.choices = [mock_choice]
        mock_client.chat.completions.create.return_value = mock_response

        provider = MiniMaxProvider(api_key="test-key")
        provider.answer_question("my document text", "what is this?")

        call_kwargs = mock_client.chat.completions.create.call_args[1]
        messages = call_kwargs["messages"]
        assert len(messages) == 1
        assert messages[0]["role"] == "user"
        assert "my document text" in messages[0]["content"]
        assert "what is this?" in messages[0]["content"]


class TestMiniMaxProviderEvaluateAnswer:
    """Tests for MiniMaxProvider.evaluate_answer()."""

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_evaluate_answer_pass(self, mock_openai):
        """Should return True when judge says pass."""
        mock_client = MagicMock()
        mock_openai.return_value = mock_client

        mock_choice = MagicMock()
        mock_choice.message.content = "<pass>Both answers say Paris is the capital.</pass>"
        mock_response = MagicMock()
        mock_response.choices = [mock_choice]
        mock_client.chat.completions.create.return_value = mock_response

        provider = MiniMaxProvider(api_key="test-key")
        result = provider.evaluate_answer("What is the capital?", "Paris", "Paris, France")

        assert result is True

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_evaluate_answer_fail(self, mock_openai):
        """Should return False when judge says fail."""
        mock_client = MagicMock()
        mock_openai.return_value = mock_client

        mock_choice = MagicMock()
        mock_choice.message.content = "<fail>The answers are different.</fail>"
        mock_response = MagicMock()
        mock_response.choices = [mock_choice]
        mock_client.chat.completions.create.return_value = mock_response

        provider = MiniMaxProvider(api_key="test-key")
        result = provider.evaluate_answer("What is the capital?", "Paris", "London")

        assert result is False

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_evaluate_answer_strips_think_tags(self, mock_openai):
        """Should strip think tags before evaluating pass/fail."""
        mock_client = MagicMock()
        mock_openai.return_value = mock_client

        mock_choice = MagicMock()
        mock_choice.message.content = "<think>Let me compare...</think>\n<pass>Same meaning.</pass>"
        mock_response = MagicMock()
        mock_response.choices = [mock_choice]
        mock_client.chat.completions.create.return_value = mock_response

        provider = MiniMaxProvider(api_key="test-key")
        result = provider.evaluate_answer("Q?", "A", "A")

        assert result is True

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_evaluate_answer_empty_choices_returns_true(self, mock_openai):
        """Should return True (lenient) when response has no choices."""
        mock_client = MagicMock()
        mock_openai.return_value = mock_client

        mock_response = MagicMock()
        mock_response.choices = []
        mock_client.chat.completions.create.return_value = mock_response

        provider = MiniMaxProvider(api_key="test-key")
        result = provider.evaluate_answer("Q?", "A", "A")

        assert result is True

    @patch("liteparse_eval.providers.llm.minimax.OpenAI")
    def test_evaluate_answer_temperature_zero(self, mock_openai):
        """Judge evaluation should use temperature=0 for consistency."""
        mock_client = MagicMock()
        mock_openai.return_value = mock_client

        mock_choice = MagicMock()
        mock_choice.message.content = "<pass>OK</pass>"
        mock_response = MagicMock()
        mock_response.choices = [mock_choice]
        mock_client.chat.completions.create.return_value = mock_response

        provider = MiniMaxProvider(api_key="test-key")
        provider.evaluate_answer("Q?", "A", "A")

        call_kwargs = mock_client.chat.completions.create.call_args[1]
        assert call_kwargs["temperature"] == 0.0
        assert call_kwargs["max_tokens"] == 256


class TestMiniMaxProviderExports:
    """Tests for MiniMaxProvider module exports."""

    def test_importable_from_providers_llm(self):
        """Should be importable from providers.llm package."""
        from liteparse_eval.providers.llm import MiniMaxProvider
        assert MiniMaxProvider is not None

    def test_importable_from_providers(self):
        """Should be importable from providers package."""
        from liteparse_eval.providers import MiniMaxProvider
        assert MiniMaxProvider is not None

    def test_importable_from_top_level(self):
        """Should be importable from top-level package."""
        from liteparse_eval import MiniMaxProvider
        assert MiniMaxProvider is not None

    def test_in_providers_all(self):
        """Should be listed in providers.__all__."""
        from liteparse_eval.providers import __all__
        assert "MiniMaxProvider" in __all__
