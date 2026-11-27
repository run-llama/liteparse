#!/usr/bin/env python3
"""Build conservative DOC family candidates from hashed text, structure and previews.

This audit reserves connected components touching new sources. Similarity edges
are leakage-screening evidence; unresolved formatting and unreviewed edges keep
holdout certification provisional. Reports contain hashes/counts, not source text.
"""
import argparse
from collections import Counter, defaultdict
import concurrent.futures
import hashlib
import itertools
import json
import math
from pathlib import Path
import re
import subprocess

SAMPLE = 128
MAX_TOKENS = 50000


def digest(value):
    return hashlib.sha256(value).hexdigest()


def fingerprint(value):
    return int.from_bytes(hashlib.blake2b(value.encode(), digest_size=8).digest(), 'big')


def sketch(tokens, width):
    return sorted({fingerprint('|'.join(tokens[i:i + width]))
                   for i in range(max(0, len(tokens) - width + 1))})[:SAMPLE]


def overlap(left, right):
    """Estimate Jaccard from the shared bottom-k union of two hash sketches."""
    a, b = set(left), set(right)
    union = sorted(a | b)[:SAMPLE]
    return sum(h in a and h in b for h in union) / len(union) if union else 0.0


def bucket(length):
    return next((i for i, limit in enumerate((10, 30, 80, 160, 320, 640, 1280))
                 if length < limit), 7)


def body_features(body):
    tokens = re.findall(r'\w+', body.casefold())
    token_count = len(tokens)
    truncated = token_count > MAX_TOKENS
    if truncated:
        tokens = tokens[:MAX_TOKENS // 2] + tokens[-MAX_TOKENS // 2:]
    tokens = ['#' if word.isdigit() else word for word in tokens]
    shapes = []
    for paragraph in body.split('\r'):
        shapes.append(':'.join(str(v) for v in (
            bucket(len(paragraph)), min(paragraph.count('\x07'), 8),
            min(paragraph.count('\t'), 8), min(paragraph.count('\x13'), 4),
            min(paragraph.count('\x0c'), 1))))
    return {'text': sketch(tokens, 5), 'controls': sketch(shapes, 5),
            'body_sha256': digest(body.encode()), 'word_count': token_count,
            'paragraph_count': len(shapes), 'text_sample_truncated': truncated}


def paragraph_features(metadata):
    if not metadata or metadata.get('status') == 'unavailable':
        return {'papx': [], 'papx_available': False, 'papx_spans': 0, 'papx_distinct': 0}
    styles, tokens = {}, []
    for span in metadata.get('spans', []):
        index = span.get('style_index')
        if index not in styles:
            styles[index] = len(styles)
        token = f"{styles[index]}:{digest(bytes(span['raw_modifiers']))}"
        if not tokens or tokens[-1] != token:
            tokens.append(token)
    return {'papx': sketch(tokens, 3), 'papx_available': True,
            'papx_spans': len(metadata.get('spans', [])),
            'papx_distinct': len(set(tokens)), 'papx_sequence_sha256': digest('|'.join(tokens).encode())}


def ink_profile(path, crop):
    from PIL import Image
    image = Image.open(path).convert('L')
    if crop:
        image = image.crop(crop)
    return [255 - pixel for pixel in image.resize((16, 24)).getdata()]


def cosine(a, b):
    if not a or not b:
        return None
    norm = math.sqrt(sum(x * x for x in a) * sum(x * x for x in b))
    return sum(x * y for x, y in zip(a, b)) / norm if norm else None


def edge(left, right):
    text = overlap(left['text'], right['text'])
    control = overlap(left['controls'], right['controls'])
    papx = overlap(left['papx'], right['papx'])
    ink = cosine(left.get('ink'), right.get('ink'))
    reasons = []
    if left['body_sha256'] == right['body_sha256'] and min(left['word_count'], right['word_count']) >= 20:
        reasons.append('exact_body')
    if text >= .65:
        reasons.append('high_text_overlap')
    if text >= .35 and max(control, papx) >= .5:
        reasons.append('text_and_structure')
    distinctive = min(left['papx_distinct'], right['papx_distinct']) >= 3 and min(left['papx_spans'], right['papx_spans']) >= 12
    if distinctive and papx >= .85 and control >= .75:
        reasons.append('paragraph_style_and_controls')
    if ink is not None and ink >= .985 and distinctive and max(papx, control) >= .85:
        reasons.append('preview_and_structure')
    if not reasons:
        return None
    return {'left': left['source_sha256'], 'right': right['source_sha256'],
            'reasons': reasons, 'text_jaccard_estimate': text,
            'control_jaccard_estimate': control, 'papx_jaccard_estimate': papx,
            'preview_ink_cosine': ink, 'review': 'unreviewed_candidate_link'}


def components(hashes, edges, new_hashes):
    parent = {h: h for h in hashes}

    def root(h):
        while parent[h] != h:
            parent[h] = parent[parent[h]]
            h = parent[h]
        return h

    for item in edges:
        a, b = root(item['left']), root(item['right'])
        if a != b:
            parent[max(a, b)] = min(a, b)
    groups = defaultdict(list)
    for h in hashes:
        groups[root(h)].append(h)
    return [{'family_id': digest('|'.join(sorted(members)).encode())[:16],
             'members': sorted(members), 'new_members': sum(h in new_hashes for h in members),
             'reservation': 'holdout_candidate_entire_component' if any(h in new_hashes for h in members) else 'existing_only',
             'certification': 'provisional_pending_link_and_unresolved_feature_review'}
            for members in sorted(groups.values(), key=lambda group: min(group))]


def inspect(row, binary, previews):
    result = {'source_sha256': row['sha256'], 'id': row['sha256'][:12]}
    try:
        path = Path(row['path'])
        if digest(path.read_bytes()) != row['sha256']:
            raise ValueError('source hash changed')
        proc = subprocess.run([str(binary), str(path)], capture_output=True, timeout=45, check=True)
        data = json.loads(proc.stdout)
        body = next(story['text'] for story in data['stories'] if story['kind'] == 'body')
        result.update(body_features(body))
        result.update(paragraph_features(data.get('paragraphs')))
        result['binary_version'] = data['binary_version']
        result['status'] = 'fingerprinted'
        preview = previews.get(row['sha256'])
        if preview:
            reference = Path(preview['reference_path'])
            if digest(reference.read_bytes()) != preview['reference_sha256']:
                raise ValueError('preview hash changed')
            result['ink'] = ink_profile(reference, preview.get('crop'))
    except Exception as error:
        result.update(status='unresolved', error_type=type(error).__name__)
    return result


def candidate_pairs(rows):
    postings = defaultdict(list)
    for row in rows:
        row['rare_candidate_anchors'] = 0
    for i, row in enumerate(rows):
        if row['word_count'] >= 20:
            postings[('exact_body', row['body_sha256'])].append(i)
        for kind in ('text', 'controls', 'papx'):
            for token in row[kind][:16]:
                postings[(kind, token)].append(i)
    pairs = set()
    skipped = 0
    for (kind, _), indices in postings.items():
        if kind == 'exact_body':
            pairs.update(zip(indices, indices[1:]))
            continue
        if len(indices) > 128:
            skipped += 1
            continue
        for index in indices:
            rows[index]['rare_candidate_anchors'] += 1
        pairs.update(itertools.combinations(indices, 2))
    for row in rows:
        row['candidate_search_limited'] = not row['rare_candidate_anchors'] and not row.get('ink')
    visual = [i for i, row in enumerate(rows) if row.get('ink')]
    pairs.update(itertools.combinations(visual, 2))
    return pairs, skipped


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--inventory', type=Path, required=True)
    parser.add_argument('--new-sources', type=Path, required=True)
    parser.add_argument('--preview-pool', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--reuse-features', type=Path, help='Reuse hashes from the same inventory and helper snapshot')
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    binary = args.output.with_suffix('.binary')
    binary.write_bytes(args.binary.read_bytes())
    binary.chmod(0o700)
    inventory = json.loads(args.inventory.read_text())['rows']
    new_hashes = {r['sha256'] for r in json.loads(args.new_sources.read_text())['new_sources']}
    pool = json.loads(args.preview_pool.read_text())['rows']
    previews = {r['source_sha256']: r for r in pool if r.get('reference_path') and r.get('preview_status') != 'coarse_layout_only_low_resolution'}
    if args.reuse_features:
        previous = json.loads(args.reuse_features.read_text())
        if previous['binary_sha256'] != digest(binary.read_bytes()) or previous['inventory_sha256'] != digest(args.inventory.read_bytes()):
            raise ValueError('feature cache provenance mismatch')
        features = previous['features']
        for row in features:
            preview = previews.get(row['source_sha256'])
            if preview and row['status'] == 'fingerprinted':
                path = Path(preview['reference_path'])
                if digest(path.read_bytes()) != preview['reference_sha256']:
                    raise ValueError('preview hash changed')
                row['ink'] = ink_profile(path, preview.get('crop'))
    else:
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as workers:
            features = list(workers.map(lambda r: inspect(r, binary.resolve(), previews), inventory))
    good = [row for row in features if row['status'] == 'fingerprinted']
    pairs, skipped = candidate_pairs(good)
    edges = [value for i, j in sorted(pairs) if (value := edge(good[i], good[j]))]
    groups = components([r['source_sha256'] for r in features], edges, new_hashes)
    by_hash = {row['source_sha256']: row for row in features}
    for group in groups:
        members = [by_hash[h] for h in group['members']]
        old_members = len(members) - group['new_members']
        group['previously_seen_members'] = old_members
        group['strict_unseen_eligible_candidate'] = bool(group['new_members'] and not old_members and all(
            member.get('papx_available') and not member.get('text_sample_truncated')
            and not member.get('candidate_search_limited')
            and member['status'] == 'fingerprinted' for member in members))
        group['exclusion_reason'] = ('previously_seen_family_member' if old_members else
                                     'unresolved_or_sampled_features' if not group['strict_unseen_eligible_candidate'] else None)
    for row in features:
        row.pop('ink', None)
    output = {'schema_version': 1, 'binary_sha256': digest(binary.read_bytes()),
              'inventory_sha256': digest(args.inventory.read_bytes()), 'features': features,
              'edges': edges, 'families': groups, 'candidate_pair_count': len(pairs),
              'common_posting_lists_skipped': skipped,
              'thresholds': {'high_text_jaccard': .65, 'mixed_text_jaccard': .35,
                             'mixed_structure_jaccard': .5, 'papx_jaccard': .85,
                             'controls_jaccard': .75, 'preview_cosine': .985,
                             'distinct_papx_minimum': 3, 'papx_spans_minimum': 12,
                             'sketch_size': SAMPLE, 'candidate_anchor_count': 16,
                             'maximum_posting_size': 128},
              'limitations': [
                  'Similarity links are conservative candidate families; manual visual and structural review remains required.',
                  'PAPX is raw fragment evidence. Style inheritance, huge-PAPX references and paragraph-end selection are unresolved.',
                  'A missing similarity link does not prove independent templates.',
                  'Long text sampling and common-feature posting limits can miss family links.',
                  'Word tokenization has weaker sensitivity for languages without space-separated words.',
                  'Unresolved decoding remains reserved and unqualified; no source text is retained.',
                  'All components touching new sources, including old siblings, are reserved from tuning.',
              ]}
    args.output.write_text(json.dumps(output, indent=2) + '\n')
    print(json.dumps({'statuses': dict(Counter(r['status'] for r in features)),
                      'paragraph_metadata': sum(r.get('papx_available', False) for r in good),
                      'edges': len(edges), 'families': len(groups),
                      'reserved_sources': sum(len(g['members']) for g in groups if g['new_members']),
                      'old_siblings_reserved': sum(len(g['members']) - g['new_members'] for g in groups if g['new_members'])}))


if __name__ == '__main__':
    main()
