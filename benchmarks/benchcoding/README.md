# Local accuracy and resource experiments

**Use the [ParseBench adapter](PARSEBENCH.md) for parsing accuracy experiments.**
It combines upstream rule-based scoring with single-pass Markdown/layout
extraction, optional fixed-OCR replay, and content-addressed score caching.
The standard-library runner below remains a small infrastructure smoke harness
and supplies the shared resource-comparison/reporting code.

This is a private, local pilot. Nothing uploads evidence, dispatches remote jobs,
creates issues, or publishes a board. It uses Python 3.10+ standard-library tools
on Linux/macOS and a compiled `lit` CLI. No Python parser/LLM dependencies or API
keys are needed.

The runner scores **the same output whose parsing resources it measures**.
Scoring happens after parsing and is excluded from the parser budget. Keep this
harness and the ground truth in the maintainer checkout when evaluating another
revision's binary. Do not use evaluator changes supplied by a candidate to accept
that candidate.

## Objective and acceptance rules

Maximize accuracy with the following per-page eligibility gates:

- CPU <= 105% of current main AND <= 110% of the fastest measured main revision
  in the preceding 30 days.
- Peak RSS <= 105% of current main. There is no historical RAM gate yet; that
  extension has not been agreed.
- Every page must succeed and produce nonempty, deterministic output.
- No accuracy category may regress relative to current main.
- An eligible candidate must improve aggregate accuracy, or retain accuracy and
  improve CPU time beyond the observed measurement spread.

The local report distinguishes `eligible_accuracy_gain`, `eligible_speed_gain`,
`no_gain`, `rejected`, and `inconclusive`. Eligibility is evidence for maintainer
review, not automatic acceptance. There is no accuracy-times-speed scalar.

Checks average equally within each page/category, pages average equally within
categories, and categories receive equal aggregate weights. Failures score zero
and remain in every denominator. An overall win cannot override a category
regression. These are strict pilot rules; changing them versions the evaluator
and requires rerunning references.

## Run the pilot

From the repository root:

```sh
python3 -m unittest discover -s benchmarks/benchcoding/tests -v
cargo build --locked --release -p liteparse --bin lit
```

Build baseline and candidate binaries from their respective exact, clean source
revisions, with the same release settings. Keep their native libraries available.
`--revision` is an operator attestation, not proof of binary provenance. The
runner hashes the actual binary and records the command. Do not label an old
prebuilt binary as current main. Prefer a separate checkout for each build;
benchmark collection does not switch branches or build software itself.

Example, after building main in a separate checkout:

```sh
python3 benchmarks/benchcoding/bench.py run \
  --binary /absolute/path/to/main-checkout/target/release/lit \
  --revision "$(git -C /absolute/path/to/main-checkout rev-parse HEAD)" \
  --role main \
  --corpus benchmarks/benchcoding/smoke.json \
  --runner-id local-machine --environment-id pinned-native-build-v1 \
  --output benchmarks/benchcoding/results/main-001
```

Measure the candidate on that same machine and native-library/toolchain setup:

```sh
python3 benchmarks/benchcoding/bench.py run \
  --binary /absolute/path/to/candidate-checkout/target/release/lit \
  --revision "$(git -C /absolute/path/to/candidate-checkout rev-parse HEAD)" \
  --role candidate \
  --corpus benchmarks/benchcoding/smoke.json \
  --runner-id local-machine --environment-id pinned-native-build-v1 \
  --output benchmarks/benchcoding/results/candidate-001

python3 benchmarks/benchcoding/bench.py compare \
  --main benchmarks/benchcoding/results/main-001/result.json \
  --candidate benchmarks/benchcoding/results/candidate-001/result.json \
  --output benchmarks/benchcoding/results/comparison-001.json
```

Comparison writes JSON and a sibling `.json.md` human-readable report. A run
writes Markdown output and stderr for every page/repetition, all raw metrics,
per-check scores, and metadata in `result.json`. Existing output destinations
are never overwritten. `results/` is ignored by Git.

Exit codes: 0 = successful collection / eligible comparison; 1 = invalid input
or infrastructure error; 2 = failed collection / rejected or no-gain comparison;
3 = inconclusive comparison. A successful collection does not mean accuracy is
perfect or that the candidate passes resource gates.

### Rolling main history

Collect main daily on a quiet, fixed machine using the same corpus, environment,
timeout, repetitions, and evaluator. Archive each run in a fresh directory.
Pass previous `result.json` files using `compare --history PATH [PATH ...]`.
Only compatible, successful runs labelled `main` from the 30 days ending at the
explicit current-main reference are eligible. The fastest reference is one
actual run/revision with the lowest sum of per-page median CPU times, not a
synthetic mix of the fastest page from each revision. Its page measurements
supply the historical limits. Failed and incompatible historical runs are
excluded and counted; malformed evidence is an error.

References are frozen for a comparison. Recollect current main and recompare
before accepting a change. Start collecting now: a month of historical timings
cannot be reconstructed from Git timestamps. The report exposes history span,
run count, and bootstrap state; it only knows the history files supplied. It
cannot certify that the operator supplied all daily runs. No scheduler is
installed by this pilot.

## Accuracy corpus

`smoke.json` uses the existing sample PDF to exercise infrastructure only. Its
few presence/order assertions are **not** a meaningful product accuracy score.
Do not accept parser changes based on it.

A real manifest should contain representative pages with reviewed ground truth:

```json
{
  "version": 1,
  "cases": [{
    "id": "invoice-p2",
    "pdf": "documents/invoice.pdf",
    "page": 2,
    "checks": [
      {"kind": "full_text", "category": "text_fidelity", "text": "Complete reviewed expected Markdown goes here."},
      {"kind": "contains", "category": "tables", "text": "| Total | 42.00 |"},
      {"kind": "ordered", "category": "reading_order", "texts": ["Invoice", "Total"]},
      {"kind": "markdown", "category": "structure", "text": "# Invoice"},
      {"kind": "absent", "category": "text_fidelity", "text": "Known erroneous duplicate"}
    ]
  }]
}
```

Paths are relative to the manifest. Page indices are one-based. IDs must be
unique alphanumeric/hyphen/underscore names. The fingerprint covers the entire
manifest and source PDF bytes; edits invalidate comparisons.

- `full_text`: normalized token edit accuracy, `1 - distance / max(token counts)`.
  Penalizes insertions, deletions and substitutions, including hallucinated
  output. NFC normalization and whitespace folding are applied; punctuation,
  case, order and Markdown syntax remain significant. Scoring is quadratic in
  token count and uses linear auxiliary memory; it is outside the parser timer.
- `contains` / `absent`: normalized substring assertions.
- `ordered`: non-overlapping normalized passages must appear in sequence.
- `markdown`: exact complete line, preserving syntax and indentation.

Use full reviewed transcriptions alongside targeted checks: presence-only checks
can reward output with extra incorrect content. Table-line checks are a basic
adapter, not TEDS/GRITS or a complete table-structure metric. Keep public tuning
cases separate from maintainer acceptance cases. Established quality benchmark
adapters and a representative, licensed corpus remain follow-up work.

## Measurement contract and limits

Each page is selected from its original PDF using `--target-pages`, in a new
native process. Loading the document, preprocessing, extraction, projection,
Markdown generation, and writing stdout are inside the timer. Initialization is
paid per page; this is **not** whole-document throughput. Other source pages
remain available to the parser. CPU is user + system time from `wait4`, including
native threads. Wall time is diagnostic. The fixed profile disables OCR, image
output and links and sets one OCR worker. No network-backed parsing is used.

There is one recorded warmup and five measured repetitions per page (minimum
three). Warmup failures and output differences also invalidate a page. Runs are
sequential, with warm OS caches; they are not cold-start measurements. Keep the
machine idle and interleave/repeat main/candidate runs when confirming a result.
Runtime libraries, toolchain, CPU power settings and thread-related environment
must be pinned by the operator using the same `--environment-id`. Hardware and
OS metadata are recorded, but that label is not automatic environment isolation.

CPU/RAM gates pass only when the candidate's highest observed value is within
the factor times the reference's lowest. They fail when the candidate's lowest
exceeds the factor times the reference's highest. Overlapping ranges are
inconclusive and need another run. These are conservative observed ranges,
**not statistical confidence intervals**. A 5% tolerance will be difficult to
resolve on very short pages or a busy laptop.

Peak RSS is native-process high-water memory, converted to bytes for macOS and
Linux. It is **not simultaneous aggregate process-tree memory**. The supported
profile is native PDFs without OCR or subprocess conversion. CPU and RAM budget
violations are rejected after collection; only wall timeout terminates work.
The process group is killed on timeout/cleanup. This runner does not provide a
hard RAM limit, filesystem/network sandbox, or protection from malicious
candidates. Do not run untrusted submissions in this local pilot. A Linux cgroup
runner is needed before external/OCR workloads or untrusted participation.

## Next steps before accepting real improvements

1. Review and version a representative accuracy corpus, including full-text,
   table, reading-order and structural ground truth.
2. Establish a fixed runner, collect current main, and accumulate daily history.
3. Use the ParseBench adapter for established accuracy metrics; add separate
   uncached whole-document regression validation before acceptance.
4. Add isolated, credential-free Linux execution with process-tree accounting
   and enforced limits before accepting untrusted candidates.
5. Add experiment records (hypotheses, costs, failed attempts) and a persistent
   board once the private evaluation loop is useful.

Manual GitHub Actions validation (smoke/full dataset, caches, evidence artifacts,
and optional dedicated-runner resource enforcement) is documented in
[ParseBench: GitHub Actions](PARSEBENCH.md#github-actions).
