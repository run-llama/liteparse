from openai import OpenAI

from .base import LLMProvider, QA_PROMPT, JUDGE_PROMPT


class MiniMaxProvider(LLMProvider):
    """
    LLM provider using MiniMax for QA via OpenAI-compatible API.
    """

    DEFAULT_BASE_URL = "https://api.minimax.io/v1"

    def __init__(
        self,
        api_key: str = None,
        model: str = "MiniMax-M2.7",
        base_url: str = None,
    ):
        """
        Initialize MiniMax QA provider.

        Args:
            api_key: MiniMax API key (or use MINIMAX_API_KEY env var)
            model: MiniMax model to use (default: MiniMax-M2.7)
            base_url: API base URL (default: https://api.minimax.io/v1)
        """
        import os

        resolved_key = api_key or os.environ.get("MINIMAX_API_KEY")
        if not resolved_key:
            raise ValueError(
                "MiniMax API key is required. Pass api_key or set MINIMAX_API_KEY env var."
            )

        self.client = OpenAI(
            api_key=resolved_key,
            base_url=base_url or self.DEFAULT_BASE_URL,
            max_retries=3,
            timeout=120.0,
        )
        self.model = model

    def answer_question(self, ocr_text: str, question: str) -> str:
        """Answer a question about a document using MiniMax."""

        response = self.client.chat.completions.create(
            model=self.model,
            max_tokens=1024,
            temperature=0.1,
            messages=[
                {
                    "role": "user",
                    "content": QA_PROMPT.format(ocr_text=ocr_text, question=question),
                }
            ],
        )
        if not response.choices or len(response.choices) == 0:
            raise ValueError("No content returned from MiniMax response")

        text = response.choices[0].message.content
        # Strip <think>...</think> tags that MiniMax M2.5+ models may produce
        if text and "<think>" in text:
            import re
            text = re.sub(r"<think>.*?</think>\s*", "", text, flags=re.DOTALL)

        return text.strip() if text else ""

    def evaluate_answer(
        self, question: str, expected_answer: str, predicted_answer: str
    ) -> bool:
        """
        Evaluate whether the predicted answer is correct compared to the expected
        answer using an LLM judge.
        """

        judge_prompt = JUDGE_PROMPT.format(
            question=question,
            expected_answer=expected_answer,
            predicted_answer=predicted_answer,
        )

        response = self.client.chat.completions.create(
            model=self.model,
            max_tokens=256,
            temperature=0.0,
            messages=[
                {
                    "role": "user",
                    "content": judge_prompt,
                }
            ],
        )
        if not response.choices or len(response.choices) == 0:
            return True

        resp_text = response.choices[0].message.content or ""
        # Strip <think>...</think> tags
        if "<think>" in resp_text:
            import re
            resp_text = re.sub(r"<think>.*?</think>\s*", "", resp_text, flags=re.DOTALL)

        resp_text_lower = resp_text.strip().lower()
        return "<pass" in resp_text_lower and "<fail" not in resp_text_lower
