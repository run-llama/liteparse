from anthropic import Anthropic

from .base import LLMProvider, QA_PROMPT, JUDGE_PROMPT

DEFAULT_MODEL = "claude-sonnet-5-5"


def _response_text(response) -> str:
    """
    Join the text blocks of a Messages API response.

    Newer models can start a response with `thinking` blocks, so the text is not
    always at `content[0]`.
    """
    return "".join(
        block.text for block in response.content if getattr(block, "type", None) == "text"
    )


class AnthropicProvider(LLMProvider):
    """
    LLM provider using Anthropic for QA.
    """

    def __init__(self, api_key: str = None, model: str = DEFAULT_MODEL):
        """
        Initialize Anthropic QA provider.

        Args:
            api_key: Anthropic API key (or use ANTHROPIC_API_KEY env var)
            model: Claude model to use
        """

        self.client = Anthropic(api_key=api_key, max_retries=100, timeout=10000)
        self.model = model

    def answer_question(self, ocr_text: str, question: str) -> str:
        """Answer a question about an image using Claude."""

        # Call Claude
        response = self.client.messages.create(
            model=self.model,
            # max_tokens covers thinking as well as the answer on models that think by default.
            max_tokens=4096,
            messages=[
                {
                    "role": "user",
                    "content": QA_PROMPT.format(ocr_text=ocr_text, question=question),
                }
            ],
            timeout=10000,
        )
        if not response.content or len(response.content) == 0:
            raise ValueError("No content returned from Anthropic response")

        text = _response_text(response)
        if not text:
            raise ValueError("No text returned from Anthropic response")
        return text

    def evaluate_answer(self, question: str, expected_answer: str, predicted_answer: str) -> bool:
        """
        Evaluate whether the predicted answer is correct compared to the expected answer using an LLM judge.
        """

        # Call the LLM judge
        judge_prompt = JUDGE_PROMPT.format(
            question=question,
            expected_answer=expected_answer,
            predicted_answer=predicted_answer,
        )

        response = self.client.messages.create(
            model=self.model,
            # Leave room for thinking before the <pass>/<fail> verdict.
            max_tokens=2048,
            messages=[
                {
                    "role": "user",
                    "content": judge_prompt,
                }
            ],
            timeout=10000,
        )
        if not response.content or len(response.content) == 0:
            # If the judge fails to respond, we can choose to be lenient and pass the answer
            return True  

        # Check if the response is "<pass>" or "<fail>"
        resp_text = _response_text(response).strip()

        resp_text_lower = resp_text.lower()
        return "<pass" in resp_text_lower and "<fail" not in resp_text_lower
