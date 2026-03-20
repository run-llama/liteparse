"""Integration tests for MiniMax LLM provider.

These tests call the real MiniMax API and require MINIMAX_API_KEY to be set.
Skip with: pytest -m "not integration"
"""

import os

import pytest

from liteparse_eval.providers.llm.minimax import MiniMaxProvider

pytestmark = pytest.mark.integration

MINIMAX_API_KEY = os.environ.get("MINIMAX_API_KEY")
skip_no_key = pytest.mark.skipif(
    not MINIMAX_API_KEY,
    reason="MINIMAX_API_KEY not set",
)


@skip_no_key
class TestMiniMaxIntegration:
    """Integration tests that call the real MiniMax API."""

    def test_answer_question_real(self):
        """Should get a real answer from MiniMax API."""
        provider = MiniMaxProvider()
        answer = provider.answer_question(
            "The Eiffel Tower is a wrought-iron lattice tower in Paris, France. "
            "It was constructed from 1887 to 1889 as the centerpiece of the 1889 "
            "World's Fair. The tower is 330 metres (1,083 ft) tall.",
            "How tall is the Eiffel Tower?",
        )
        assert answer
        assert any(term in answer.lower() for term in ["330", "1,083", "1083"])

    def test_evaluate_answer_pass_real(self):
        """Should pass when answers are semantically equivalent."""
        provider = MiniMaxProvider()
        result = provider.evaluate_answer(
            "How tall is the Eiffel Tower?",
            "330 metres",
            "The Eiffel Tower is 330 meters tall.",
        )
        assert result is True

    def test_evaluate_answer_fail_real(self):
        """Should fail when answers differ semantically."""
        provider = MiniMaxProvider()
        result = provider.evaluate_answer(
            "How tall is the Eiffel Tower?",
            "330 metres",
            "The Statue of Liberty is 93 meters tall.",
        )
        assert result is False
