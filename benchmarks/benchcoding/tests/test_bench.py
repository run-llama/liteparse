import copy
from datetime import datetime, timedelta, timezone
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("bench", Path(__file__).parents[1] / "bench.py")
bench = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bench)


def result(cpu=1.0, ram=1000, accuracy=True, role="candidate", revision="candidate"):
    checks = [{"category": "text", "passed": accuracy, "check": {"kind": "contains", "category": "text", "text": "hello"}}]
    sample = {"cpu_seconds": cpu, "peak_rss_bytes": ram, "wall_seconds": cpu,
              "exit_code": 0, "timed_out": False, "output_sha256": "same"}
    case = {"id": "page1", "status": "ok", "checks": checks,
            "warmup": copy.deepcopy(sample), "samples": [copy.deepcopy(sample) for _ in range(3)]}
    return {"version": 1, "role": role, "revision": revision,
            "created_at": datetime.now(timezone.utc).isoformat(),
            "corpus_sha256": "corpus", "evaluator_sha256": "eval",
            "protocol": {"repetitions": 3}, "environment": {"host": "test"},
            "cases": [case], "accuracy": bench.aggregate_accuracy([case])}


class AccuracyTests(unittest.TestCase):
    def test_full_text_penalizes_missing_and_extra_content(self):
        self.assertEqual(bench.text_accuracy("a b c", "a b c"), 1)
        self.assertAlmostEqual(bench.text_accuracy("a b c", "a c"), 2 / 3)
        self.assertEqual(bench.text_accuracy("a b", "a b invented invented"), .5)
        self.assertEqual(bench.text_accuracy("a b", ""), 0)
        scored = bench.score("a c", [{"kind": "full_text", "category": "text", "text": "a b c"}])
        self.assertAlmostEqual(bench.aggregate_accuracy([{"status": "ok", "checks": scored}])["overall"], 2 / 3)

    def test_order_and_normalization(self):
        checks = [{"kind": "ordered", "category": "order", "texts": ["café", "next"]}]
        self.assertTrue(bench.score("cafe\u0301\n  next", checks)[0]["passed"])
        self.assertFalse(bench.score("next café", checks)[0]["passed"])
        checks[0]["texts"] = ["same", "same"]
        self.assertFalse(bench.score("same", checks)[0]["passed"])

    def test_markdown_requires_complete_line(self):
        check = [{"kind": "markdown", "category": "structure", "text": "# Heading"}]
        self.assertTrue(bench.score("# Heading\nBody", check)[0]["passed"])
        self.assertFalse(bench.score("## Heading\nBody", check)[0]["passed"])

    def test_absence(self):
        self.assertFalse(bench.score("bad", [{"kind": "absent", "category": "text", "text": "bad"}])[0]["passed"])

    def test_category_balancing_and_failed_pages(self):
        cases = [{"status": "ok", "checks": [{"category": "text", "passed": True}] * 10 + [{"category": "table", "passed": False}]}]
        self.assertEqual(bench.aggregate_accuracy(cases)["overall"], 0.5)
        cases[0]["status"] = "failed"
        self.assertEqual(bench.aggregate_accuracy(cases)["overall"], 0)

    def test_smoke_manifest(self):
        cases, files, fingerprint = bench.load_corpus(Path(__file__).parents[1] / "smoke.json")
        self.assertEqual(len(cases), 1)
        self.assertEqual(len(fingerprint), 64)
        self.assertTrue(files[cases[0]["id"]]["path"].is_file())


class ComparisonTests(unittest.TestCase):
    def setUp(self):
        self.main = result(accuracy=False, role="main", revision="main")

    def test_accuracy_gain_with_allowed_slowdown(self):
        self.assertEqual(bench.compare(result(cpu=1.04), self.main, [])["status"], "eligible_accuracy_gain")

    def test_cpu_and_memory_gates(self):
        for candidate in [result(cpu=1.051), result(ram=1051)]:
            self.assertEqual(bench.compare(candidate, self.main, [])["status"], "rejected")

    def test_historical_limit_prevents_creeping_slowdown(self):
        old = result(cpu=.9, role="main", revision="old")
        old["created_at"] = (datetime.fromisoformat(self.main["created_at"]) - timedelta(days=10)).isoformat()
        report = bench.compare(result(cpu=1.0), self.main, [old])
        self.assertEqual(report["status"], "rejected")
        self.assertEqual(report["fastest_main_revision"], "old")

    def test_expired_future_incompatible_and_candidate_history_excluded(self):
        history = []
        for days in [-31, 1]:
            old = result(cpu=.1, role="main")
            old["created_at"] = (datetime.fromisoformat(self.main["created_at"]) + timedelta(days=days)).isoformat()
            history.append(old)
        history.append(result(cpu=.1))
        other = result(cpu=.1, role="main")
        other["environment"] = {"host": "other"}
        history.append(other)
        report = bench.compare(result(), self.main, history)
        self.assertEqual(report["status"], "eligible_accuracy_gain")
        self.assertEqual(report["excluded_history_count"], 4)

    def test_noise_is_inconclusive(self):
        candidate = result(cpu=1.04)
        candidate["cases"][0]["samples"][0]["cpu_seconds"] = 1.06
        self.assertEqual(bench.compare(candidate, self.main, [])["status"], "inconclusive")

    def test_speed_gain_and_no_gain(self):
        self.assertEqual(bench.compare(result(cpu=.9, accuracy=False), self.main, [])["status"], "eligible_speed_gain")
        self.assertEqual(bench.compare(result(accuracy=False), self.main, [])["status"], "no_gain")

    def test_accuracy_loss_cannot_be_bought_with_speed(self):
        main = result(role="main")
        self.assertEqual(bench.compare(result(cpu=.5, accuracy=False), main, [])["status"], "rejected")

    def test_failure_is_not_dropped(self):
        candidate = result(cpu=.5)
        candidate["cases"][0]["status"] = "failed"
        candidate["accuracy"] = bench.aggregate_accuracy(candidate["cases"])
        self.assertEqual(bench.compare(candidate, self.main, [])["status"], "rejected")

    def test_tampering_or_incompatible_evidence_errors(self):
        for field in ["environment", "corpus_sha256", "evaluator_sha256"]:
            candidate = result()
            candidate[field] = "different"
            with self.assertRaises(ValueError):
                bench.compare(candidate, self.main, [])
        candidate = result()
        candidate["accuracy"]["overall"] = .8
        with self.assertRaises(ValueError):
            bench.compare(candidate, self.main, [])

    def test_nan_missing_samples_and_false_success_rejected(self):
        for mutation in [lambda c: c["samples"].pop(),
                         lambda c: c["samples"][0].update(cpu_seconds=float("nan")),
                         lambda c: c["warmup"].update(exit_code=1),
                         lambda c: c["samples"][0].update(output_sha256="changed")]:
            candidate = result()
            mutation(candidate["cases"][0])
            with self.assertRaises(ValueError):
                bench.compare(candidate, self.main, [])

    def test_changed_checks_rejected_even_with_matching_fingerprint(self):
        candidate = result()
        candidate["cases"][0]["checks"][0]["check"]["text"] = "easier assertion"
        with self.assertRaises(ValueError):
            bench.compare(candidate, self.main, [])

    def test_report_labels_bootstrap(self):
        report = bench.compare(result(), self.main, [])
        self.assertTrue(report["bootstrap_history"])
        self.assertIn("Bootstrap only", bench.render_report(report))

    def test_category_regression_overrides_aggregate_gain(self):
        candidate = result()
        for item, passed in [(candidate, False), (self.main, True)]:
            item["cases"][0]["checks"].append({"category": "tables", "passed": passed,
                "check": {"kind": "contains", "category": "tables", "text": "total"}})
            item["accuracy"] = bench.aggregate_accuracy(item["cases"])
        report = bench.compare(candidate, self.main, [])
        self.assertEqual(report["status"], "rejected")
        self.assertEqual(report["category_regressions"], ["tables"])


@unittest.skipUnless(hasattr(bench.os, "wait4"), "Unix measurement")
class MeasurementTests(unittest.TestCase):
    def measured(self, code, timeout=3):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            metrics = bench.measure([sys.executable, "-c", code], root / "out", root / "err", timeout)
            return metrics, (root / "out").read_text(), (root / "err").read_text()

    def test_real_process_output_cpu_and_rss(self):
        metrics, output, _ = self.measured("print(sum(range(1000000)))")
        self.assertEqual(output.strip(), "499999500000")
        self.assertEqual(metrics["exit_code"], 0)
        self.assertGreater(metrics["cpu_seconds"], 0)
        self.assertGreater(metrics["peak_rss_bytes"], 0)

    def test_failure_and_timeout(self):
        metrics, _, _ = self.measured("raise SystemExit(7)")
        self.assertEqual(metrics["exit_code"], 7)
        metrics, _, _ = self.measured("import time; time.sleep(10)", .1)
        self.assertTrue(metrics["timed_out"])
        self.assertLess(metrics["wall_seconds"], 2)

    def test_runner_keeps_empty_and_unstable_outputs_ineligible(self):
        # Run the complete CLI against an executable test double, no native dependencies.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, body, expected in [
                ("empty", "", "failed"),
                ("unstable", "import time\nprint(time.time_ns())", "unstable"),
                ("failure", "raise SystemExit(2)", "failed"),
                ("success", "print('This is a simple PDF file. Fun fun fun.')", "ok"),
            ]:
                binary = root / name
                binary.write_text(f"#!{sys.executable}\n{body}\n")
                binary.chmod(0o755)
                proc = bench.subprocess.run([sys.executable, str(Path(bench.__file__)), "run", "--binary", str(binary),
                    "--corpus", str(Path(bench.__file__).with_name("smoke.json")), "--output", str(root / (name + '-run')),
                    "--revision", "test", "--runner-id", "test", "--environment-id", "test", "--repetitions", "3"], capture_output=True)
                evidence = json.loads((root / (name + '-run') / "result.json").read_text())
                self.assertEqual(evidence["cases"][0]["status"], expected, proc.stderr)
                self.assertEqual(proc.returncode, 0 if expected == "ok" else 2)


if __name__ == "__main__":
    unittest.main()
