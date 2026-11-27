#!/usr/bin/env python3
"""Compare native raw stories with provenance-verified LibreOffice OOXML exports.

This is a disagreement detector, not a transcription accuracy score. Native raw
fields/revisions and LibreOffice's visible export have different semantics. Text
is read in memory; reports contain hashes/counts and source-local artifact paths.
"""
import argparse
from collections import Counter, defaultdict
from difflib import SequenceMatcher
import hashlib
import json
from pathlib import Path
import re
import subprocess
import time
import unicodedata
import xml.etree.ElementTree as ET
import zipfile

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
MC = '{http://schemas.openxmlformats.org/markup-compatibility/2006}'
KINDS = ('body', 'footnotes', 'headers', 'comments', 'endnotes', 'textboxes', 'header_textboxes')


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def digest(text):
    return hashlib.sha256(text.encode()).hexdigest()


def normalized(text):
    return ''.join(c for c in unicodedata.normalize('NFC', text)
                   if not c.isspace() and unicodedata.category(c) != 'Cc')


def field_results(text):
    """Project nested field controls to cached results; flag unbalanced controls.

    Formatting-based hidden/deleted text remains unresolved. This projection is
    diagnostic only and must not be interpreted as the intended Word view.
    """
    stack, out, invalid = [], [], 0
    for char in text:
        if char == '\x13':
            stack.append(False)
        elif char == '\x14':
            if stack:
                stack[-1] = True
            else:
                invalid += 1
        elif char == '\x15':
            if stack:
                stack.pop()
            else:
                invalid += 1
        elif all(stack):
            out.append(char)
    return ''.join(out), invalid + len(stack)


def validate_projection(source, projected):
    """Check UTF-16 source ownership and UTF-8 output coverage independently.

    Generated labels anchor at the next source position without consuming it.
    Suppressed text consumes source positions without producing output bytes.
    This verifies provenance accounting, not semantic transcription accuracy.
    """
    cp = source['start_cp']
    source_end = cp + source['utf16_units']
    source_bytes = source['text'].encode('utf-16-le')
    if len(source_bytes) // 2 != source['utf16_units']:
        raise ValueError('source UTF-16 length mismatch')
    encoded = projected['text'].encode('utf-8')
    offset = 0
    generated = 0
    for segment in projected['segments']:
        start, end = segment['start_cp'], segment['end_cp']
        out_start, out_end = segment['output_start'], segment['output_end']
        if start != cp or end < start or end > source_end:
            raise ValueError('source partition gap, overlap or bounds')
        if out_start != offset or out_end < out_start or out_end > len(encoded):
            raise ValueError('output partition gap, overlap or bounds')
        encoded[out_start:out_end].decode('utf-8')
        if start == end:
            if segment['kind'] not in ('generated_list_label', 'generated_form_value', 'generated_symbol_field') or out_start == out_end:
                raise ValueError('unsupported zero-width source segment')
            generated += 1
        else:
            a = (start - source['start_cp']) * 2
            b = (end - source['start_cp']) * 2
            source_bytes[a:b].decode('utf-16-le')
        cp, offset = end, out_end
    if cp != source_end or offset != len(encoded):
        raise ValueError('incomplete source or output coverage')
    return {'source_units': source['utf16_units'], 'output_bytes': offset,
            'generated_segments': generated, 'partition_valid': True}


def validate_transcription(native):
    """Require each raw story exactly once, including stories absent from LO exports."""
    raw = native['stories']
    projected = native['transcription']['stories']
    sources = {s['kind']: s for s in raw}
    projections = {s['kind']: s for s in projected}
    if len(sources) != len(raw) or len(projections) != len(projected):
        raise ValueError('duplicate source or projected story')
    if sources.keys() != projections.keys():
        raise ValueError('missing or unexpected projected story')
    return {kind: validate_projection(source, projections[kind])
            for kind, source in sources.items()}


def without_unresolved_segments(story):
    """Omit only helper-marked unresolved controls, retaining literal U+FFFC text."""
    encoded = story['text'].encode()
    return b''.join(encoded[s['output_start']:s['output_end']]
                    for s in story['segments'] if s['kind'] != 'unresolved_control').decode()


def ooxml_stories(path, diagnostics=None):
    stories = defaultdict(list)
    if diagnostics is None:
        diagnostics = {}
    for kind in KINDS:
        diagnostics[kind] = {'unresolved_font_symbols': 0}
    with zipfile.ZipFile(path) as archive:
        for name in sorted(archive.namelist()):
            base = Path(name).name
            if name == 'word/document.xml':
                kind = 'body'
            elif re.fullmatch(r'(header|footer)\d+\.xml', base) and name.startswith('word/'):
                kind = 'headers'
            elif name in ('word/footnotes.xml', 'word/endnotes.xml', 'word/comments.xml'):
                kind = base[:-4]
            else:
                continue
            root = ET.fromstring(archive.read(name))

            def walk(node, target):
                if node.tag == W + 'del':
                    return
                if node.tag in (W + 'footnote', W + 'endnote') and node.get(W + 'type', 'normal') != 'normal':
                    return
                if node.tag == W + 'txbxContent':
                    target = 'header_textboxes' if kind == 'headers' else 'textboxes'
                if node.tag == W + 't':
                    stories[target].append(node.text or '')
                elif node.tag == W + 'softHyphen':
                    stories[target].append('\u00ad')
                elif node.tag == W + 'noBreakHyphen':
                    stories[target].append('\u2011')
                elif node.tag == W + 'sym':
                    diagnostics[target]['unresolved_font_symbols'] += 1
                elif node.tag in (W + 'tab', W + 'br', W + 'cr'):
                    stories[target].append(' ')
                children = list(node)
                if node.tag == MC + 'AlternateContent':
                    choices = [child for child in children if child.tag == MC + 'Choice']
                    children = choices[:1] or [child for child in children if child.tag == MC + 'Fallback'][:1]
                for child in children:
                    walk(child, target)
                if node.tag == W + 'p':
                    stories[target].append('\n')
            walk(root, kind)
    return {kind: ''.join(stories[kind]) for kind in KINDS}


def compare(native, converted):
    left, right = normalized(native), normalized(converted)
    a, b = Counter(left), Counter(right)
    # Long repeated documents make exact sequence alignment quadratic.
    order = SequenceMatcher(None, left, right, autojunk=False).ratio() if max(len(left), len(right)) <= 2000 else None
    return {
        'native_characters': len(left), 'lo_characters': len(right),
        'native_only_character_occurrences': sum((a - b).values()),
        'lo_only_character_occurrences': sum((b - a).values()),
        'same_normalized_order': left == right, 'sequence_ratio': order,
        'native_sha256': digest(left), 'lo_sha256': digest(right),
    }


def select(corpus, previews, conversions):
    accepted = {r['source_sha256']: r for r in corpus['rows'] if r.get('status') == 'raw_stories_only'}
    reasons = defaultdict(list)
    for row in previews['rows']:
        if row.get('preview_status') == 'previously_visually_validated' and row['source_sha256'] in accepted:
            reasons[row['source_sha256']].append('qualified_preview')
    eligible = sorted((r for h, r in accepted.items() if h in conversions), key=lambda r: r['source_sha256'])
    strata = defaultdict(list)
    for row in eligible:
        strata['revision:' + row['version']].append(row)
        for story in row['stories']:
            if story['utf16_units'] > 2:
                strata['story:' + story['kind']].append(row)
    for label, rows in sorted(strata.items()):
        for row in rows[:6]:
            reasons[row['source_sha256']].append(label)
    return accepted, reasons


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--dataset', required=True, type=Path)
    parser.add_argument('--corpus', required=True, type=Path)
    parser.add_argument('--previews', required=True, type=Path)
    parser.add_argument('--conversion-manifest', type=Path, action='append', required=True)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--include-report', type=Path, help='Keep all sources selected in a previous report')
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    frozen_binary = args.output.with_suffix('.binary')
    frozen_binary.write_bytes(args.binary.read_bytes())
    frozen_binary.chmod(0o700)
    binary_sha256 = sha(frozen_binary)
    corpus, previews = (json.loads(p.read_text()) for p in (args.corpus, args.previews))
    conversions = {}
    conversion_provenance = []
    for path in args.conversion_manifest:
        data = json.loads(path.read_text())
        conversion_provenance.append({'path': str(path), 'sha256': sha(path), 'converter_version': data.get('converter_version', 'unrecorded_in_manifest')})
        for row in data.get('records', data.get('results', [])):
            if row.get('conversion_ok') and row.get('converted_sha256'):
                conversions[row.get('source_sha256', row.get('sha256'))] = row
    accepted, selected = select(corpus, previews, conversions)
    if args.include_report:
        for previous in json.loads(args.include_report.read_text())['rows']:
            h = previous['source_sha256']
            if h not in accepted:
                raise ValueError('previous roster source absent from accepted corpus')
            selected[h].append('retained_previous_roster')
    paths = {}
    for path in sorted(args.dataset.rglob('*')):
        if path.is_file() and path.suffix.lower() == '.doc' and not path.name.startswith('._'):
            h = sha(path)
            if h in selected:
                paths[h] = path
    rows = []
    for h, reasons in sorted(selected.items()):
        started = time.monotonic()
        row = {'id': h[:12], 'source_sha256': h, 'selection': reasons,
               'revision': accepted[h]['version'], 'split': 'unassigned_pending_template_family_audit'}
        try:
            source, old = paths[h], conversions[h]
            frozen = Path(old.get('original_frozen_path', old.get('frozen_path', '')))
            converted = Path(old['converted_path'])
            if sha(frozen) != h or sha(converted) != old['converted_sha256']:
                raise ValueError('source or conversion provenance mismatch')
            row.update(source_path=str(source), converted_path=str(converted), converted_sha256=old['converted_sha256'])
            result = subprocess.run([str(frozen_binary.resolve()), str(source)], capture_output=True, timeout=30, check=True)
            native = json.loads(result.stdout)
            lo_diagnostics = {}
            lo = ooxml_stories(converted, lo_diagnostics)
            raw_stories = {s['kind']: s for s in native['stories']}
            raw = {k: s['text'] for k, s in raw_stories.items()}
            transcription = native.get('transcription', {})
            projections = {s['kind']: s for s in transcription.get('stories', [])}
            row['native_transcription'] = {k: v for k, v in transcription.items() if k != 'stories'}
            provenance = validate_transcription(native) if 'transcription' in native else {}
            row['projection_provenance'] = provenance
            row['stories'] = {}
            for kind in KINDS:
                text = raw.get(kind, '')
                projected, malformed = field_results(text)
                row['stories'][kind] = {
                    'raw': compare(text, lo[kind]),
                    'lo_diagnostics': lo_diagnostics[kind],
                    'cached_field_projection': compare(projected, lo[kind]),
                    'field_control_count': sum(text.count(c) for c in '\x13\x14\x15'),
                    'unbalanced_field_controls': malformed,
                    'attribution': 'part-level; headers/footers pooled; table text retained in enclosing story',
                }
                if kind in projections:
                    current = projections[kind]
                    row['stories'][kind]['native_projection'] = {
                        'provenance': provenance[kind],
                        'comparison': compare(current['text'], lo[kind]),
                        'text_only_diagnostic': compare(without_unresolved_segments(current), lo[kind]),
                        'omitted_unresolved_source_units': sum(s['end_cp'] - s['start_cp'] for s in current['segments'] if s['kind'] == 'unresolved_control'),
                        'diagnostics': current['diagnostics'],
                        'fields_balanced': current['fields_balanced'],
                        'unresolved_control_units': current['unresolved_control_units'],
                    }
            row['status'] = 'compared'
        except Exception as error:
            row.update(status='error', error=str(error))
        row['seconds'] = time.monotonic() - started
        rows.append(row)
        print(json.dumps({'id': row['id'], 'status': row['status']}), flush=True)
    summary = {
        'schema_version': 1, 'binary_sha256': binary_sha256, 'rows': rows,
        'conversion_provenance': conversion_provenance,
        'statuses': dict(Counter(r['status'] for r in rows)),
        'revisions': dict(Counter(r['revision'] for r in rows)),
        'accepted_corpus_revisions': dict(Counter(r['version'] for r in accepted.values())),
        'eligible_conversion_revisions': dict(Counter(r['version'] for h, r in accepted.items() if h in conversions)),
        'limitations': [
            'LibreOffice exports are comparison evidence, not independent Word ground truth.',
            'Raw native fields, revisions, hidden text and symbols remain unresolved.',
            'Cached field projection is diagnostic and does not resolve formatting or revision visibility.',
            'Text-only diagnostics omit source-marked unresolved controls; they cannot prove object-content completeness.',
            'Headers are pooled; OOXML conversion can deduplicate or relocate stories.',
            'Selection is stratified; no family-separated holdout claimed until manual family audit.',
            'Character inventory differences detect disagreements, not proven omissions.',
            'LO font-coded w:sym records are counted as unresolved, omitted from strings; affected comparisons are incomplete.',
            'Sequence ratio omitted for stories over 2000 normalized characters.',
        ],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary['statuses']))


if __name__ == '__main__':
    main()
