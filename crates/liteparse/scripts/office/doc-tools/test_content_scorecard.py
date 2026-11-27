"""Behavior checks for CP accounting and uncertainty classification."""
import unittest
import hashlib
import gzip
import json
import subprocess
import sys
import tempfile
from pathlib import Path

from content_scorecard import RangeCounter, analyze, check_references, choose_batch, compare_rows, subtract, summarize


def payload():
    return {
        'status': 'decoded',
        'stories': [{'kind': 'body', 'start_cp': 0, 'utf16_units': 4, 'text': 'ABCD'}],
        'hidden_properties': {'spans': [
            {'start_cp': 0, 'end_cp': 2, 'hidden': False},
            {'start_cp': 2, 'end_cp': 3, 'hidden': True},
            {'start_cp': 3, 'end_cp': 4, 'hidden': False},
        ]},
        'transcription': {'stories': [{'kind': 'body', 'text': '•D', 'segments': [
            {'start_cp': 0, 'end_cp': 1, 'output_start': 0, 'output_end': 0, 'kind': 'field_instruction'},
            {'start_cp': 1, 'end_cp': 2, 'output_start': 0, 'output_end': 0, 'kind': 'deleted_revision'},
            {'start_cp': 2, 'end_cp': 3, 'output_start': 0, 'output_end': 0, 'kind': 'hidden_text'},
            {'start_cp': 3, 'end_cp': 3, 'output_start': 0, 'output_end': 3, 'kind': 'generated_list_label'},
            {'start_cp': 3, 'end_cp': 4, 'output_start': 3, 'output_end': 4, 'kind': 'text'},
        ], 'diagnostics': []}]},
    }


class ScorecardTests(unittest.TestCase):
    def test_stored_geometry_text_is_uncertainty_without_story_loss(self):
        data = payload()
        before = analyze(data)
        data['geometry_text'] = {'status': 'source_text_available_display_order_and_visibility_unresolved',
                                 'shapes': [{'source_text': 'A😀', 'candidates': [{'text': 'A😀'}]}]}
        after = analyze(data)
        self.assertEqual(after['status'], 'partial_logical_extraction')
        self.assertEqual(after['stories'], before['stories'])
        self.assertEqual(after['component_errors'], {})
        self.assertEqual(after['component_uncertainties']['geometry_text']['stored_candidate_utf16_units'], 3)
        summary = summarize([after], {})
        self.assertEqual(summary['documents_with_component_uncertainty'], 1)
        self.assertEqual(summary['totals']['unclassified_empty_candidate_cp'], 0)
        data['geometry_text']['shapes'][0]['candidates'][0]['text'] = ''
        self.assertEqual(analyze(data)['status'], before['status'])
        data['geometry_text'] = {'status': 'unavailable', 'error': 'invalid source records'}
        row = analyze(data)
        self.assertEqual(row['component_uncertainties'], {})
        self.assertIn('geometry_text', row['component_errors'])

    def test_uncertainty_schema_must_be_recomputed_on_both_sides(self):
        old, new = analyze(payload()), analyze(payload())
        old['source_sha256'] = new['source_sha256'] = 'a' * 64
        del old['component_uncertainties']
        with self.assertRaisesRegex(ValueError, 'recompute both caches'):
            compare_rows([old], [new])

    def test_mixed_metric_schemas_cannot_be_reported_as_parser_changes(self):
        old = analyze(payload())
        new = analyze(payload())
        old['source_sha256'] = new['source_sha256'] = 'a' * 64
        del old['component_errors']
        with self.assertRaisesRegex(ValueError, 'recompute both caches'):
            compare_rows([old], [new])

    def test_projection_error_marks_accounted_text_partial_without_creating_loss(self):
        data = payload()
        before = analyze(data)
        data['list_projection_error'] = 'unsupported: list unindexed fields'
        after = analyze(data)
        self.assertEqual(before['status'], 'source_accounted_no_reported_gaps')
        self.assertEqual(after['status'], 'partial_logical_extraction')
        self.assertEqual(after['stories'], before['stories'])
        self.assertEqual(after['source_accounting'], 'complete')
        self.assertEqual(after['component_errors']['list_projection_error']['error'],
                         data['list_projection_error'])

    def test_unavailable_metadata_retains_errors_and_counts_documents_once(self):
        data = payload()
        data['fonts'] = {'status': 'unavailable', 'error': 'invalid: font charset'}
        data['headers'] = {'status': 'unavailable'}
        row = analyze(data)
        self.assertEqual(row['status'], 'partial_logical_extraction')
        self.assertEqual(row['component_errors']['headers'], {'status': 'unavailable', 'error': None})
        summary = summarize([row], {})
        self.assertEqual(summary['documents_with_component_errors'], 1)
        self.assertEqual(summary['component_error_document_counts_overlapping'], {'fonts': 1, 'headers': 1})
        self.assertEqual(summary['totals']['unclassified_empty_candidate_cp'], 0)

    def test_empty_errors_and_nested_authored_error_keys_do_not_change_status(self):
        data = payload()
        data['list_projection_error'] = None
        data['headers'] = {'status': 'decoded', 'error': ''}
        data['source_record'] = {'value': {'error': 'authored content'}}
        row = analyze(data)
        self.assertEqual(row['component_errors'], {})
        self.assertEqual(row['status'], 'source_accounted_no_reported_gaps')

    def test_suppressed_instructions_revisions_and_hidden_are_not_visible_loss(self):
        row = analyze(payload())
        story = row['stories'][0]
        self.assertEqual(story['source_cp_accounted'], 4)
        self.assertEqual(story['source_cp_with_output'], 1)
        self.assertEqual(story['empty_source_cp_by_kind'], {
            'field_instruction': 1, 'deleted_revision': 1, 'hidden_text': 1})
        self.assertEqual(story['unclassified_empty_candidate_cp'], 0)
        self.assertEqual(story['generated_zero_cp_anchors'], {'generated_list_label': 1})
        self.assertEqual(row['semantic_accuracy'], 'not_established')

    def test_generated_symbol_fields_are_accounted_separately(self):
        data = payload()
        data['transcription']['stories'][0]['segments'][3]['kind'] = 'generated_symbol_field'
        story = analyze(data)['stories'][0]
        self.assertEqual(story['generated_zero_cp_anchors'], {'generated_symbol_field': 1})
        self.assertEqual(story['source_cp_accounted'], 4)
        self.assertEqual(story['empty_candidate_cp_by_kind'], {'field_instruction': 1, 'deleted_revision': 1})

    def test_reference_tampering_and_out_of_cohort_sources_are_explicit(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'proof.json'
            path.write_bytes(b'{}')
            entry = {'source_sha256': 'a', 'path': str(path), 'sha256': hashlib.sha256(b'{}').hexdigest(),
                     'kind': 'source', 'scope': 'bounded raw-source proof'}
            self.assertEqual(check_references([entry], {'a'})[0]['status'], 'hash_verified')
            path.write_bytes(b'changed')
            self.assertEqual(check_references([entry], {'a'})[0]['status'], 'hash_changed')
            with self.assertRaises(ValueError):
                check_references([entry], {'b'})

    def test_unclassified_empty_nonhidden_text_is_review_candidate(self):
        data = payload()
        data['transcription']['stories'][0]['segments'][1]['kind'] = 'unprojected_tail'
        row = analyze(data)
        self.assertEqual(row['stories'][0]['unclassified_empty_candidate_cp'], 1)
        self.assertEqual(row['status'], 'partial_logical_extraction')

    def test_unknown_or_missing_visibility_prevents_no_reported_gaps_status(self):
        for spans in [[], [{'start_cp': 0, 'end_cp': 4, 'hidden': None}]]:
            data = payload()
            data['hidden_properties']['spans'] = spans
            self.assertEqual(analyze(data)['status'], 'partial_logical_extraction')

    def test_output_gap_duplicate_story_and_utf16_mismatch_are_rejected(self):
        data = payload()
        data['transcription']['stories'][0]['segments'][-1]['output_start'] = 2
        with self.assertRaises(ValueError):
            analyze(data)
        data = payload()
        data['stories'].append(data['stories'][0])
        with self.assertRaises(ValueError):
            analyze(data)
        data = payload()
        data['transcription']['stories'].append(data['transcription']['stories'][0])
        with self.assertRaises(ValueError):
            analyze(data)
        data = payload()
        data['stories'][0]['text'] = '😀BCD'
        with self.assertRaises(ValueError):
            analyze(data)

    def test_source_and_generated_denominators_stay_separate(self):
        row = analyze(payload())
        summary = summarize([row], {'helper_sha256': 'test'})
        self.assertEqual(summary['totals']['raw_utf16'], 4)
        self.assertEqual(summary['totals']['logical_utf16'], 2)
        self.assertEqual(summary['generated_zero_cp_anchors'], {'generated_list_label': 1})
        self.assertNotIn('ABCD', str(summary))

    def test_mixed_batch_covers_component_pairs_deterministically(self):
        rows = [
            {'source_sha256': 'a', 'status': 'partial', 'components': ['body']},
            {'source_sha256': 'b', 'status': 'partial', 'components': ['body', 'notes', 'fields']},
            {'source_sha256': 'c', 'status': 'partial', 'components': ['body', 'tables', 'fields']},
        ]
        batch = choose_batch(rows, 2)
        self.assertEqual({r['source_sha256'] for r in batch['rows']}, {'b', 'c'})
        self.assertEqual(batch, choose_batch(list(reversed(rows)), 2))

    def test_cached_evaluation_does_not_rewrite_external_receipts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cache = root / 'cache'
            cache.mkdir()
            source = 'a' * 64
            roster = root / 'roster.jsonl'
            roster.write_text(json.dumps({'source_sha256': source, 'source_path': '/unavailable', 'cohort': 'development'}) + '\n')
            data = cache / (source + '.json.gz')
            with gzip.open(data, 'wt') as stream:
                json.dump(payload(), stream)
            receipt = root / 'cache-manifest.json'
            receipt.write_text(json.dumps({source: hashlib.sha256(data.read_bytes()).hexdigest()}, separators=(',', ':')))
            original = receipt.read_bytes()
            (root / 'cache-meta.json').write_text(json.dumps({'helper_sha256': 'b' * 64,
                'roster_sha256': hashlib.sha256(roster.read_bytes()).hexdigest()}))
            command = [sys.executable, str(Path(__file__).with_name('content_scorecard.py')),
                '--roster', str(roster), '--cache', str(cache), '--output', str(root / 'out'),
                '--cached-only', '--expected-count', '1']
            subprocess.run(command, check=True, capture_output=True)
            summary = json.loads((root / 'out/summary.json').read_text())
            self.assertEqual(summary['schema_version'], 3)
            self.assertEqual(summary['provenance']['evaluator_schema_version'], summary['schema_version'])
            self.assertEqual(receipt.read_bytes(), original)
            self.assertTrue((root / 'out/evaluated-cache-manifest.json').exists())
            receipt.write_text(json.dumps({'c' * 64: 'd' * 64}))
            subprocess.run(command, check=True, capture_output=True)
            summary = json.loads((root / 'out/summary.json').read_text())
            self.assertEqual(summary['statuses'], {'accounting_or_harness_error': 1})

    def test_prefix_counts_preserve_gaps_and_half_open_boundaries(self):
        counter = RangeCounter([(2, 5), (4, 7), (10, 12)])
        self.assertEqual(counter.count(0, 2), 0)
        self.assertEqual(counter.count(2, 10), 5)
        self.assertEqual(counter.count(6, 11), 2)
        self.assertEqual(counter.count(12, 20), 0)

    def test_comparison_rejects_changed_raw_stories_or_cohorts(self):
        before = analyze(payload())
        before['source_sha256'] = 'a'
        after = analyze(payload())
        after['source_sha256'] = 'a'
        self.assertEqual(compare_rows([before], [after])['changed_documents'], [])
        after['stories'][0]['raw_text_sha256'] = 'changed'
        with self.assertRaises(ValueError):
            compare_rows([before], [after])
        with self.assertRaises(ValueError):
            compare_rows([before], [])

    def test_interval_subtraction_preserves_gaps_and_nested_overlaps(self):
        self.assertEqual(subtract([(0, 10), (20, 30)], [(2, 4), (3, 6), (9, 22), (28, 35)]),
                         [(0, 2), (6, 9), (22, 28)])


if __name__ == '__main__':
    unittest.main()
