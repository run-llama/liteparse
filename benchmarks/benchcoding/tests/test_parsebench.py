import importlib.util
import argparse
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parents[1]))
import bench
import parsebench as adapter

HAS_PARSEBENCH = importlib.util.find_spec("parse_bench") is not None


class AdapterTests(unittest.TestCase):
    def test_missing_metric_is_not_silently_averaged_away(self):
        with self.assertRaises(ValueError):
            adapter.metric_score({"success": True, "metrics": []}, "content_faithfulness")
        self.assertEqual(adapter.metric_score({"success": False, "metrics": []}, "content_faithfulness"), 0)
        with self.assertRaises(ValueError):
            adapter.metric_score({"success": True, "metrics": [{"metric_name": "x", "value": float('nan')}]}, "x")

    def test_cache_snapshot_detects_mutation(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "profile.json").write_text('{}')
            before = adapter.cache_snapshot(root)
            (root / "response.json").write_text('{"results": []}')
            self.assertNotEqual(before, adapter.cache_snapshot(root))
            self.assertEqual(adapter.cache_snapshot(root), adapter.cache_snapshot(root))

    def test_doc_category_accuracy_is_not_weighted_by_page_count(self):
        def unit(name, score):
            return {"id": name, "status": "ok", "checks": [{"category": "text", "passed": score == 1, "score": score}]}
        result = {"cases": [unit("page1", 1)] * 10, "accuracy_cases": [unit("doc1", 0), unit("doc2", 1)]}
        self.assertEqual(bench.accuracy_for_result(result)["overall"], .5)


@unittest.skipUnless(HAS_PARSEBENCH, "Install requirements-parsebench.txt for upstream integration tests")
class UpstreamTests(unittest.TestCase):
    def test_prepare_deduplicates_shared_pdfs_and_detects_changed_data(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = Path(__file__).parents[3] / "integration_tests_data/sample.pdf"
            (root / "sample.pdf").write_bytes(source.read_bytes())
            for group in ("text_content", "text_formatting"):
                row = {"pdf": "sample.pdf", "page": 1, "category": group,
                       "type": "present", "rule": {"text": "Sample PDF", "max_diffs": 0}}
                (root / f"{group}.jsonl").write_text(json.dumps(row) + '\n')
            output = root / "manifest.json"
            adapter.prepare(argparse.Namespace(dataset=root, output=output,
                groups="text_content,text_formatting", limit_per_category=0, seed="test"))
            manifest = adapter.load_manifest(output)
            self.assertEqual(len(manifest['documents']), 1)
            self.assertEqual(len(manifest['units']), 2)
            self.assertEqual(len({u['id'] for u in manifest['units']}), 2)
            (root / "sample.pdf").write_bytes(b'changed')
            with self.assertRaises(ValueError):
                adapter.load_manifest(output)

    def test_identity_survives_json_roundtrip(self):
        identity = adapter.evaluator_identity()
        self.assertEqual(identity, json.loads(json.dumps(identity)))

    def test_real_rules_and_score_cache_invalidation(self):
        from parse_bench.test_cases.schema import ParseTestCase
        source = Path(__file__).parents[3] / "integration_tests_data/sample.pdf"
        case = ParseTestCase(test_id="chart/synthetic", group="chart", file_path=source,
                             test_rules=[{"type": "present", "text": "hello", "max_diffs": 0}])
        unit = {"id": "chart:synthetic", "type": "parse", "test_case": case.model_dump(mode="json"), "metric": "rule_pass_rate"}
        pages = [{"page": 1, "width": 100, "height": 100, "markdown": "hello", "blocks": []}]
        with tempfile.TemporaryDirectory() as temp:
            cache = Path(temp)
            first, hit = adapter.evaluate(pages, unit, source, cache, "fixed-evaluator")
            self.assertFalse(hit)
            self.assertEqual(adapter.metric_score(first, "rule_pass_rate"), 1)
            second, hit = adapter.evaluate(pages, unit, source, cache, "fixed-evaluator")
            self.assertTrue(hit)
            self.assertEqual(second['metrics'], json.loads(json.dumps(first, default=str))['metrics'])
            pages[0]['markdown'] = "wrong"
            third, hit = adapter.evaluate(pages, unit, source, cache, "fixed-evaluator")
            self.assertFalse(hit)
            self.assertEqual(adapter.metric_score(third, "rule_pass_rate"), 0)
            _, hit = adapter.evaluate(pages, unit, source, cache, "another-evaluator")
            self.assertFalse(hit)
            for path in cache.glob('*.json'):
                path.write_text('{"key": "tampered"}')
            with self.assertRaises(ValueError):
                adapter.evaluate(pages, unit, source, cache, "fixed-evaluator")

    def test_normalization_keeps_page_indices_and_layout(self):
        pages = [{"page": 2, "width": 100, "height": 200, "markdown": "| A |\n| --- |\n| B |", "blocks": [
            {"kind": "paragraph", "text": "hello", "bbox": {"x": 10, "y": 20, "width": 50, "height": 30}}
        ]}]
        result = adapter.normalized_result(pages, {"id": "layout:example"}, Path("example.pdf"))
        self.assertEqual(result.output.pages[0].page_index, 1)
        self.assertEqual(result.output.layout_pages[0].page_number, 2)
        self.assertAlmostEqual(result.output.layout_pages[0].items[0].bbox.x, .1)
        self.assertIn('<table>', result.output.markdown)


if __name__ == '__main__':
    unittest.main()
