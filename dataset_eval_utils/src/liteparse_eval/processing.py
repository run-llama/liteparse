"""
Process PDFs and images to create a structured dataset using Anthropic's Claude with vision.
"""

import base64
import json
import random
from pathlib import Path
from typing import List, Literal

from anthropic import Anthropic
from liteparse import LiteParse
from pydantic import BaseModel, Field

from liteparse_eval.providers.llm.anthropic import DEFAULT_MODEL


# Define the output schema using Pydantic-like structure
class QAPair(BaseModel):
    question: str = Field(..., description="A question that can only be answered using information from the page.")
    answer: str = Field()

class PageAnnotation(BaseModel):
    has_text: bool = Field(..., description="Whether the document contains readable text")
    document_type: Literal["academic_paper", "form", "invoice", "newspaper", "other"] = Field(
        ..., description="The type of document"
    )
    layout_complexity: Literal["simple", "multi_column", "complex"] = Field(
        ..., description="The complexity of the document layout"
    )
    qa_pairs: List[QAPair] = Field(
        ...,
        description="Question-answer pairs about the document",
        example=[{"question": "What is the main topic?", "answer": "Sample Answer"}]
    )


def pdf_to_images(pdf_path: Path, dpi: int = 150) -> List[Path]:
    """
    Convert a PDF to a list of image paths (one per page) using liteparse.

    Args:
        pdf_path: Path to the PDF file
        dpi: DPI for rendering (default: 150)

    Returns:
        List of paths to generated images (one per page)
    """
    parser = LiteParse()
    result = parser.screenshot(pdf_path, dpi=dpi)

    return [Path(s.image_path) for s in result.screenshots]

    
def encode_image(image_path: Path) -> tuple[str, str]:
    """
    Encode an image to base64 and determine its media type.

    Args:
        image_path: Path to the image file

    Returns:
        Tuple of (base64_encoded_data, media_type)
    """
    with open(image_path, "rb") as image_file:
        image_data = base64.standard_b64encode(image_file.read()).decode("utf-8")

    # Determine media type from extension
    extension = image_path.suffix.lower()
    media_type_map = {
        ".jpg": "image/jpeg",
        ".jpeg": "image/jpeg",
        ".png": "image/png",
        ".gif": "image/gif",
        ".webp": "image/webp"
    }
    media_type = media_type_map.get(extension, "image/jpeg")

    return image_data, media_type


def analyze_image_with_claude(
    client: Anthropic,
    image_path: Path,
    model: str = DEFAULT_MODEL
) -> PageAnnotation | None:
    """
    Analyze an image using Claude with structured outputs.

    Args:
        client: Anthropic client
        image_path: Path to the image to analyze
        model: Model to use for analysis

    Returns:
        Structured analysis result as a dictionary
    """
    image_data, media_type = encode_image(image_path)

    prompt = """Analyze this document image and provide a structured analysis.

Extract:
1. Whether it contains readable text
2. The document type (academic_paper, form, invoice, newspaper, or other)
3. Layout complexity (simple, multi_column, or complex)
4. Generate 3-5 question-answer pairs about the document content

Be thorough and accurate in your analysis. This data will be used in a document parsing benchmark (LLM-as-a-judge on QA responses), so extracted data should be interesting, diverse, and sometimes challenging."""

    # With .parse() - can pass Pydantic model directly
    response = client.beta.messages.parse(
        model=model,
        max_tokens=8192,
        betas=["structured-outputs-2025-11-13"],
        messages=[
            {
                "role": "user",
                "content": [
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": media_type,
                            "data": image_data,
                        },
                    },
                    {
                        "type": "text",
                        "text": prompt
                    }
                ],
            }
        ],
        output_format=PageAnnotation,
    )

    return response.parsed_output


def process_file(
    client: Anthropic,
    file_path: Path,
    output_dir: Path,
    model: str = DEFAULT_MODEL
) -> None:
    """
    Process a single file (PDF or image) and save results.

    Args:
        client: Anthropic client
        file_path: Path to the file to process
        output_dir: Directory to save output JSON files
        model: Model to use for analysis
    """
    print(f"Processing: {file_path}")

    # Handle PDFs vs images
    if file_path.suffix.lower() == ".pdf":
        try:
            image_paths = pdf_to_images(file_path)
        except NotImplementedError:
            print(f"  Skipping PDF (conversion not implemented): {file_path}")
            return
    else:
        # Single image file
        image_paths = [file_path]

    # Process each page/image
    for page_num, image_path in enumerate(image_paths, start=1):
        try:
            print(f"  Analyzing page {page_num}/{len(image_paths)}...")
            result = analyze_image_with_claude(client, image_path, model)

            if result is None:
                print(f"  ✗ No results for page {page_num} in {file_path}")
                continue

            # Create output filename
            if len(image_paths) > 1:
                # Multi-page PDF
                output_filename = f"{file_path.stem}_page_{page_num:03d}.json"
            else:
                # Single image
                output_filename = f"{file_path.stem}.json"

            output_path = output_dir / output_filename

            # Save result
            with open(output_path, "w", encoding="utf-8") as f:
                json.dump(result.model_dump(), f, indent=2, ensure_ascii=False)

            print(f"  ✓ Saved: {output_path}")

        except Exception as e:
            print(f"  ✗ Error processing page {page_num}: {e}")


def find_documents(input_dir: Path) -> List[Path]:
    """
    Find all PDF and image files in a directory (recursively).

    Args:
        input_dir: Directory to search

    Returns:
        List of paths to document files
    """
    supported_extensions = {".pdf", ".jpg", ".jpeg", ".png", ".gif", ".webp"}
    documents = []

    for ext in supported_extensions:
        documents.extend(input_dir.rglob(f"*{ext}"))
        documents.extend(input_dir.rglob(f"*{ext.upper()}"))

    return sorted(set(documents))


def main():
    """Main entry point."""
    import argparse

    parser = argparse.ArgumentParser(
        description="Process PDFs and images to create a structured dataset"
    )
    parser.add_argument(
        "input_dir",
        type=Path,
        help="Directory containing PDFs and/or images to process"
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=Path("output"),
        help="Directory to save output JSON files (default: ./output)"
    )
    parser.add_argument(
        "--model",
        type=str,
        default=DEFAULT_MODEL,
        help=f"Claude model to use (default: {DEFAULT_MODEL})"
    )
    parser.add_argument(
        "--api-key",
        type=str,
        help="Anthropic API key (or set ANTHROPIC_API_KEY environment variable)"
    )

    args = parser.parse_args()

    # Validate input directory
    if not args.input_dir.exists():
        print(f"Error: Input directory does not exist: {args.input_dir}")
        return 1

    # Create output directory
    args.output_dir.mkdir(parents=True, exist_ok=True)

    # Initialize Anthropic client
    client = Anthropic(api_key=args.api_key) if args.api_key else Anthropic()

    # Find all documents
    documents = find_documents(args.input_dir)

    if not documents:
        print(f"No documents found in {args.input_dir}")
        return 1


    documents = random.sample(documents, 50)
    print(f"Found {len(documents)} document(s) to process\n")

    # Process each document
    for i, doc_path in enumerate(documents, start=1):
        print(f"\n[{i}/{len(documents)}]")
        process_file(client, doc_path, args.output_dir, args.model)

    print(f"\n✓ Complete! Results saved to: {args.output_dir}")
    return 0


if __name__ == "__main__":
    exit(main())
