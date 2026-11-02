#!/usr/bin/env python3
"""Run the local validation sequence used by GitHub Actions; no publishing."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

import bench

HERE = Path(__file__).resolve().parent


def call(script, *arguments, allowed=(0,)):
    result = subprocess.run([sys.executable, str(HERE / script), *map(str, arguments)])
    if result.returncode not in allowed:
        raise RuntimeError(f'{script} failed with exit code {result.returncode}')


def acceptance(report, enforce_resources):
    if report['category_regressions']:
        raise ValueError('Accuracy category regression: ' + ', '.join(report['category_regressions']))
    if enforce_resources:
        if report['history_span_days'] < 29:
            raise ValueError('Resource acceptance requires compatible main history spanning at least 29 days')
        if report['status'] in {'rejected', 'inconclusive'}:
            raise ValueError('Resource acceptance failed: ' + report['status'])


def check_replay(record_dir, replay_dir):
    result = json.loads((replay_dir / 'result.json').read_text())
    for case in result['cases']:
        expected = (record_dir / f"{case['id']}.json").read_bytes()
        for sample in [case['warmup'], *case['samples']]:
            if expected != (replay_dir / sample['output']).read_bytes():
                raise ValueError('OCR record/replay output mismatch: ' + case['id'])


def main(args):
    if args.enforce_resources and args.history_dir is None:
        raise ValueError('Configure a persistent history directory for resource acceptance')
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = args.output / 'manifest.json'
    call('parsebench.py', 'prepare', '--dataset', args.dataset, '--output', manifest)
    workers = [('main', args.main_worker, args.main_revision), ('candidate', args.candidate_worker, args.candidate_revision)]
    # Prime both before timing: any new image request is present in the same
    # frozen snapshot for both references. OCR engine/model changes are out of scope.
    for role, worker, _ in workers:
        call('parsebench.py', 'prime', '--manifest', manifest, '--worker', worker,
             '--ocr-cache', args.ocr_cache, '--ocr-id', args.ocr_id, '--output', args.output / f'prime-{role}')
    history = sorted(args.history_dir.glob('*.json')) if args.history_dir and args.history_dir.exists() else []
    for role, worker, revision in workers:
        call('parsebench.py', 'run', '--manifest', manifest, '--worker', worker,
             '--revision', revision, '--role', role, '--runner-id', args.runner_id,
             '--environment-id', args.environment_id, '--repetitions', args.repetitions,
             '--ocr-cache', args.ocr_cache, '--ocr-id', args.ocr_id,
             '--score-cache', args.score_cache, '--output', args.output / role)
        check_replay(args.output / f'prime-{role}', args.output / role)
        if role == 'main' and args.history_dir:
            args.history_dir.mkdir(parents=True, exist_ok=True)
            source = args.output / role / 'result.json'
            data = source.read_bytes()
            destination = args.history_dir / (bench.digest(data) + '.json')
            if not destination.exists():
                with destination.open('xb') as stream:
                    stream.write(data)
    # Also exercise the independent no-OCR profile. Never mix its resource data
    # with replay measurements. Existing upstream unit tests verify score-cache hits.
    call('parsebench.py', 'run', '--manifest', manifest, '--worker', args.candidate_worker,
         '--revision', args.candidate_revision, '--runner-id', args.runner_id,
         '--environment-id', args.environment_id, '--repetitions', args.repetitions,
         '--score-cache', args.score_cache, '--output', args.output / 'no-ocr', allowed=(0, 2))
    comparison = args.output / 'comparison.json'
    history_args = ['--history', *history] if history else []
    call('bench.py', 'compare', '--main', args.output / 'main/result.json',
         '--candidate', args.output / 'candidate/result.json', *history_args,
         '--output', comparison, allowed=(0, 2, 3))
    report = json.loads(comparison.read_text())
    report_text = Path(str(comparison) + '.md').read_text()
    scope = 'Enforced on the configured dedicated runner' if args.enforce_resources else 'Report-only resources; accuracy regressions still fail validation'
    summary = f'# ParseBench CI validation\n\n{scope}.\n\n' + report_text
    no_ocr = json.loads((args.output / 'no-ocr/result.json').read_text())
    missing = sum(c['status'] != 'ok' for c in no_ocr['cases'])
    summary += f'\nNo-OCR diagnostic: {missing} failed/empty pages (not the OCR-enabled acceptance profile).\n'
    (args.output / 'summary.md').write_text(summary)
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as stream:
            stream.write(summary)
    acceptance(report, args.enforce_resources)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('dataset', 'main-worker', 'candidate-worker', 'output', 'ocr-cache', 'score-cache'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('main-revision', 'candidate-revision', 'runner-id', 'environment-id', 'ocr-id'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--history-dir', type=Path)
    parser.add_argument('--repetitions', type=int, default=5)
    parser.add_argument('--enforce-resources', action='store_true')
    args = parser.parse_args()
    try:
        main(args)
    except (ValueError, RuntimeError, OSError) as exc:
        parser.exit(1, f'error: {exc}\n')
