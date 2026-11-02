#!/usr/bin/env python3
"""Pinned ParseBench adapter: prepare, prime OCR, measure, and score locally."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import importlib.metadata
import json
import math
import os
from pathlib import Path
import platform
import sys
import time

import bench

UPSTREAM_REVISION = "ee474343e29b11095f017626193d0b3395386aea"
METRICS = {"table": "grits_trm_composite", "chart": "rule_pass_rate",
           "text_content": "content_faithfulness", "text_formatting": "semantic_formatting",
           "layout": "layout_element_rule_pass_rate"}
# Force deterministic scoring before importing any ParseBench module.
os.environ["LLAMACLOUD_BENCH_LLM_NORMALIZATION"] = "off"


def evaluator_identity():
    import parse_bench
    root = Path(parse_bench.__file__).parent
    sources = {str(p.relative_to(root)): bench.digest(p.read_bytes()) for p in sorted(root.rglob("*.py"))}
    dependencies = sorted([d.metadata["Name"].lower(), d.version] for d in importlib.metadata.distributions())
    return {"sources_sha256": bench.digest(bench.canonical(sources)), "dependencies": dependencies,
            "upstream_reference": UPSTREAM_REVISION}


def prepare(args):
    from parse_bench.test_cases.loader import load_test_cases
    from pypdf import PdfReader
    tests = load_test_cases(args.dataset)
    groups = args.groups.split(",")
    if not set(groups).issubset(METRICS) or len(set(groups)) != len(groups):
        raise ValueError("Choose unique canonical ParseBench groups")
    selected = []
    for group in groups:
        candidates = [t for t in tests if t.group == group]
        candidates.sort(key=lambda t: bench.digest((args.seed + t.group + t.test_id).encode()))
        if not candidates:
            raise ValueError(f"No cases for {group}")
        selected.extend(candidates[:args.limit_per_category] if args.limit_per_category else candidates)
    documents, units, excluded = {}, [], []
    for test in selected:
        if getattr(test, "qa_config", None) or getattr(test, "qa_configs", None):
            raise ValueError("QA cases are not part of this parsing objective")
        if test.group == "layout" and hasattr(test, "get_layout_annotations") and not test.get_layout_annotations():
            excluded.append({"test_id": test.test_id, "reason": "no layout ground truth"})
            continue
        source = test.file_path.resolve()
        pdf_hash = bench.digest(source.read_bytes())
        if pdf_hash not in documents:
            documents[pdf_hash] = {"pdf": os.path.relpath(source, args.output.resolve().parent),
                                   "sha256": pdf_hash, "pages": len(PdfReader(source).pages)}
        payload = test.model_dump(mode="json")
        payload["file_path"] = documents[pdf_hash]["pdf"]
        # Category-qualified identity avoids text-content/formatting ID collisions.
        units.append({"id": f"{test.group}:{test.test_id}", "document": pdf_hash,
                      "type": "layout_detection" if hasattr(test, "get_layout_annotations") else "parse",
                      "test_case": payload, "metric": METRICS[test.group]})
    if set(u["test_case"]["group"] for u in units) != set(groups):
        raise ValueError("A selected category has no scorable ground truth")
    if len({u["id"] for u in units}) != len(units):
        raise ValueError("Duplicate category/test IDs")
    manifest = {"version": 1, "kind": "parsebench", "evaluator": evaluator_identity(),
                "selection": {"seed": args.seed, "limit_per_category": args.limit_per_category, "groups": groups},
                "documents": documents, "units": units, "excluded_ground_truth": excluded}
    with args.output.open("x") as stream:
        stream.write(json.dumps(manifest, indent=2) + "\n")
    print(f"Prepared {len(units)} evaluations over {len(documents)} unique PDFs; {len(excluded)} GT exclusions")


def load_manifest(path):
    manifest = json.loads(path.read_text())
    if manifest["kind"] != "parsebench" or manifest["version"] != 1 or not manifest["units"]:
        raise ValueError("Invalid ParseBench manifest")
    if manifest["evaluator"] != evaluator_identity():
        raise ValueError("ParseBench code/dependencies changed; prepare and rerun both references")
    for document in manifest["documents"].values():
        source = (path.resolve().parent / document["pdf"]).resolve()
        if bench.digest(source.read_bytes()) != document["sha256"]:
            raise ValueError(f"Dataset changed: {source}")
        if document["pages"] < 1:
            raise ValueError("Empty PDF")
    return manifest


def cache_snapshot(root):
    if root is None:
        return None
    if not (root / "profile.json").is_file():
        raise ValueError("OCR cache needs a recorded profile")
    files = {p.name: bench.digest(p.read_bytes()) for p in sorted(root.glob("*.json"))}
    return bench.digest(bench.canonical(files))


def command(args, source, page, mode):
    cmd = [str(args.worker.resolve()), str(source), "--page", str(page), "--ocr-mode", mode,
           "--language", args.language, "--dpi", str(args.dpi)]
    if mode != "off":
        cmd += ["--ocr-cache", str(args.ocr_cache.resolve()), "--ocr-id", args.ocr_id]
    if mode == "record" and args.ocr_server_url:
        cmd += ["--ocr-server-url", args.ocr_server_url]
    return cmd


def prime(args):
    manifest = load_manifest(args.manifest)
    args.output.mkdir(parents=True, exist_ok=False)
    records = []
    for doc_id, doc in sorted(manifest["documents"].items()):
        source = (args.manifest.resolve().parent / doc["pdf"]).resolve()
        for page in range(1, doc["pages"] + 1):
            stem = f"{doc_id}-{page}"
            measurement = bench.measure(command(args, source, page, "record"), args.output / f"{stem}.json",
                                        args.output / f"{stem}.stderr", args.timeout)
            records.append({"document": doc_id, "page": page, **measurement})
            if measurement["exit_code"] != 0 or measurement["timed_out"]:
                raise ValueError(f"OCR preparation failed: {stem}; inspect stderr")
    snapshot = cache_snapshot(args.ocr_cache)
    (args.output / "prime.json").write_text(json.dumps({"ocr_snapshot": snapshot, "records": records}, indent=2))
    print(f"OCR snapshot: {snapshot}; preparation cost recorded separately")


def normalized_result(pages, unit, source):
    from parse_bench.inference.providers.parse.liteparse import LiteParseProvider, _build_layout_pages
    from parse_bench.schemas.parse_output import PageIR, ParseOutput
    from parse_bench.schemas.pipeline_io import InferenceRequest, InferenceResult
    convert = LiteParseProvider._convert_md_tables_to_html
    layout_raw = [{"page_index": p["page"] - 1, "text": p["markdown"],
                   "width": p["width"], "height": p["height"], "blocks": p["blocks"]} for p in pages]
    output = ParseOutput(example_id=unit["id"], pipeline_name="liteparse_markdown", task_type="parse",
                         pages=[PageIR(page_index=p["page"] - 1, markdown=convert(p["markdown"])) for p in pages],
                         markdown="\n\n".join(convert(p["markdown"]) for p in pages),
                         layout_pages=_build_layout_pages(layout_raw))
    now = datetime(2000, 1, 1)  # deterministic metadata, never part of measured latency
    return InferenceResult(request=InferenceRequest(example_id=unit["id"], source_file_path=str(source), product_type="parse"),
                           pipeline_name="liteparse_markdown", product_type="parse", raw_output={"pages": layout_raw},
                           output=output, started_at=now, completed_at=now, latency_in_ms=0)


def evaluate(pages, unit, source, cache, identity):
    from parse_bench.evaluation.runner import _evaluate_single_worker
    key = bench.digest(bench.canonical({"output": pages, "unit": unit, "evaluator": identity,
                                      "adapter": bench.digest(Path(__file__).read_bytes())}))
    path = cache / f"{key}.json"
    if path.exists():
        stored = json.loads(path.read_text())
        if stored["key"] != key or stored["sha256"] != bench.digest(bench.canonical(stored["evaluation"])):
            raise ValueError("Corrupt score cache")
        return stored["evaluation"], True
    test_case = dict(unit["test_case"], file_path=str(source))
    result = normalized_result(pages, unit, source)
    cross = unit["type"] == "layout_detection"
    evaluation = _evaluate_single_worker(result.model_dump(mode="json"), test_case, unit["type"], cross,
                                         "layout_detection" if cross else "parse")
    # Failed evaluation is never cached: transient failures must be retried.
    if evaluation["success"]:
        encoded = json.loads(json.dumps(evaluation, default=str))
        payload = {"key": key, "sha256": bench.digest(bench.canonical(encoded)), "evaluation": encoded}
        with path.open("x") as stream:
            json.dump(payload, stream)
    return evaluation, False


def metric_score(evaluation, metric):
    if not evaluation["success"]:
        return 0.0
    matches = [m["value"] for m in evaluation["metrics"] if m["metric_name"] == metric]
    if len(matches) != 1 or not math.isfinite(matches[0]) or not 0 <= matches[0] <= 1:
        raise ValueError(f"Required ParseBench metric missing/invalid: {metric}")
    return matches[0]


def run(args):
    manifest = load_manifest(args.manifest)
    if args.repetitions < 3:
        raise ValueError("At least three measured repetitions are required")
    mode = "replay" if args.ocr_cache else "off"
    snapshot = cache_snapshot(args.ocr_cache)
    args.output.mkdir(parents=True, exist_ok=False)
    args.score_cache.mkdir(parents=True, exist_ok=True)
    identity = bench.digest(bench.canonical(manifest["evaluator"]))
    result = {"version": 1, "role": args.role, "revision": args.revision,
              "created_at": datetime.now(timezone.utc).isoformat(), "corpus_sha256": bench.digest(bench.canonical(manifest)),
              "evaluator_sha256": bench.digest(Path(__file__).read_bytes() + Path(bench.__file__).read_bytes()),
              "binary_sha256": bench.digest(args.worker.read_bytes()), "binary": str(args.worker.resolve()),
              "protocol": {"profile": f"parsebench-single-pass-{mode}", "ocr_snapshot": snapshot,
                           "ocr_id": args.ocr_id, "language": args.language, "dpi": args.dpi,
                           "repetitions": args.repetitions, "timeout": args.timeout, "evaluator": identity},
              "environment": {"system": platform.system(), "release": platform.release(), "machine": platform.machine(),
                              "host": platform.node(), "cpu_count": os.cpu_count(), "python": platform.python_version(),
                              "runner_id": args.runner_id, "environment_id": args.environment_id},
              "cases": [], "accuracy_cases": [], "score_cache_hits": 0}
    outputs, resource_map = {}, {}
    for doc_id, doc in sorted(manifest["documents"].items()):
        source = (args.manifest.resolve().parent / doc["pdf"]).resolve()
        outputs[doc_id], resource_map[doc_id] = [], []
        for page in range(1, doc["pages"] + 1):
            page_id = f"{doc_id}-{page}"
            resource_map[doc_id].append(page_id)
            samples, parsed, status = [], None, "ok"
            for index in range(args.repetitions + 1):
                output, stderr = args.output / f"{page_id}-{index}.json", args.output / f"{page_id}-{index}.stderr"
                metrics = bench.measure(command(args, source, page, mode), output, stderr, args.timeout)
                raw = output.read_bytes()
                metrics.update(output=output.name, stderr=stderr.name, output_sha256=bench.digest(raw))
                samples.append(metrics)
                try:
                    parsed = json.loads(raw)
                    if parsed["page"] != page or parsed["total_pages"] != doc["pages"] or not parsed["markdown"].strip():
                        raise ValueError("Missing/wrong page")
                except (ValueError, KeyError, TypeError):
                    status = "failed"
                if metrics["exit_code"] != 0 or metrics["timed_out"]:
                    status = "failed"
            if status == "ok" and len({s["output_sha256"] for s in samples}) != 1:
                status = "unstable"
            if status == "ok":
                outputs[doc_id].append(parsed)
            result["cases"].append({"id": page_id, "status": status, "warmup": samples[0], "samples": samples[1:],
                                    "checks": [{"category": "completeness", "passed": status == "ok",
                                                "check": {"page": page, "pdf_sha256": doc_id}}]})
            print(f"{page_id[:12]} page {page}: {status}", flush=True)
    # Scoring starts after ALL timing, so evaluator CPU activity cannot pollute samples.
    started = time.monotonic()
    for unit in manifest["units"]:
        doc_id = unit["document"]
        source = (args.manifest.resolve().parent / manifest["documents"][doc_id]["pdf"]).resolve()
        resources = [c for c in result["cases"] if c["id"] in resource_map[doc_id]]
        status = "ok" if all(c["status"] == "ok" for c in resources) else "failed"
        evaluation = {"success": False, "metrics": [], "error": "Parser failed"}
        value = 0.0
        if status == "ok":
            evaluation, hit = evaluate(outputs[doc_id], unit, source, args.score_cache, identity)
            result["score_cache_hits"] += int(hit)
            value = metric_score(evaluation, unit["metric"])
            if not evaluation["success"]:
                status = "failed"
        evidence = bench.digest(unit["id"].encode()) + "-evaluation.json"
        (args.output / evidence).write_text(json.dumps(evaluation, indent=2, default=str))
        result["accuracy_cases"].append({"id": unit["id"], "status": status, "resource_ids": resource_map[doc_id],
                                        "evidence": evidence, "checks": [{"category": unit["test_case"]["group"],
                                        "score": value, "passed": value == 1, "check": {"metric": unit["metric"]}}]})
    if cache_snapshot(args.ocr_cache) != snapshot:
        raise ValueError("OCR cache changed during timing; discard run and recollect references")
    result["scoring_seconds"] = time.monotonic() - started
    result["accuracy"] = bench.accuracy_for_result(result)
    bench.validate_result(result)
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(f"Accuracy {result['accuracy']['overall']:.4f}; {result['score_cache_hits']} score cache hits")
    return 0 if bench.valid(result) else 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    prep = sub.add_parser("prepare")
    prep.add_argument("--dataset", type=Path, required=True)
    prep.add_argument("--output", type=Path, required=True)
    prep.add_argument("--groups", default=",".join(METRICS))
    prep.add_argument("--limit-per-category", type=int, default=0)
    prep.add_argument("--seed", default="pilot-v1")
    for name in ("prime", "run"):
        action = sub.add_parser(name)
        action.add_argument("--manifest", type=Path, required=True)
        action.add_argument("--worker", type=Path, required=True)
        action.add_argument("--output", type=Path, required=True)
        action.add_argument("--ocr-cache", type=Path, required=name == "prime")
        action.add_argument("--ocr-id")
        action.add_argument("--language", default="eng")
        action.add_argument("--dpi", type=float, default=150)
        action.add_argument("--timeout", type=float, default=60)
        if name == "prime":
            action.add_argument("--ocr-server-url")
        else:
            action.add_argument("--revision", required=True)
            action.add_argument("--role", choices=["candidate", "main"], default="candidate")
            action.add_argument("--runner-id", required=True)
            action.add_argument("--environment-id", required=True)
            action.add_argument("--repetitions", type=int, default=5)
            action.add_argument("--score-cache", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.action == "prepare":
            if args.limit_per_category < 0:
                raise ValueError("Subset size must be nonnegative")
            prepare(args)
            return 0
        if bool(args.ocr_cache) != bool(args.ocr_id):
            raise ValueError("--ocr-cache and --ocr-id must be supplied together")
        if not math.isfinite(args.timeout) or args.timeout <= 0 or not math.isfinite(args.dpi) or args.dpi <= 0:
            raise ValueError("Timeout and DPI must be finite and positive")
        return prime(args) if args.action == "prime" else run(args)
    except (ValueError, OSError, KeyError, TypeError) as exc:
        parser.exit(1, f"error: {exc}\n")


if __name__ == "__main__":
    sys.exit(main())
