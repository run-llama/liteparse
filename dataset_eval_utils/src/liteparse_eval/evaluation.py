"""
Evaluation and benchmarking script for text extraction and LLM-based document understanding.

This script provides:
1. LLM QA evaluation using an LLM judge for pass/fail evaluation
2. Latency tracking for LLM and parse operations
"""

import argparse
import json
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import List, Optional

from liteparse_eval.providers import (
    ParserProvider,
    LLMProvider,
    AnthropicProvider,
    MiniMaxProvider,
    LiteparseProvider,
    MarkItDownProvider,
    PyMuPDFProvider,
    PyPDFProvider,
)


@dataclass
class LatencyMetrics:
    """Latency metrics for provider calls."""
    latencies: List[float] = field(default_factory=list)  # Individual call latencies in seconds

    @property
    def count(self) -> int:
        """Number of calls."""
        return len(self.latencies)

    @property
    def average(self) -> float:
        """Average latency in seconds."""
        return sum(self.latencies) / len(self.latencies) if self.latencies else 0.0

    @property
    def min(self) -> float:
        """Minimum latency in seconds."""
        return min(self.latencies) if self.latencies else 0.0

    @property
    def max(self) -> float:
        """Maximum latency in seconds."""
        return max(self.latencies) if self.latencies else 0.0

    @property
    def stddev(self) -> float:
        """Standard deviation of latency in seconds."""
        if not self.latencies or len(self.latencies) < 2:
            return 0.0
        mean = self.average
        variance = sum((x - mean) ** 2 for x in self.latencies) / len(self.latencies)
        return variance ** 0.5

    @property
    def total(self) -> float:
        """Total latency in seconds."""
        return sum(self.latencies) if self.latencies else 0.0

    def to_dict(self) -> dict:
        """Convert to dictionary for JSON serialization."""
        return {
            "count": self.count,
            "total_seconds": round(self.total, 3),
            "average_seconds": round(self.average, 3),
            "min_seconds": round(self.min, 3),
            "max_seconds": round(self.max, 3),
            "stddev_seconds": round(self.stddev, 3),
            "individual_latencies": [round(lat, 3) for lat in self.latencies]
        }


@dataclass
class QAResult:
    """Result for a single QA pair evaluation."""
    question: str
    expected_answer: str
    predicted_answer: str
    llm_judge_pass: bool


@dataclass
class QAEvalResult:
    """Results for QA evaluation on a single document."""
    file_path: Path
    total_questions: int
    llm_judge_pass_rate: float
    qa_results: List[QAResult]
    llm_latency_metrics: Optional[LatencyMetrics] = None
    parse_latency_seconds: Optional[float] = None


class Benchmark:
    """Main benchmark runner for text extraction and QA evaluation."""

    def __init__(
        self,
        parser_provider: Optional[ParserProvider] = None,
        llm_provider: Optional[LLMProvider] = None,
        llm_judge_provider: Optional[LLMProvider] = None,
    ):
        """
        Initialize the benchmark.

        Args:
            parser_provider: Parser provider to use for text extraction
            llm_provider: LLM provider to use for answering questions
            llm_judge_provider: LLM provider for judge-based evaluation
        """
        self.parser_provider = parser_provider
        self.llm_provider = llm_provider
        self.llm_judge_provider = llm_judge_provider

    def run_qa_eval(
        self,
        extracted_text: str,
        doc_path: Path,
        ground_truth_path: Path,
        parse_latency: Optional[float] = None
    ) -> QAEvalResult:
        """
        Run QA evaluation on a single document.

        Args:
            extracted_text: Extracted text from the document
            doc_path: Path to the source document
            ground_truth_path: Path to the ground truth JSON file
            parse_latency: Time taken for text extraction in seconds

        Returns:
            QAEvalResult with evaluation metrics
        """
        if not self.llm_provider or not self.llm_judge_provider:
            raise ValueError("LLM provider and judge provider must be configured")

        # Load ground truth
        with open(ground_truth_path, "r") as f:
            ground_truth = json.load(f)

        # Get predicted answers with latency tracking
        predicted_answers = []
        llm_latency_metrics = LatencyMetrics()
        for qa_pair in ground_truth["qa_pairs"]:
            start_time = time.perf_counter()
            answer = self.llm_provider.answer_question(extracted_text, qa_pair["question"])
            latency = time.perf_counter() - start_time
            llm_latency_metrics.latencies.append(latency)
            predicted_answers.append(answer)

        # Evaluate with LLM judge
        qa_results = []
        judge_passes = 0

        for predicted, gt_pair in zip(predicted_answers, ground_truth["qa_pairs"]):
            question = gt_pair["question"]
            expected = gt_pair["answer"]

            try:
                llm_judge_pass = self.llm_judge_provider.evaluate_answer(question, expected, predicted)
            except Exception as e:
                print(f"  Warning: LLM judge evaluation failed: {e}")
                llm_judge_pass = False

            if llm_judge_pass:
                judge_passes += 1

            qa_results.append(QAResult(
                question=question,
                expected_answer=expected,
                predicted_answer=predicted,
                llm_judge_pass=llm_judge_pass,
            ))

        total = len(ground_truth["qa_pairs"])
        llm_judge_pass_rate = judge_passes / total if total > 0 else 0.0

        result = QAEvalResult(
            file_path=doc_path,
            total_questions=total,
            llm_judge_pass_rate=llm_judge_pass_rate,
            qa_results=qa_results,
            llm_latency_metrics=llm_latency_metrics,
            parse_latency_seconds=parse_latency,
        )
        return result

    def run_full_benchmark(
        self,
        data_dir: Path,
        ground_truth_dir: Path,
        output_path: Optional[Path] = None
    ) -> dict:
        """
        Run full benchmark across all documents using batch extraction.

        Args:
            data_dir: Directory containing input documents
            ground_truth_dir: Directory containing ground truth JSON files
            output_path: Optional path to save detailed results

        Returns:
            Dictionary with aggregated benchmark results
        """
        qa_results: list[QAEvalResult] = []
        extracted_texts: dict[str, str] = {}  # Store extracted text for each document

        # Find all ground truth files
        gt_files = sorted(ground_truth_dir.glob("*.json"))

        print(f"Running benchmark on {len(gt_files)} documents...")

        # Find all source documents and match them to ground truth files
        source_docs = sorted(data_dir.glob("*.pdf"))
        doc_gt_pairs: list[tuple[Path, Path]] = []

        for gt_path in gt_files:
            source_doc = next((
                doc for doc in source_docs if doc.stem == gt_path.stem
            ), None)

            if not source_doc:
                print(f"  Warning: Could not find source document for {gt_path.name}")
                continue

            doc_gt_pairs.append((source_doc, gt_path))

        if not doc_gt_pairs:
            print("No documents found to process.")
            return {}

        # Extract text
        parse_latency_per_doc: dict[Path, float] = {}
        if self.parser_provider:
            docs_to_extract = [doc for doc, _ in doc_gt_pairs]

            for doc_path in docs_to_extract:
                start_time = time.perf_counter()
                try:
                    parse_result = self.parser_provider.extract_text(doc_path)
                    total_time = time.perf_counter() - start_time

                    extracted_texts[str(doc_path)] = parse_result
                    parse_latency_per_doc[doc_path] = total_time
                except Exception as e:
                    print(f"  Error: extraction failed: {e}")
                    parse_result = ""

        # Run QA evaluation for each document
        for i, (source_doc, gt_path) in enumerate(doc_gt_pairs, 1):
            print(f"\n[{i}/{len(doc_gt_pairs)}] Evaluating: {gt_path.name}")

            extracted_text = extracted_texts.get(str(source_doc), "")
            parse_latency = parse_latency_per_doc.get(source_doc)

            # Run QA evaluation
            try:
                qa_result = self.run_qa_eval(extracted_text, source_doc, gt_path, parse_latency)
                qa_results.append(qa_result)
                latency_str = ""
                if qa_result.llm_latency_metrics:
                    avg_lat = qa_result.llm_latency_metrics.average
                    latency_str = f" [avg LLM: {avg_lat:.2f}s]"

                print(f"  QA: LLM judge pass: {qa_result.llm_judge_pass_rate:.1%}{latency_str}")
            except Exception as e:
                print(f"  Error: QA evaluation failed: {e}")

        # Aggregate results
        aggregate = {}

        if qa_results:
            total_questions = sum(r.total_questions for r in qa_results)
            total_llm_judge_passes = sum(
                r.llm_judge_pass_rate * r.total_questions for r in qa_results
            )

            # Aggregate parse latency metrics
            parse_latencies = [r.parse_latency_seconds for r in qa_results if r.parse_latency_seconds is not None]
            parse_latency_metrics = LatencyMetrics(latencies=parse_latencies) if parse_latencies else None

            # Aggregate LLM latency metrics across all documents
            all_llm_latencies = []
            for r in qa_results:
                if r.llm_latency_metrics:
                    all_llm_latencies.extend(r.llm_latency_metrics.latencies)
            llm_latency_metrics = LatencyMetrics(latencies=all_llm_latencies) if all_llm_latencies else None

            aggregate["qa"] = {
                "total_documents": len(qa_results),
                "total_questions": total_questions,
                "overall_llm_judge_pass_rate": total_llm_judge_passes / total_questions if total_questions > 0 else 0.0,
                "per_document_results": [
                    {
                        "file": str(r.file_path),
                        "llm_judge_pass_rate": r.llm_judge_pass_rate,
                        "total_questions": r.total_questions,
                        "parse_latency_seconds": r.parse_latency_seconds,
                        "llm_latency_metrics": r.llm_latency_metrics.to_dict() if r.llm_latency_metrics else None
                    }
                    for r in qa_results
                ]
            }

            if parse_latency_metrics:
                aggregate["qa"]["parse_latency_metrics"] = parse_latency_metrics.to_dict()

            if llm_latency_metrics:
                aggregate["qa"]["llm_latency_metrics"] = llm_latency_metrics.to_dict()

        # Save results if requested
        if output_path:
            # Save aggregate results
            with open(f"{output_path}.json", "w") as f:
                json.dump(aggregate, f, indent=2)
            print(f"\nAggregate results saved to: {output_path}")

            # Save detailed results with extracted text for debugging
            detailed_output_path = output_path.parent / f"{output_path.stem}_detailed{output_path.suffix}"
            detailed_results = self._build_detailed_results(qa_results, extracted_texts)
            with open(f"{detailed_output_path}.json", "w") as f:
                json.dump(detailed_results, f, indent=2)
            print(f"Detailed results saved to: {detailed_output_path}")

            # Generate HTML report
            try:
                from liteparse_eval.report import HTMLReportGenerator

                html_report_path = output_path.parent / f"{output_path.stem}_report.html"
                generator = HTMLReportGenerator(
                    detailed_results=detailed_results,
                    ground_truth_dir=ground_truth_dir
                )
                generator.generate_report(html_report_path)
                print(f"HTML report saved to: {html_report_path}")
            except Exception as e:
                print(f"Warning: HTML report generation failed: {e}")
                # Don't fail the entire benchmark if HTML generation fails

        return aggregate

    def _build_detailed_results(
        self,
        qa_results: list[QAEvalResult],
        extracted_texts: dict[str, str]
    ) -> dict:
        """
        Build detailed results including extracted text and individual test results.

        Args:
            qa_results: List of QA evaluation results
            extracted_texts: Dictionary mapping file paths to extracted text

        Returns:
            Dictionary with detailed results for debugging
        """
        detailed = {"documents": []}

        # Create a mapping of file paths to results
        qa_map = {str(r.file_path): r for r in qa_results}

        # Combine results for each document
        all_files = set(qa_map.keys())

        for file_path in sorted(all_files):
            doc_result = {
                "file": file_path,
                "extracted_text": extracted_texts.get(file_path, "")
            }

            # Add QA evaluation details
            if file_path in qa_map:
                qa_result = qa_map[file_path]
                doc_result["qa_evaluation"] = {
                    "llm_judge_pass_rate": qa_result.llm_judge_pass_rate,
                    "total_questions": qa_result.total_questions,
                    "parse_latency_seconds": qa_result.parse_latency_seconds,
                    "llm_latency_metrics": qa_result.llm_latency_metrics.to_dict() if qa_result.llm_latency_metrics else None,
                    "qa_pairs": [
                        {
                            "question": qa.question,
                            "expected_answer": qa.expected_answer,
                            "predicted_answer": qa.predicted_answer,
                            "llm_judge_pass": qa.llm_judge_pass,
                        }
                        for qa in qa_result.qa_results
                    ]
                }

            detailed["documents"].append(doc_result)

        return detailed


def main():
    """Entry point of the benchmark framework."""

    parser = argparse.ArgumentParser(
        description="Benchmark text extraction and LLM providers on document understanding tasks"
    )
    parser.add_argument(
        "--data-dir",
        type=Path,
        required=True,
        help="Directory containing source documents"
    )
    parser.add_argument(
        "--ground-truth-dir",
        type=Path,
        required=True,
        help="Directory containing ground truth JSON files"
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="Path to save detailed benchmark results"
    )
    parser.add_argument(
        "--parse-provider",
        type=str,
        choices=["pymupdf", "pypdf", "markitdown", "liteparse"],
        default="liteparse",
        help="Parse provider to use for text extraction. (default: liteparse)"
    )
    parser.add_argument(
        "--llm-provider",
        type=str,
        choices=["anthropic", "minimax"],
        default="anthropic",
        help="LLM provider to use. (default: anthropic)"
    )

    args = parser.parse_args()

    # Initialize parser provider
    if args.parse_provider == "pymupdf":
        parser_provider = PyMuPDFProvider()
    elif args.parse_provider == "pypdf":
        parser_provider = PyPDFProvider()
    elif args.parse_provider == "markitdown":
        parser_provider = MarkItDownProvider()
    elif args.parse_provider == "liteparse":
        parser_provider = LiteparseProvider()
    else:
        raise ValueError("Please specify a valid parser provider using --parse-provider")

    # Initialize LLM provider
    if args.llm_provider == "anthropic":
        llm_provider = AnthropicProvider()
        llm_judge_provider = AnthropicProvider(model="claude-haiku-4-5-20251001")
    elif args.llm_provider == "minimax":
        llm_provider = MiniMaxProvider()
        llm_judge_provider = MiniMaxProvider(model="MiniMax-M2.5-highspeed")
    else:
        raise ValueError("Please specify a valid LLM provider using --llm-provider")

    benchmark = Benchmark(
        parser_provider=parser_provider,
        llm_provider=llm_provider,
        llm_judge_provider=llm_judge_provider,
    )

    results = benchmark.run_full_benchmark(
        data_dir=args.data_dir,
        ground_truth_dir=args.ground_truth_dir,
        output_path=args.output
    )

    print("\n" + "="*60)
    print("BENCHMARK RESULTS")
    print("="*60)

    if "qa" in results:
        print(f"\nQA Evaluation:")
        print(f"  Overall LLM Judge Pass Rate: {results['qa']['overall_llm_judge_pass_rate']:.1%}")
        print(f"  Total Questions: {results['qa']['total_questions']}")


if __name__ == "__main__":
    exit(main())
