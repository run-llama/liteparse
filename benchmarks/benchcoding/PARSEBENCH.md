# ParseBench accuracy with a fixed resource budget

This is the primary accuracy path for the private pilot. It uses upstream
ParseBench's deterministic metrics, not question answering or an LLM judge.
The small `bench.py run` manifest remains only an infrastructure test.

## What runs

1. `prepare` loads ParseBench JSONL/sidecar ground truth into a frozen manifest.
   It fingerprints the PDFs, rules, installed evaluator source, and dependencies.
2. A Rust worker parses each unique PDF page once per repetition, producing
   **both Markdown and layout blocks from that same parse**. Shared text-content
   and formatting PDFs are measured only once. Multi-page PDFs retain separate
   per-page resource budgets; their outputs are reassembled for document scoring.
3. After all timing ends, the upstream evaluator scores the saved outputs.
   Its LiteParse normalization converts pipe tables to HTML and normalizes
   layout coordinates. Scoring is never inside the parser's CPU budget.
4. The existing comparison command applies the 5% current-main and 10%
   fastest-main CPU ceilings, and the 5% current-main RAM ceiling, per page.

| Category | Upstream metric |
| --- | --- |
| Tables | `grits_trm_composite` |
| Charts | `rule_pass_rate` |
| Content faithfulness | `content_faithfulness` |
| Semantic formatting | `semantic_formatting` |
| Visual grounding | `layout_element_rule_pass_rate` |

Scores average over documents within each category, then equally over the
selected categories. Resource failures remain in the denominator and prevent
eligibility. Missing required metrics are errors, not silently omitted scores.
Layout cases without layout annotations are excluded at manifest preparation
and listed explicitly; their ground truth cannot support this metric. There is
no fallback to a different visual-grounding metric for a weak candidate.

A subset's average is a development score, not the full ParseBench leaderboard
score. Acceptance requires the frozen full corpus and human review. This adapter
pins private upstream normalization/evaluation helpers to a specific revision;
upstream upgrades require integration testing and new reference runs.

## Setup and data

Python 3.12+ is required by ParseBench; the pinned dependencies were tested with
Python 3.13. No API keys are needed for scoring.

```sh
python3.13 -m venv benchmarks/benchcoding/.venv
benchmarks/benchcoding/.venv/bin/pip install -r benchmarks/benchcoding/requirements-parsebench.txt
cargo build --locked --release -p liteparse --example bench_page
mkdir -p benchmarks/benchcoding/results
```

Use an existing local [ParseBench dataset](https://huggingface.co/datasets/llamaindex/ParseBench),
or download it with the official `parse-bench download` CLI in that environment.
The dataset's small `test-data` branch is useful for integration testing. The
manifest hashes the actual downloaded bytes, so a floating remote dataset cannot
silently change an existing experiment. Freeze the source dataset revision when
collecting acceptance baselines.

```sh
benchmarks/benchcoding/.venv/bin/python benchmarks/benchcoding/parsebench.py prepare \
  --dataset /path/to/parsebench-data \
  --output benchmarks/benchcoding/results/parsebench.json
```

For iteration, prepare a separate manifest with `--limit-per-category 20`.
Selection is deterministic from `--seed` (default `pilot-v1`), category and test
ID. `--groups table,text_content` can narrow investigation; it creates a different
objective and cannot be compared with full-category runs.

## OCR preparation and replay

Recognition is computed once, outside timing, then replayed in-process through
LiteParse's existing OCR-engine trait. Rendering, image preparation, selection
of OCR regions, cache lookup, OCR merging, projection and Markdown generation
still run on every measured repetition.

```sh
benchmarks/benchcoding/.venv/bin/python benchmarks/benchcoding/parsebench.py prime \
  --manifest benchmarks/benchcoding/results/parsebench.json \
  --worker target/release/examples/bench_page \
  --ocr-cache benchmarks/benchcoding/results/ocr-v1 \
  --ocr-id tesseract-pinned-models-eng-v1 \
  --output benchmarks/benchcoding/results/prime-v1
```

The default preparation engine is built-in Tesseract. `prime --ocr-server-url
http://localhost:8829/ocr` can instead record an already-running HTTP OCR engine.
No server is started or provisioned automatically. The operator must choose an
immutable `--ocr-id` covering the actual engine build, model weights, and engine
settings. For Tesseract, include the traineddata version/hash in that identity.

Cache keys include the engine profile, grayscale preference, exact rendered
image bytes, dimensions, language and DPI. Boundaries/crops are therefore part
of the key. Record mode writes response files atomically; replay mode never
creates an OCR engine, calls an upstream server, or writes cache entries. A miss
is a parse error, never an empty OCR result or a live fallback.

`prime.json` records preparation CPU/wall time and peak RSS separately. The
runner fingerprints the full cache snapshot before and after measurement. If a
candidate changes image preparation and needs new entries, prime a fresh/shared
snapshot and **rerun both main and candidate against that same snapshot**.
Protect this directory from candidate edits for acceptance runs.

**Replay is a fixed-OCR experiment profile, not end-to-end OCR performance.**
It is suitable for extraction/layout/merging changes. Changes to OCR engines or
models require a new profile and independent uncached end-to-end validation.
Recognition CPU/RAM is excluded from replay measurements; never compare replay
numbers with uncached/no-OCR baselines. The OCR snapshot and settings are part
of comparison compatibility, and reports retain the profile in result JSON.

## Measure, score and compare

```sh
benchmarks/benchcoding/.venv/bin/python benchmarks/benchcoding/parsebench.py run \
  --manifest benchmarks/benchcoding/results/parsebench.json \
  --worker target/release/examples/bench_page \
  --revision YOUR_EXACT_SOURCE_COMMIT --role candidate \
  --runner-id dedicated-machine --environment-id pinned-native-build-v1 \
  --ocr-cache benchmarks/benchcoding/results/ocr-v1 \
  --ocr-id tesseract-pinned-models-eng-v1 \
  --score-cache benchmarks/benchcoding/results/scores \
  --output benchmarks/benchcoding/results/candidate-001
```

Omit both OCR arguments for a separate no-OCR profile. The measured `run`
command deliberately has no record/live-OCR option. Use the same fixed worker
source when building main and candidate: in a main checkout that predates this
example, copy `crates/liteparse/examples/bench_page.rs` from the evaluator branch
before building it against main. Only the benchmark worker is added; parser
source stays at the recorded revision. The binary hash is retained, but source
revision labels are still operator attestations, not verified build provenance.

Run the same command with the main-built worker, `--role main` and a fresh output
directory, then compare using `bench.py compare --main ... --candidate ...
--history ... --output ...` as described in [README.md](README.md). Daily history
must use this exact manifest, profile, evaluator and machine. These references
cannot reuse timings from the earlier Markdown-only smoke harness.

## Other work we avoid repeating

- **Duplicate parsing:** content and formatting cases share one parsed document;
  identical PDF bytes are deduplicated across paths and categories.
- **Second layout parse:** Markdown and blocks come from one Rust API call.
- **Unchanged scoring:** cache keys cover output, ground truth, evaluator source,
  dependency versions and adapter. Changed outputs or rules are rescored. The
  cache is local trusted evidence; do not accept a candidate-supplied score cache.
- **Unnecessary full sweeps:** deterministic category-balanced subsets support
  development. Full-corpus verification remains required for acceptance.

Parser outputs/resource measurements are **never reused across measured runs**.
Only OCR recognition and deterministic scoring are cached. This preserves the
meaning of speed measurements. Timing is serial; scoring starts afterward so
expensive table metrics cannot contend with the parser. A score-cache hit still
writes the detailed evaluation into the run's evidence directory.

## Validation

```sh
benchmarks/benchcoding/.venv/bin/python -m unittest discover -s benchmarks/benchcoding/tests -v
cargo test --locked --release -p liteparse --example bench_page
```

The optional integration tests exercise actual upstream rule scoring, table
normalization, layout page numbering and score-cache invalidation. The Rust
tests cover OCR cache key inputs, successful replay and strict cache misses.
The existing local-runner limitations (post-run resource gates, no sandbox,
operator-pinned hardware/dependencies) still apply.
