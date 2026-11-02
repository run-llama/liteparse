#!/usr/bin/env python3
"""Local, standard-library-only accuracy/resource experiment harness (Unix)."""
from __future__ import annotations

import argparse
from collections import defaultdict
from datetime import datetime, timedelta, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import signal
import statistics
import subprocess
import time
import unicodedata

VERSION = 1
FLAGS = ["--no-ocr", "--format", "markdown", "--image-mode", "off", "--no-links", "--num-workers", "1", "--quiet"]


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical(value) -> bytes:
    return json.dumps(value, sort_keys=True, ensure_ascii=False).encode()


def normalize(text: str) -> str:
    return " ".join(unicodedata.normalize("NFC", text).split())


def load_corpus(path: Path):
    manifest = json.loads(path.read_text())
    cases = manifest["cases"]
    if not cases or manifest.get("version") != VERSION:
        raise ValueError("Expected a nonempty version 1 corpus")
    ids = set()
    files = {}
    for case in cases:
        if not re.fullmatch(r"[A-Za-z0-9_-]+", case["id"]) or case["id"] in ids:
            raise ValueError("Case IDs must be unique safe filenames")
        ids.add(case["id"])
        if type(case["page"]) is not int or case["page"] < 1 or not case["checks"]:
            raise ValueError("Each case needs a positive page and accuracy checks")
        source = (path.parent / case["pdf"]).resolve()
        if source.suffix.lower() != ".pdf":
            raise ValueError("Only native PDFs are supported")
        files[case["id"]] = {"path": source, "sha256": digest(source.read_bytes())}
        for check in case["checks"]:
            if check.get("kind") not in {"contains", "absent", "ordered", "markdown", "full_text"}:
                raise ValueError("Unknown accuracy check")
            if not isinstance(check.get("category"), str) or not check["category"]:
                raise ValueError("Every check needs a category")
            if check["kind"] == "ordered":
                if len(check.get("texts", [])) < 2 or not all(isinstance(t, str) and normalize(t) for t in check["texts"]):
                    raise ValueError("Ordered checks need at least two nonempty strings")
            elif not isinstance(check.get("text"), str) or not normalize(check["text"]):
                raise ValueError("Checks need nonempty text")
    fingerprint = digest(canonical({"manifest": manifest, "files": {k: v["sha256"] for k, v in files.items()}}))
    return cases, files, fingerprint


def text_accuracy(expected: str, actual: str) -> float:
    """Normalized token edit accuracy: penalizes omissions AND hallucinations."""
    left, right = normalize(expected).split(), normalize(actual).split()
    denominator = max(len(left), len(right), 1)
    if len(left) < len(right):
        left, right = right, left
    row = list(range(len(right) + 1))
    for i, token in enumerate(left, 1):
        next_row = [i]
        for j, other in enumerate(right, 1):
            next_row.append(min(next_row[-1] + 1, row[j] + 1, row[j - 1] + (token != other)))
        row = next_row
    return 1 - row[-1] / denominator


def score(text: str, checks: list[dict]) -> list[dict]:
    normalized = normalize(text)
    results = []
    for check in checks:
        kind = check["kind"]
        value = None
        if kind == "full_text":
            value = text_accuracy(check["text"], text)
            passed = value == 1
        elif kind == "ordered":
            cursor = 0
            passed = True
            for part in check["texts"]:
                index = normalized.find(normalize(part), cursor)
                if index < 0:
                    passed = False
                    break
                cursor = index + len(normalize(part))
        elif kind == "markdown":
            # Exact line check preserves Markdown syntax, indentation and table pipes.
            passed = check["text"] in text.splitlines()
        else:
            passed = normalize(check["text"]) in normalized
            if kind == "absent":
                passed = not passed
        results.append({"category": check["category"], "passed": passed,
                        "score": float(passed) if value is None else value, "check": check})
    return results


def measure(command: list[str], output: Path, stderr: Path, timeout: float) -> dict:
    """wait4 accounts for native threads; peak RSS is NOT aggregate tree memory.

    Native PDF/no-OCR execution is the supported workload. This is a local
    measurement tool, not a security boundary for untrusted submissions.
    """
    if not hasattr(os, "wait4"):
        raise ValueError("Resource measurement requires Linux or macOS")
    start = time.monotonic()
    timed_out = False
    with output.open("wb") as out, stderr.open("wb") as err:
        proc = subprocess.Popen(command, stdout=out, stderr=err, start_new_session=True)
        try:
            while True:
                pid, status, usage = os.wait4(proc.pid, os.WNOHANG)
                if pid:
                    break
                if time.monotonic() - start >= timeout:
                    timed_out = True
                    os.killpg(proc.pid, signal.SIGKILL)
                    _, status, usage = os.wait4(proc.pid, 0)
                    break
                time.sleep(0.005)
        finally:
            # Also clean up unexpected descendants after the parent exits.
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            if 'status' not in locals():
                _, status, usage = os.wait4(proc.pid, 0)
            proc.returncode = os.waitstatus_to_exitcode(status)
    return {
        "exit_code": proc.returncode, "timed_out": timed_out,
        "cpu_seconds": usage.ru_utime + usage.ru_stime,
        "wall_seconds": time.monotonic() - start,
        "peak_rss_bytes": int(usage.ru_maxrss * (1 if platform.system() == "Darwin" else 1024)),
    }


def aggregate_accuracy(cases: list[dict]) -> dict:
    # Equal category weights; within each category, equal page weights.
    categories = defaultdict(list)
    for case in cases:
        per_page = defaultdict(list)
        for check in case["checks"]:
            per_page[check["category"]].append(check.get("score", float(check["passed"])) if case["status"] == "ok" else 0.0)
        for category, values in per_page.items():
            categories[category].append(statistics.mean(values))
    means = {key: statistics.mean(values) for key, values in sorted(categories.items())}
    return {"overall": statistics.mean(means.values()), "categories": means}


def run(args):
    if args.repetitions < 3 or args.timeout <= 0 or not math.isfinite(args.timeout):
        raise ValueError("Use at least 3 repetitions and a finite positive timeout")
    binary = args.binary.resolve(strict=True)
    cases, files, corpus_hash = load_corpus(args.corpus.resolve())
    # Exclusive creation protects prior evidence from accidental overwrites.
    args.output.mkdir(parents=True, exist_ok=False)
    protocol = {"flags": FLAGS, "repetitions": args.repetitions, "timeout": args.timeout,
                "mode": "isolated-page-warm-cache", "measurement": "wait4-native-process-v1"}
    environment = {"system": platform.system(), "release": platform.release(),
                   "machine": platform.machine(), "host": platform.node(), "cpu_count": os.cpu_count(),
                   "python": platform.python_version(), "runner_id": args.runner_id,
                   "environment_id": args.environment_id}
    result = {"version": VERSION, "role": args.role, "revision": args.revision,
              "created_at": datetime.now(timezone.utc).isoformat(), "corpus_sha256": corpus_hash,
              "evaluator_sha256": digest(Path(__file__).read_bytes()), "protocol": protocol,
              "environment": environment, "binary_sha256": digest(binary.read_bytes()),
              "binary": str(binary), "cases": []}
    for case in cases:
        command = [str(binary), "parse", str(files[case["id"]]["path"]),
                   "--target-pages", str(case["page"]), "--max-pages", str(case["page"]), *FLAGS]
        samples, texts = [], []
        # Untimed-for-ranking warmup is still recorded, including failures.
        for index in range(args.repetitions + 1):
            stem = f"{case['id']}-{index}"
            output, stderr = args.output / f"{stem}.md", args.output / f"{stem}.stderr"
            metrics = measure(command, output, stderr, args.timeout)
            raw = output.read_bytes()
            metrics.update(output=output.name, stderr=stderr.name, output_sha256=digest(raw))
            samples.append(metrics)
            texts.append(raw.decode("utf-8", errors="replace"))
        failed = any(s["exit_code"] != 0 or s["timed_out"] for s in samples)
        unstable = len({s["output_sha256"] for s in samples}) != 1
        empty = not all(normalize(t) for t in texts)
        status = "failed" if failed or empty else "unstable" if unstable else "ok"
        result["cases"].append({"id": case["id"], "pdf_sha256": files[case["id"]]["sha256"],
                                "page": case["page"], "status": status, "command": command,
                                "warmup": samples[0], "samples": samples[1:],
                                "checks": score(texts[-1], case["checks"])})
        print(f"{case['id']}: {status}", flush=True)
    result["accuracy"] = aggregate_accuracy(result["cases"])
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(f"Accuracy: {result['accuracy']['overall']:.4f}; evidence: {args.output}")
    return 0 if all(c["status"] == "ok" for c in result["cases"]) else 2


def compatible(a: dict, b: dict) -> bool:
    return all(a[key] == b[key] for key in ("version", "corpus_sha256", "evaluator_sha256", "protocol", "environment")) and [c["id"] for c in a["cases"]] == [c["id"] for c in b["cases"]]


def validate_result(result: dict):
    if result["version"] != VERSION or not result["cases"]:
        raise ValueError("Invalid result version or empty cases")
    if len({c["id"] for c in result["cases"]}) != len(result["cases"]):
        raise ValueError("Duplicate result case IDs")
    for case in result["cases"]:
        if len(case["samples"]) != result["protocol"]["repetitions"] or len(case["samples"]) < 3:
            raise ValueError("Missing resource repetitions")
        if not case["checks"]:
            raise ValueError("Missing accuracy checks")
        for check in case["checks"]:
            value = check.get("score", float(check["passed"]))
            if type(check["passed"]) is not bool or not math.isfinite(value) or not 0 <= value <= 1:
                raise ValueError("Invalid accuracy score")
        for sample in [case["warmup"], *case["samples"]]:
            for key in ("cpu_seconds", "peak_rss_bytes", "wall_seconds"):
                if not math.isfinite(sample[key]) or sample[key] <= 0:
                    raise ValueError(f"Invalid {key}")
        if case["status"] == "ok" and (
            any(s["exit_code"] != 0 or s["timed_out"] for s in [case["warmup"], *case["samples"]])
            or len({s["output_sha256"] for s in [case["warmup"], *case["samples"]]}) != 1
        ):
            raise ValueError("Successful status contradicts sample evidence")
    if result["accuracy"] != aggregate_accuracy(result["cases"]):
        raise ValueError("Accuracy does not match per-page evidence")


def valid(result: dict) -> bool:
    return all(c["status"] == "ok" for c in result["cases"])


def total_cpu(result: dict) -> float:
    return sum(statistics.median(s["cpu_seconds"] for s in c["samples"]) for c in result["cases"])


def bound(candidate: list[float], reference: list[float], factor: float) -> str:
    # Observed ranges are a conservative noise guard, NOT confidence intervals.
    if max(candidate) <= factor * min(reference):
        return "pass"
    if min(candidate) > factor * max(reference):
        return "fail"
    return "inconclusive"


def compare(candidate: dict, main: dict, history: list[dict]) -> dict:
    for item in [candidate, main, *history]:
        validate_result(item)
    if main["role"] != "main" or not valid(main) or not compatible(candidate, main):
        raise ValueError("Need a valid, compatible current-main reference")
    anchor = datetime.fromisoformat(main["created_at"])
    if anchor.tzinfo is None:
        raise ValueError("Reference timestamps must include timezone")
    eligible = [main]
    excluded = 0
    for item in history:
        when = datetime.fromisoformat(item["created_at"])
        if when.tzinfo is None:
            raise ValueError("Reference timestamps must include timezone")
        if item["role"] == "main" and valid(item) and compatible(candidate, item) and anchor - timedelta(days=30) <= when <= anchor:
            eligible.append(item)
        else:
            excluded += 1
    fastest = min(eligible, key=total_cpu)
    gates = []
    for cand, current, fast in zip(candidate["cases"], main["cases"], fastest["cases"]):
        if any([c["check"] for c in cand["checks"]] != [c["check"] for c in ref["checks"]] for ref in (current, fast)):
            raise ValueError("Per-page accuracy checks differ")
        for metric, reference, factor, label in (
            ("cpu_seconds", current, 1.05, "cpu_current_main"),
            ("cpu_seconds", fast, 1.10, "cpu_fastest_main_30d"),
            ("peak_rss_bytes", current, 1.05, "ram_current_main"),
        ):
            values = [s[metric] for s in cand["samples"]]
            baseline = [s[metric] for s in reference["samples"]]
            gates.append({"case": cand["id"], "gate": label, "status": bound(values, baseline, factor),
                          "candidate_range": [min(values), max(values)],
                          "reference_range": [min(baseline), max(baseline)], "factor": factor})
    delta = candidate["accuracy"]["overall"] - main["accuracy"]["overall"]
    categories = main["accuracy"]["categories"]
    if categories.keys() != candidate["accuracy"]["categories"].keys():
        raise ValueError("Accuracy categories differ")
    regressions = [k for k, v in categories.items() if candidate["accuracy"]["categories"][k] < v]
    # Require a speed gain bigger than the observed spread, not a lucky median.
    faster = sum(max(s["cpu_seconds"] for s in c["samples"]) for c in candidate["cases"]) < sum(min(s["cpu_seconds"] for s in c["samples"]) for c in main["cases"])
    if not valid(candidate) or regressions or any(g["status"] == "fail" for g in gates):
        status = "rejected"
    elif any(g["status"] == "inconclusive" for g in gates):
        status = "inconclusive"
    elif delta > 0:
        status = "eligible_accuracy_gain"
    elif delta == 0 and faster:
        status = "eligible_speed_gain"
    else:
        status = "no_gain"
    return {"status": status, "candidate_revision": candidate["revision"], "main_revision": main["revision"],
            "fastest_main_revision": fastest["revision"], "main_reference_count": len(eligible),
            "excluded_history_count": excluded, "history_window_days": 30,
            "history_span_days": (anchor - min(datetime.fromisoformat(r["created_at"]) for r in eligible)).total_seconds() / 86400,
            "bootstrap_history": len({r["revision"] for r in eligible}) == 1,
            "accuracy_delta": delta, "candidate_accuracy": candidate["accuracy"], "main_accuracy": main["accuracy"],
            "cpu_speedup": total_cpu(main) / total_cpu(candidate), "category_regressions": regressions,
            "gates": gates, "note": "Local evidence only; eligibility is not maintainer acceptance. Observed ranges are not confidence intervals."}


def render_report(report: dict) -> str:
    lines = ["# Local accuracy / resource comparison", "", f"Status: **{report['status']}**", "",
             f"Candidate: `{report['candidate_revision']}`", f"Current main: `{report['main_revision']}`",
             f"Fastest main in supplied 30-day history: `{report['fastest_main_revision']}`", "",
             f"Accuracy change: {report['accuracy_delta']:+.4f}. CPU speedup: {report['cpu_speedup']:.3f}x.", "",
             "| Accuracy category | Main | Candidate |", "| --- | ---: | ---: |"]
    for key, value in report["main_accuracy"]["categories"].items():
        lines.append(f"| {key} | {value:.4f} | {report['candidate_accuracy']['categories'][key]:.4f} |")
    lines += ["", "| Page | Resource gate | Status |", "| --- | --- | --- |"]
    for gate in report["gates"]:
        lines.append(f"| {gate['case']} | {gate['gate']} | {gate['status']} |")
    lines += ["", f"History: {report['main_reference_count']} supplied compatible runs spanning {report['history_span_days']:.1f} days; {report['excluded_history_count']} excluded.", ""]
    if report["bootstrap_history"]:
        lines += ["Bootstrap only: there is no distinct historical main revision yet.", ""]
    lines.append(report["note"])
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    runner = sub.add_parser("run", help="Measure and score the same page outputs")
    runner.add_argument("--binary", type=Path, required=True)
    runner.add_argument("--corpus", type=Path, required=True)
    runner.add_argument("--output", type=Path, required=True)
    runner.add_argument("--revision", required=True, help="Exact source commit used to build the binary (operator attestation)")
    runner.add_argument("--role", choices=["candidate", "main"], default="candidate")
    runner.add_argument("--runner-id", required=True, help="Stable identity of the dedicated benchmark machine")
    runner.add_argument("--environment-id", required=True, help="Pinned toolchain/native dependency environment identity")
    runner.add_argument("--repetitions", type=int, default=5)
    runner.add_argument("--timeout", type=float, default=60)
    comparison = sub.add_parser("compare", help="Check gates and report accuracy/speed tradeoffs")
    comparison.add_argument("--candidate", type=Path, required=True)
    comparison.add_argument("--main", type=Path, required=True)
    comparison.add_argument("--history", type=Path, nargs="*", default=[])
    comparison.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.action == "run":
            return run(args)
        paths = [args.candidate, args.main, *args.history]
        data = [json.loads(p.read_text()) for p in paths]
        report = compare(data[0], data[1], data[2:])
        report["evidence"] = [{"path": str(p), "sha256": digest(p.read_bytes())} for p in paths]
        markdown_path = Path(str(args.output) + ".md")
        if args.output.exists() or markdown_path.exists():
            raise ValueError("Comparison output already exists")
        with args.output.open("x") as stream:
            stream.write(json.dumps(report, indent=2) + "\n")
        with markdown_path.open("x") as stream:
            stream.write(render_report(report))
        print(f"{report['status']}: accuracy delta {report['accuracy_delta']:+.4f}, CPU speedup {report['cpu_speedup']:.3f}x")
        print(f"Current main: {report['main_revision']}; fastest main: {report['fastest_main_revision']}")
        return 0 if report["status"].startswith("eligible_") else 3 if report["status"] == "inconclusive" else 2
    except (ValueError, KeyError, OSError, TypeError) as exc:
        parser.exit(1, f"error: {exc}\n")


if __name__ == "__main__":
    raise SystemExit(main())
