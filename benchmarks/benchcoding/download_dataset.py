#!/usr/bin/env python3
"""Download immutable ParseBench CI datasets; verify cached bytes before reuse."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import urllib.parse
import urllib.request

REVISIONS = {
    'smoke': '68bbab242f749df2e2ef753daabcbbbe291d943e',
    'full': '2805a1d940f95a203e0ae4b88be9934f7765b3fc',
}
BASE = 'https://huggingface.co/datasets/llamaindex/ParseBench'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def checked_path(root, name):
    path = (root / name).resolve()
    if not path.is_relative_to(root.resolve()):
        raise ValueError('Dataset path escapes destination')
    return path


def download(suite, root):
    revision = REVISIONS[suite]
    root.mkdir(parents=True, exist_ok=True)
    marker = root / 'download.json'
    if marker.exists():
        recorded = json.loads(marker.read_text())
        discovered = {str(p.relative_to(root)) for p in root.rglob('*')
                      if p.is_file() and (p.suffix in {'.pdf', '.jsonl'} or p.name == 'expected_markdown.json')}
        if recorded['revision'] == revision and recorded['files'] and all(
            checked_path(root, name).is_file() and sha(checked_path(root, name)) == digest
            for name, digest in recorded['files'].items()
        ) and discovered == set(recorded['files']):
            print(f'Verified cached {suite} dataset at {revision}')
            return
    with urllib.request.urlopen(f'https://huggingface.co/api/datasets/llamaindex/ParseBench/revision/{revision}', timeout=60) as response:
        metadata = json.load(response)
    if metadata['sha'] != revision:
        raise ValueError('Dataset revision mismatch')
    names = [f['rfilename'] for f in metadata['siblings'] if
             f['rfilename'].endswith(('.jsonl', '.pdf')) or f['rfilename'] == 'expected_markdown.json']
    if not names or not any(n.endswith('.jsonl') for n in names):
        raise ValueError('Dataset has no scoring rules')
    # A wrong cache key must not leave another revision's extra cases in discovery.
    unexpected = [p for p in root.rglob('*') if p.is_file() and p.suffix in {'.pdf', '.jsonl'}
                  and str(p.relative_to(root)) not in names]
    if unexpected:
        raise ValueError('Destination contains another dataset; use a fresh directory')

    def fetch(name):
        destination = checked_path(root, name)
        destination.parent.mkdir(parents=True, exist_ok=True)
        temporary = destination.with_name(destination.name + '.partial')
        url = f'{BASE}/resolve/{revision}/{urllib.parse.quote(name)}'
        with urllib.request.urlopen(url, timeout=120) as source, temporary.open('wb') as target:
            while block := source.read(1024 * 1024):
                target.write(block)
        temporary.replace(destination)
        return name, sha(destination)

    with ThreadPoolExecutor(max_workers=4) as pool:
        hashes = dict(pool.map(fetch, names))
    marker.write_text(json.dumps({'revision': revision, 'files': hashes}, indent=2) + '\n')
    print(f'Downloaded {len(hashes)} files at {revision}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--suite', choices=REVISIONS, default='smoke')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--revision-only', action='store_true')
    args = parser.parse_args()
    if args.revision_only:
        print(REVISIONS[args.suite])
    elif args.output is None:
        parser.error('--output is required')
    else:
        download(args.suite, args.output)
