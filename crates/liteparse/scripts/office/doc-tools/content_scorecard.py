#!/usr/bin/env python3
"""Measure source-owned DOC extraction independently of page-render admission.

Reports contain hashes and counts, not document text. CP accounting, logical
projection and semantic accuracy are separate dimensions. Cached runs bind an
explicit historical helper receipt; fresh runs verify every source hash.
"""
import argparse
import bisect
import collections
import concurrent.futures
import gzip
import hashlib
import html
import json
from pathlib import Path
import subprocess
import time

from transcription_baseline import validate_projection


def sha(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1048576), b""):
            h.update(chunk)
    return h.hexdigest()


def union(ranges):
    result = []
    for start, end in sorted(ranges):
        if end <= start:
            continue
        if result and start <= result[-1][1]:
            result[-1] = (result[-1][0], max(end, result[-1][1]))
        else:
            result.append((start, end))
    return result


def subtract(xs, ys):
    out = []
    ys = union(ys)
    j = 0
    for a, b in union(xs):
        while j < len(ys) and ys[j][1] <= a:
            j += 1
        cursor, k = a, j
        while k < len(ys) and ys[k][0] < b:
            c, d = ys[k]
            if c > cursor:
                out.append((cursor, min(c, b)))
            cursor = max(cursor, d)
            if cursor >= b:
                break
            k += 1
        if cursor < b:
            out.append((cursor, b))
    return out


def intersection(left, right):
    return subtract(left, subtract(left, right))


def length(ranges):
    return sum(end - start for start, end in union(ranges))


def overlap(left, right):
    left = union(left)
    right = union(right)
    i = j = total = 0
    while i < len(left) and j < len(right):
        total += max(0, min(left[i][1], right[j][1]) - max(left[i][0], right[j][0]))
        if left[i][1] < right[j][1]:
            i += 1
        else:
            j += 1
    return total


def diagnostic_family(code):
    if any(word in code for word in ('list', 'numbering', 'bullet')):
        return 'generated_labels'
    if any(word in code for word in ('field', 'form', 'hyperlink')):
        return 'fields_and_generated_values'
    if any(word in code for word in ('hidden', 'visibility', 'revision')):
        return 'visibility_and_revisions'
    if any(word in code for word in ('note', 'header', 'textbox', 'comment')):
        return 'story_assembly'
    return 'other_source_semantics'


class RangeCounter:
    """Answer CP interval counts with a sorted union and prefix sums."""
    def __init__(self, ranges):
        self.ranges = union(ranges)
        self.starts = [a for a, _ in self.ranges]
        self.prefix = [0]
        for a, b in self.ranges:
            self.prefix.append(self.prefix[-1] + b - a)

    def before(self, position):
        i = bisect.bisect_left(self.starts, position) - 1
        if i < 0:
            return 0
        a, b = self.ranges[i]
        return self.prefix[i] + min(position, b) - a

    def count(self, start, end):
        return self.before(end) - self.before(start)


def component_errors(payload):
    """Retain explicit CLI component failures without interpreting them as lost text.

    Only top-level error fields and metadata envelopes are inspected. Authored
    content and nested source records may contain ordinary fields named error.
    """
    errors = {}
    for name, value in payload.items():
        if name.endswith('_error') and value:
            errors[name] = {'error': value}
        elif isinstance(value, dict) and (value.get('error') or value.get('status') == 'unavailable'):
            errors[name] = {'status': value.get('status'), 'error': value.get('error')}
    return errors


def component_uncertainties(payload):
    """Report stored supplementary text without assigning it story CPs or visible loss."""
    geometry = payload.get('geometry_text', {})
    if not isinstance(geometry, dict) or geometry.get('status') == 'unavailable':
        return {}
    candidates = [p for shape in geometry.get('shapes', [])
                  for p in shape.get('candidates', []) if p.get('text')]
    if not candidates:
        return {}
    return {'geometry_text': {
        'reason': 'saved_text_visibility_and_reading_order_unresolved',
        'stored_text_candidates': len(candidates),
        'stored_candidate_utf16_units': sum(len(p['text'].encode('utf-16-le')) // 2 for p in candidates),
        'source_ownership': 'table_stream_bytes_outside_story_CP_accounting',
    }}


def analyze(payload):
    raw_stories = payload['stories']
    projection = payload.get('transcription', {})
    projected = {s['kind']: s for s in projection.get('stories', [])}
    if (len(projected) != len(raw_stories)
            or len(projected) != len(projection.get('stories', []))
            or len({s['kind'] for s in raw_stories}) != len(raw_stories)):
        raise ValueError('raw/projected story cardinality or duplicate story')
    visibility = payload.get('hidden_properties', {}).get('spans', [])
    visibility_ranges = {
        name: union((s['start_cp'], s['end_cp']) for s in visibility if s.get('hidden') is value)
        for name, value in [('known_hidden', True), ('known_nonhidden', False), ('unknown', None)]
    }
    fields = {s['kind']: s for s in payload.get('fields', {}).get('stories', [])}
    stories, families = [], collections.Counter()
    for raw in raw_stories:
        kind = raw['kind']
        text = projected[kind]
        validate_projection(raw, text)
        start, end = raw['start_cp'], raw['start_cp'] + raw['utf16_units']
        encoded = text['text'].encode()
        emitted, instructions, results, lexical = [], [], [], []
        indexed_instructions = []
        empty_by_kind, source_by_kind, generated = (collections.Counter() for _ in range(3))
        unknown_semantics = []
        for segment in text['segments']:
            a, b = segment['start_cp'], segment['end_cp']
            lo, hi = segment['output_start'], segment['output_end']
            name = segment['kind']
            if a == b:
                generated[name] += 1
                continue
            source_by_kind[name] += b - a
            if hi > lo:
                emitted.append((a, b))
            else:
                empty_by_kind[name] += b - a
            if name in ('unresolved_control', 'unprojected_tail'):
                unknown_semantics.append((a, b))
            if name == 'field_instruction':
                instructions.append((a, b))
        for field in fields.get(kind, {}).get('table', {}).get('ranges', []):
            for key, ranges in [('instruction', instructions), ('result', results)]:
                interval = field.get(key)
                if interval:
                    ranges.append((interval['start'], interval['end']))
                    if key == 'instruction':
                        indexed_instructions.append((interval['start'], interval['end']))
        cp = start
        for char in raw['text']:
            size = len(char.encode('utf-16-le')) // 2
            if not char.isspace() and ord(char) >= 32 and ord(char) != 127:
                lexical.append((cp, cp + size))
            cp += size
        candidate = intersection(subtract(lexical, indexed_instructions), visibility_ranges['known_nonhidden'])
        candidate_counter = RangeCounter(candidate)
        empty_candidate = collections.Counter()
        for segment in text['segments']:
            if segment['output_start'] == segment['output_end']:
                empty_candidate[segment['kind']] += candidate_counter.count(segment['start_cp'], segment['end_cp'])
        empty_candidate = +empty_candidate
        classified = {'field_instruction', 'deleted_revision', 'hidden_text', 'field_marker', 'paragraph_mark'}
        unexplained = sum(n for name, n in empty_candidate.items() if name not in classified)
        diagnostics = collections.Counter(d['code'] for d in text.get('diagnostics', []))
        families.update({name: sum(n for code, n in diagnostics.items() if diagnostic_family(code) == name)
                         for name in {diagnostic_family(code) for code in diagnostics}})
        stories.append({
            'kind': kind, 'raw_utf16': end - start,
            'raw_text_sha256': hashlib.sha256(raw['text'].encode()).hexdigest(),
            'source_partition_complete': True, 'source_cp_accounted': end - start,
            'logical_utf8_bytes': len(encoded), 'logical_utf16': len(text['text'].encode('utf-16-le')) // 2,
            'source_cp_with_output': length(emitted), 'empty_source_cp_by_kind': dict(empty_by_kind),
            'source_cp_by_kind': dict(source_by_kind), 'generated_zero_cp_anchors': dict(generated),
            'unresolved_segment_cp': length(unknown_semantics),
            'indexed_and_projected_instruction_cp': length(instructions),
            'cached_result_cp': length(results), 'cached_result_cp_with_output': overlap(results, emitted),
            'known_nonhidden_lexical_outside_indexed_instructions_cp': length(candidate),
            'empty_candidate_cp_by_kind': dict(empty_candidate),
            'unclassified_empty_candidate_cp': unexplained,
            'visibility_cp': {name: overlap([(start, end)], spans) for name, spans in visibility_ranges.items()},
            'visibility_uncovered_cp': end - start - overlap([(start, end)], sum(visibility_ranges.values(), [])),
            'diagnostics': dict(diagnostics),
        })
    errors = component_errors(payload)
    uncertainties = component_uncertainties(payload)
    unresolved = bool(errors or uncertainties) or any(s['diagnostics'] or s['unresolved_segment_cp']
        or s['visibility_cp']['unknown'] or s['visibility_uncovered_cp'] for s in stories if s['raw_utf16'])
    return {'status': 'partial_logical_extraction' if unresolved else 'source_accounted_no_reported_gaps',
            'source_accounting': 'complete', 'semantic_accuracy': 'not_established',
            'parser_status': payload.get('status'), 'projection_status': projection.get('status'),
            'stories': stories, 'diagnostic_families': dict(families),
            'component_errors': errors,
            'component_uncertainties': uncertainties,
            'trailing_source_units': len(payload.get('trailing_utf16_units', []))}


def summarize(rows, provenance):
    totals = collections.Counter()
    statuses = collections.Counter()
    component_docs = collections.Counter()
    diagnostics = collections.Counter()
    diagnostic_docs = collections.Counter()
    error_docs = collections.Counter()
    documents_with_errors = 0
    uncertainty_docs = collections.Counter()
    documents_with_uncertainty = 0
    suppressed = collections.Counter()
    generated = collections.Counter()
    for row in rows:
        statuses[row['status']] += 1
        error_docs.update(row.get('component_errors', {}).keys())
        documents_with_errors += bool(row.get('component_errors'))
        uncertainty_docs.update(row.get('component_uncertainties', {}).keys())
        documents_with_uncertainty += bool(row.get('component_uncertainties'))
        diagnostic_docs.update({code for s in row.get('stories', []) for code in s['diagnostics']})
        component_docs.update(row.get('components', []))
        for story in row.get('stories', []):
            for key in ['raw_utf16', 'source_cp_accounted', 'logical_utf8_bytes', 'logical_utf16',
                        'source_cp_with_output', 'unresolved_segment_cp', 'cached_result_cp',
                        'cached_result_cp_with_output', 'unclassified_empty_candidate_cp', 'visibility_uncovered_cp']:
                totals[key] += story[key]
            totals['stories_partition_complete'] += story['source_partition_complete']
            for key, value in story['visibility_cp'].items():
                totals['visibility_' + key + '_cp'] += value
            diagnostics.update(story['diagnostics'])
            suppressed.update(story['empty_source_cp_by_kind'])
            generated.update(story['generated_zero_cp_anchors'])
    return {'schema_version': 3, 'documents': len(rows), 'provenance': provenance,
            'statuses': dict(statuses), 'totals': dict(totals),
            'documents_with_component_errors': documents_with_errors,
            'documents_with_component_uncertainty': documents_with_uncertainty,
            'component_uncertainty_document_counts_overlapping': dict(uncertainty_docs),
            'component_error_document_counts_overlapping': dict(error_docs),
            'component_document_counts_overlapping': dict(component_docs),
            'diagnostic_events': dict(diagnostics), 'diagnostic_document_counts': dict(diagnostic_docs),
            'empty_source_cp_by_kind': dict(suppressed), 'generated_zero_cp_anchors': dict(generated),
            'limitations': [
                'Raw story retention and CP accounting do not establish semantic accuracy or reading order.',
                'Suppressed instructions, deleted revisions and hidden text are classified separately from possible omissions.',
                'Unclassified nonhidden empty CPs are review candidates, not proven visible text loss.',
                'Generated anchors are counted separately; unsupported labels have no raw source-CP denominator.',
                'Document status describes reported extraction uncertainty, not page-render completion.',
                'Explicit component errors or unavailable metadata make a document partial even when raw accounting is complete; they do not establish missing visible text.',
                'Stored supplementary geometry text has unresolved visibility and order; its byte ownership is outside story CP accounting and is not definite missing text.',
                'Renderer first failures and reference availability are independent annotations, not extraction scores.',
                'Diagnostic counts are observed records; parser diagnostic limits may cap them.',
            ]}


def choose_batch(rows, size):
    """Greedily cover feature pairs to favor mixed-component cases over single guards."""
    pending = []
    for row in rows:
        features = set(row.get('components', []))
        features.update('diagnostic:' + k for k in row.get('diagnostic_families', {}))
        features.update('component_error:' + k for k in row.get('component_errors', {}))
        if row.get('renderer_reason'):
            features.add('renderer:' + row['renderer_reason'])
        ordered = sorted(features)
        pairs = {(a, b) for i, a in enumerate(ordered) for b in ordered[i + 1:]}
        pending.append((row, features, pairs))
    chosen, seen, seen_pairs = [], set(), set()
    while pending and len(chosen) < size:
        best = max(range(len(pending)), key=lambda i: (
            len(pending[i][2] - seen_pairs), len(pending[i][1] - seen),
            len(pending[i][1]), pending[i][0]['source_sha256']))
        row, features, pairs = pending.pop(best)
        chosen.append({'source_sha256': row['source_sha256'], 'features': sorted(features),
                       'new_feature_pairs': len(pairs - seen_pairs), 'status': row['status']})
        seen.update(features)
        seen_pairs.update(pairs)
    return {'selection': 'deterministic greedy coverage of overlapping component/diagnostic/first-stop pairs',
            'not_random_accuracy_sample': True, 'feature_pairs_covered': len(seen_pairs), 'rows': chosen}


def report_html(summary, rows):
    esc = html.escape
    cards = ''.join(f'<li><b>{v:,}</b> {esc(k)}</li>' for k, v in summary['totals'].items())
    records = ''.join('<tr><td>' + esc(r['source_sha256'][:12]) + '</td><td>' + esc(r['status'])
                      + '</td><td>' + esc(', '.join(r.get('components', []))) + '</td><td>'
                      + str(sum(s['raw_utf16'] for s in r.get('stories', []))) + '</td></tr>' for r in rows)
    sections = '<h2>Document uncertainty</h2><ul>' + ''.join(
        f'<li>{esc(k)}: {v:,}</li>' for k, v in summary['statuses'].items()) + '</ul>'
    for title, key in [('Generated anchors (no authored CPs)', 'generated_zero_cp_anchors'),
                       ('Suppressed source CPs by reason', 'empty_source_cp_by_kind'),
                       ('Documents with diagnostic (overlapping)', 'diagnostic_document_counts'),
                       ('Documents with component errors (overlapping)', 'component_error_document_counts_overlapping'),
                       ('Documents with supplementary text uncertainty (overlapping)', 'component_uncertainty_document_counts_overlapping')]:
        sections += '<h2>' + title + '</h2><ul>' + ''.join(
            f'<li>{esc(k)}: {v:,}</li>' for k, v in sorted(summary[key].items(), key=lambda item: -item[1])) + '</ul>'
    sections += '<h2>Independent evidence availability</h2><p>' + str(
        summary.get('documents_with_verified_external_evidence', 0)) + (
        ' sources have hash-verified external evidence receipts. Raw CFB/CLX checks and existing renderer-versus-LO '
        'comparisons retain their original scopes; they are not a corpus-wide logical accuracy score.</p>')
    return ('<!doctype html><meta charset="utf-8"><title>DOC content scorecard</title>'
            '<style>body{font:16px system-ui;max-width:1200px;margin:40px auto;padding:20px}'
            'table{border-collapse:collapse;width:100%}td,th{padding:8px;border-bottom:1px solid #ddd;text-align:left}'
            'li{margin:8px 0}input{padding:10px;width:90%}</style><h1>DOC content scorecard</h1>'
            '<p>Source-owned extraction is evaluated independently of rendering.</p><ul>' + cards
            + '</ul>' + sections + '<h2>Interpretation</h2><ul>' + ''.join('<li>' + esc(x) + '</li>' for x in summary['limitations'])
            + '</ul><h2>Reproducibility</h2><pre>' + esc(json.dumps(summary['provenance'], indent=2))
            + '</pre><h2>Documents</h2><input placeholder="Filter hash, status or component" '
            'oninput="for(const r of document.querySelectorAll(\'tbody tr\'))r.hidden=!r.innerText.includes(this.value)">'
            '<table><thead><tr><th>Source hash</th><th>Extraction status</th><th>Components</th><th>Raw UTF-16</th>'
            '</tr></thead><tbody>' + records + '</tbody></table>')


def compare_rows(before, after):
    """Compare the same source cohort without treating metric improvements as accuracy proof."""
    a = {row['source_sha256']: row for row in before}
    b = {row['source_sha256']: row for row in after}
    if len(a) != len(before) or len(b) != len(after) or a.keys() != b.keys():
        raise ValueError('comparison cohort mismatch or duplicate source')
    schemas = {('component_errors' in row, 'component_uncertainties' in row) for row in before + after}
    if len(schemas) > 1:
        raise ValueError('scorecard schema mismatch; recompute both caches with the same evaluator before comparing parser gains')
    changed = []
    for source, old in a.items():
        new = b[source]
        old_raw = {s['kind']: (s['raw_utf16'], s['raw_text_sha256']) for s in old.get('stories', [])}
        new_raw = {s['kind']: (s['raw_utf16'], s['raw_text_sha256']) for s in new.get('stories', [])}
        if old_raw != new_raw:
            raise ValueError('comparison raw story identity differs')
        def anchors(row):
            values = collections.Counter()
            for story in row.get('stories', []):
                values.update(story['generated_zero_cp_anchors'])
            return values
        old_values, new_values = anchors(old), anchors(new)
        delta = {key: new_values[key] - old_values[key] for key in old_values.keys() | new_values.keys()
                 if new_values[key] != old_values[key]}
        if delta or old['status'] != new['status'] or old.get('diagnostic_families') != new.get('diagnostic_families') or old.get('component_errors') != new.get('component_errors') or old.get('component_uncertainties') != new.get('component_uncertainties'):
            changed.append({'source_sha256': source, 'before_status': old['status'], 'after_status': new['status'],
                            'generated_anchor_delta': delta,
                            'before_component_errors': old.get('component_errors', {}),
                            'after_component_errors': new.get('component_errors', {}),
                            'before_component_uncertainties': old.get('component_uncertainties', {}),
                            'after_component_uncertainties': new.get('component_uncertainties', {})})
    return {'documents': len(a), 'raw_story_identity_exact': True, 'changed_documents': changed,
            'scope': 'Metric deltas only; exact generated text and source-family correctness require separate inverse/source proof.'}


def check_references(entries, approved):
    """Validate external receipts without interpreting renderer comparisons as logical scores."""
    results = []
    for entry in entries:
        if entry['source_sha256'] not in approved:
            raise ValueError('reference source is outside the approved roster')
        path = Path(entry['path'])
        status = 'missing' if not path.exists() else ('hash_verified' if sha(path) == entry['sha256'] else 'hash_changed')
        results.append({'source_sha256': entry['source_sha256'], 'kind': entry['kind'],
                        'scope': entry['scope'], 'status': status, 'artifact_sha256': entry['sha256']})
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--roster', type=Path, required=True, help='Explicit approved JSONL source_sha256/source_path roster')
    parser.add_argument('--binary', type=Path, help='Optimized doc-read executable for a fresh run')
    parser.add_argument('--cache', type=Path, required=True)
    parser.add_argument('--cache-meta', type=Path)
    parser.add_argument('--cached-only', action='store_true')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--renderer-census', type=Path)
    parser.add_argument('--baseline-rows', type=Path, help='Compare identical source cohort against a prior scorecard rows.json')
    parser.add_argument('--reference-manifest', type=Path, help='Optional source-bound external evidence receipts')
    parser.add_argument('--expected-count', type=int, required=True)
    parser.add_argument('--batch-size', type=int, default=24)
    parser.add_argument('--jobs', type=int, default=4)
    args = parser.parse_args()
    roster = [json.loads(line) for line in args.roster.read_text().splitlines() if line]
    if len(roster) != args.expected_count or len({r['source_sha256'] for r in roster}) != len(roster):
        parser.error('roster cardinality or source hash uniqueness mismatch')
    if any(len(r['source_sha256']) != 64 or any(c not in '0123456789abcdef' for c in r['source_sha256']) for r in roster):
        parser.error('source_sha256 must be 64 lowercase hexadecimal characters')
    if any('holdout' in str(r.get('cohort', '')).lower() for r in roster):
        parser.error('holdout rows are not allowed in this development scorecard')
    meta_path = args.cache_meta or args.cache.parent / 'cache-meta.json'
    if args.cached_only:
        if not meta_path.exists():
            parser.error('cached-only requires an existing helper/cache provenance receipt')
        identity = json.loads(meta_path.read_text())
    else:
        if not args.binary:
            parser.error('fresh mode requires --binary')
        identity = {'helper_sha256': sha(args.binary), 'roster_sha256': sha(args.roster)}
        if meta_path.exists() and json.loads(meta_path.read_text()) != identity:
            parser.error('cache identity differs; use a separate cache')
        meta_path.parent.mkdir(parents=True, exist_ok=True)
        meta_path.write_text(json.dumps(identity, indent=2))
    if identity.get('roster_sha256') != sha(args.roster):
        parser.error('cache roster identity differs')
    manifest_path = args.cache.parent / 'cache-manifest.json'
    cache_hashes = json.loads(manifest_path.read_text()) if manifest_path.exists() else {}
    if args.cached_only and not cache_hashes:
        parser.error('cached-only requires cache-manifest.json with per-payload hashes')
    args.cache.mkdir(parents=True, exist_ok=True)
    args.output.mkdir(parents=True, exist_ok=True)
    renderer = {}
    if args.renderer_census:
        renderer = {r['source_sha256']: r for r in map(json.loads, args.renderer_census.read_text().splitlines())}
    def evaluate(item):
        h = item['source_sha256']
        row = {'source_sha256': h}
        cache = args.cache / (h + '.json.gz')
        try:
            if not args.cached_only:
                if sha(item['source_path']) != h:
                    raise ValueError('source hash differs from approved roster')
                if not cache.exists():
                    run = subprocess.run([str(args.binary.resolve()), item['source_path']], capture_output=True, timeout=90)
                    if run.returncode:
                        row.update(status='parser_error', returncode=run.returncode,
                                   error_sha256=hashlib.sha256(run.stderr).hexdigest())
                        return row
                    payload = json.loads(run.stdout)
                    temporary = cache.with_suffix('.tmp')
                    with gzip.open(temporary, 'wt', compresslevel=1) as stream:
                        json.dump(payload, stream, ensure_ascii=False)
                    temporary.replace(cache)
            if args.cached_only and h not in cache_hashes:
                raise ValueError('cached payload receipt missing')
            if h in cache_hashes and sha(cache) != cache_hashes[h]:
                raise ValueError('cached payload hash differs from receipt')
            with gzip.open(cache, 'rt') as stream:
                payload = json.load(stream)
            row['cache_sha256'] = sha(cache)
            row.update(analyze(payload))
            row['components'] = sorted(set(
                ['story:' + s['kind'] for s in row['stories'] if s['raw_utf16']]
                + ['supplementary:' + k for k in row['component_uncertainties']]
                + ['fields'] * any(s.get('table', {}).get('ranges') for s in payload.get('fields', {}).get('stories', []))
                + ['generated_labels'] * any(s['generated_zero_cp_anchors'].get('generated_list_label') for s in row['stories'])
                + ['generated_values'] * any(any(k != 'generated_list_label' for k in s['generated_zero_cp_anchors']) for s in row['stories'])
                + ['table_cell_markers'] * any('\x07' in s['text'] for s in payload['stories'])
                + ['drawing_anchors'] * any('\x08' in s['text'] for s in payload['stories'])
                + ['object_markers'] * any('\x01' in s['text'] for s in payload['stories'])))
            if h in renderer:
                row['renderer_reason'] = renderer[h]['reason']
        except FileNotFoundError as error:
            row.update(status='missing_input_or_cache', error_sha256=hashlib.sha256(str(error).encode()).hexdigest())
        except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as error:
            row.update(status='accounting_or_harness_error', error_kind=type(error).__name__,
                       error_sha256=hashlib.sha256(str(error).encode()).hexdigest())
        return row
    started = time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        rows = list(pool.map(evaluate, roster))
    if not args.cached_only and sha(args.binary) != identity['helper_sha256']:
        raise ValueError('helper changed during corpus run')
    provenance = {**identity, 'evaluator_schema_version': 3, 'mode': 'historical_cached' if args.cached_only else 'source_hash_verified',
                  'seconds': time.monotonic() - started, 'script_sha256': sha(__file__),
                  'renderer_census_sha256': sha(args.renderer_census) if args.renderer_census else None}
    evaluated_manifest = args.output / 'evaluated-cache-manifest.json'
    evaluated_manifest.write_text(json.dumps({r['source_sha256']: r['cache_sha256'] for r in rows if 'cache_sha256' in r}, indent=2))
    if not manifest_path.exists() and not args.cached_only:
        manifest_path.write_bytes(evaluated_manifest.read_bytes())
    provenance['cache_manifest_sha256'] = sha(manifest_path)
    provenance['evaluated_cache_manifest_sha256'] = sha(evaluated_manifest)
    references = check_references(json.loads(args.reference_manifest.read_text()), {r['source_sha256'] for r in roster}) if args.reference_manifest else []
    provenance['reference_manifest_sha256'] = sha(args.reference_manifest) if args.reference_manifest else None
    summary = summarize(rows, provenance)
    summary['external_evidence'] = references
    summary['documents_with_verified_external_evidence'] = len({r['source_sha256'] for r in references if r['status'] == 'hash_verified'})
    if args.baseline_rows:
        (args.output / 'comparison.json').write_text(json.dumps(compare_rows(json.loads(args.baseline_rows.read_text()), rows), indent=2))
    (args.output / 'rows.json').write_text(json.dumps(rows))
    (args.output / 'summary.json').write_text(json.dumps(summary, indent=2))
    (args.output / 'representative-batch.json').write_text(json.dumps(choose_batch(rows, args.batch_size), indent=2))
    (args.output / 'report.html').write_text(report_html(summary, rows))
    print(json.dumps({'documents': len(rows), 'statuses': summary['statuses'], 'totals': summary['totals']}))


if __name__ == '__main__':
    main()
