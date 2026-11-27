"""Guard story attribution and diagnostic normalization in the corpus harness."""
import tempfile
import unittest
from pathlib import Path
import zipfile

import transcription_baseline as bench


class TranscriptionBaselineTests(unittest.TestCase):
    def test_nested_field_projection_preserves_only_cached_results(self):
        self.assertEqual(bench.field_results('A\x13IF \x13X\x14Y\x15\x14Z\x15B'), ('AZB', 0))
        self.assertEqual(bench.field_results('A\x13X'), ('A', 1))
        self.assertEqual(bench.field_results('A\x14B'), ('AB', 1))

    def test_source_literal_object_character_is_retained(self):
        story = {'text': 'A\ufffc\ufffcB', 'segments': [
            {'kind': 'text', 'output_start': 0, 'output_end': 4},
            {'kind': 'unresolved_control', 'output_start': 4, 'output_end': 7},
            {'kind': 'text', 'output_start': 7, 'output_end': 8},
        ]}
        self.assertEqual(bench.without_unresolved_segments(story), 'A\ufffcB')

    def test_provenance_accepts_explicit_form_value_anchor(self):
        source = {'start_cp': 0, 'utf16_units': 1, 'text': 'x'}
        projection = {'text': 'Chosen', 'segments': [
            {'kind': 'generated_form_value', 'start_cp': 0, 'end_cp': 0,
             'output_start': 0, 'output_end': 6},
            {'kind': 'suppressed', 'start_cp': 0, 'end_cp': 1,
             'output_start': 6, 'output_end': 6},
        ]}
        self.assertTrue(bench.validate_projection(source, projection)['partition_valid'])
        projection['segments'][0]['kind'] = 'arbitrary_generated'
        with self.assertRaises(ValueError):
            bench.validate_projection(source, projection)

    def test_provenance_supports_generated_suppressed_and_supplementary_text(self):
        source = {'start_cp': 10, 'utf16_units': 4, 'text': 'A😀B'}
        projected = {'text': '1.😀B', 'segments': [
            {'kind': 'generated_list_label', 'start_cp': 10, 'end_cp': 10, 'output_start': 0, 'output_end': 2},
            {'kind': 'deleted_revision', 'start_cp': 10, 'end_cp': 11, 'output_start': 2, 'output_end': 2},
            {'kind': 'text', 'start_cp': 11, 'end_cp': 14, 'output_start': 2, 'output_end': 7},
        ]}
        result = bench.validate_projection(source, projected)
        self.assertEqual(result['generated_segments'], 1)
        self.assertEqual(result['source_units'], 4)
        for field, value in [('start_cp', 12), ('end_cp', 13),
                             ('output_start', 3), ('output_end', 6)]:
            with self.subTest(field=field):
                altered = dict(projected, segments=[dict(s) for s in projected['segments']])
                altered['segments'][-1][field] = value
                with self.assertRaises(ValueError):
                    bench.validate_projection(source, altered)
        projected['segments'][0]['kind'] = 'text'
        with self.assertRaises(ValueError):
            bench.validate_projection(source, projected)

    def test_provenance_rejects_surrogate_and_utf8_splits(self):
        source = {'start_cp': 0, 'utf16_units': 2, 'text': '😀'}
        for end_cp, out_end in [(1, 4), (2, 1)]:
            projected = {'text': '😀', 'segments': [
                {'kind': 'text', 'start_cp': 0, 'end_cp': end_cp,
                 'output_start': 0, 'output_end': out_end},
            ]}
            with self.assertRaises(ValueError):
                bench.validate_projection(source, projected)

    def test_story_coverage_includes_non_exported_stories(self):
        raw = {'kind': 'macro', 'start_cp': 12, 'utf16_units': 1, 'text': 'X'}
        projected = {'kind': 'macro', 'text': 'X', 'segments': [
            {'kind': 'text', 'start_cp': 12, 'end_cp': 13, 'output_start': 0, 'output_end': 1},
        ]}
        native = {'stories': [raw], 'transcription': {'stories': [projected]}}
        self.assertTrue(bench.validate_transcription(native)['macro']['partition_valid'])
        for stories in [[], [projected, projected], [dict(projected, kind='body')]]:
            with self.subTest(stories=stories):
                with self.assertRaises(ValueError):
                    bench.validate_transcription(dict(native, transcription={'stories': stories}))
        with self.assertRaises(ValueError):
            bench.validate_transcription(dict(native, stories=[raw, raw]))

    def test_inventory_and_order_are_distinct(self):
        result = bench.compare('a b c', 'cba')
        self.assertEqual(result['native_only_character_occurrences'], 0)
        self.assertEqual(result['lo_only_character_occurrences'], 0)
        self.assertFalse(result['same_normalized_order'])
        self.assertTrue(bench.compare('e\u0301\x07', 'é')['same_normalized_order'])

    def test_ooxml_story_attribution_and_table_text(self):
        ns = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"'
        document = f'''<w:document {ns}><w:body>
        <w:p><w:r><w:t>Body</w:t></w:r><w:del><w:r><w:delText>Deleted</w:delText></w:r></w:del></w:p>
        <w:tbl><w:tr><w:tc><w:p><w:r><w:t>Cell</w:t><w:softHyphen/><w:noBreakHyphen/><w:sym w:font="Symbol" w:char="F041"/></w:r></w:p></w:tc></w:tr></w:tbl>
        <mc:AlternateContent><mc:Choice Requires="wps"><w:p><w:r><w:txbxContent><w:p><w:r><w:t>Box</w:t></w:r></w:p></w:txbxContent></w:r></w:p></mc:Choice><mc:Fallback><w:p><w:r><w:txbxContent><w:p><w:r><w:t>Box</w:t></w:r></w:p></w:txbxContent></w:r></w:p></mc:Fallback></mc:AlternateContent>
        </w:body></w:document>'''
        header = f'<w:hdr {ns}><w:p><w:r><w:t>Header</w:t></w:r><w:txbxContent><w:p><w:r><w:t>HeaderBox</w:t></w:r></w:p></w:txbxContent></w:p></w:hdr>'
        notes = f'<w:footnotes {ns}><w:footnote w:type="separator"><w:p><w:r><w:t>Separator</w:t></w:r></w:p></w:footnote><w:footnote w:id="1"><w:p><w:r><w:t>Note</w:t></w:r></w:p></w:footnote></w:footnotes>'
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'test.docx'
            with zipfile.ZipFile(path, 'w') as archive:
                archive.writestr('word/document.xml', document)
                archive.writestr('word/header1.xml', header)
                archive.writestr('word/footnotes.xml', notes)
            diagnostics = {}
            result = {k: bench.normalized(v) for k, v in bench.ooxml_stories(path, diagnostics).items()}
        self.assertEqual(result['body'], 'BodyCell\u00ad\u2011')
        self.assertEqual(diagnostics['body']['unresolved_font_symbols'], 1)
        self.assertEqual(result['textboxes'], 'Box')
        self.assertEqual(result['headers'], 'Header')
        self.assertEqual(result['header_textboxes'], 'HeaderBox')
        self.assertEqual(result['footnotes'], 'Note')


if __name__ == '__main__':
    unittest.main()
