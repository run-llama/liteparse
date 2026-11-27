#!/usr/bin/env python3
"""Run raw-story inspection over a local corpus; retain hashes and counts, not document text."""
import argparse
import collections
import concurrent.futures
import hashlib
import json
import subprocess
import time
from pathlib import Path


def evaluate(binary, path):
    started = time.monotonic()
    row = {"id": hashlib.sha256(str(path).encode()).hexdigest()[:12]}
    try:
        row["source_sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
        result = subprocess.run([str(binary), str(path)], capture_output=True, timeout=30)
        if result.returncode:
            row.update(status="rejected", reason=result.stderr.decode(errors="replace").strip())
        else:
            data = json.loads(result.stdout)
            row.update(
                status=data["status"],
                version=hex(data["binary_version"]),
                pieces=data["piece_count"],
                trailing_container_bytes=data["trailing_container_bytes"],
                stories=[{
                    "kind": story["kind"], "utf16_units": story["utf16_units"],
                    "text_sha256": hashlib.sha256(story["text"].encode()).hexdigest(),
                } for story in data["stories"]],
            )
    except Exception as error:
        row.update(status="harness_error", reason=str(error))
    row["seconds"] = time.monotonic() - started
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("dataset", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    paths = sorted(p for p in args.dataset.rglob("*") if p.is_file()
                   and p.suffix.lower() == ".doc" and not p.name.startswith("._"))
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        rows = list(pool.map(lambda p: evaluate(args.binary.resolve(), p), paths))
    report = {
        "stage": "raw stories; no formatting, revision resolution or rendering",
        "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        "total": len(rows),
        "statuses": dict(collections.Counter(r["status"] for r in rows)),
        "reasons": dict(collections.Counter(r["reason"] for r in rows if "reason" in r)),
        "accepted_versions": dict(collections.Counter(r["version"] for r in rows if "version" in r)),
        "rows": rows,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k != "rows"}, indent=2))


if __name__ == "__main__":
    main()
