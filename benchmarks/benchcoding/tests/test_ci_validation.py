import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parents[1]))
import ci_validate
import download_dataset


class AcceptanceTests(unittest.TestCase):
    def report(self, status='no_gain', span=30, regressions=None):
        return {'status': status, 'history_span_days': span, 'category_regressions': regressions or []}

    def test_hosted_timing_is_report_only_but_accuracy_is_enforced(self):
        ci_validate.acceptance(self.report(status='inconclusive'), False)
        ci_validate.acceptance(self.report(status='rejected'), False)
        with self.assertRaises(ValueError):
            ci_validate.acceptance(self.report(regressions=['table']), False)

    def test_dedicated_gate_requires_history_and_conclusive_resources(self):
        ci_validate.acceptance(self.report(), True)
        for report in [self.report(span=0), self.report(status='inconclusive'), self.report(status='rejected')]:
            with self.assertRaises(ValueError):
                ci_validate.acceptance(report, True)

    def test_replay_parity_covers_every_repetition(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            record, replay = root/'record', root/'replay'
            record.mkdir(); replay.mkdir()
            (record/'p1.json').write_bytes(b'expected')
            for name in ['warmup.json', 'sample.json']:
                (replay/name).write_bytes(b'expected')
            (replay/'result.json').write_text(json.dumps({'cases': [{'id': 'p1', 'warmup': {'output': 'warmup.json'},
                                                                    'samples': [{'output': 'sample.json'}]}]}))
            ci_validate.check_replay(record, replay)
            (replay/'sample.json').write_bytes(b'changed')
            with self.assertRaises(ValueError):
                ci_validate.check_replay(record, replay)


class DownloadTests(unittest.TestCase):
    def test_pinned_download_verified_cache_and_corruption_repair(self):
        calls = []
        revision = download_dataset.REVISIONS['smoke']
        metadata = {'sha': revision, 'siblings': [{'rfilename': 'text.jsonl'}, {'rfilename': 'docs/a.pdf'}]}
        def urlopen(url, **kwargs):
            calls.append(url)
            if '/api/' in url:
                return io.BytesIO(json.dumps(metadata).encode())
            self.assertIn(revision, url)
            return io.BytesIO(b'fixture')
        with tempfile.TemporaryDirectory() as temp, patch.object(download_dataset.urllib.request, 'urlopen', side_effect=urlopen):
            root = Path(temp)
            download_dataset.download('smoke', root)
            self.assertEqual(len(calls), 3)
            download_dataset.download('smoke', root)
            self.assertEqual(len(calls), 3)
            (root/'docs/a.pdf').write_bytes(b'corrupted')
            download_dataset.download('smoke', root)
            self.assertEqual((root/'docs/a.pdf').read_bytes(), b'fixture')
            (root/'unexpected.jsonl').write_text('{}')
            with self.assertRaises(ValueError):
                download_dataset.download('smoke', root)

    def test_download_rejects_traversal(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(ValueError):
                download_dataset.checked_path(Path(temp), '../escape.pdf')


if __name__ == '__main__':
    unittest.main()
