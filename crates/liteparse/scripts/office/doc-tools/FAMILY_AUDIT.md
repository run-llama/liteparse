# DOC family leakage audit

Run this before using newly ingested documents for parser tuning:

```sh
python3 rust/crates/doc-read/tools/family_audit.py \
  --binary /path/to/frozen-release/doc-read \
  --inventory /path/to/current-inventory.json \
  --new-sources /path/to/intake-delta.json \
  --preview-pool /path/to/current-preview-pool.json \
  --output /path/to/family-audit.json

python3 -B -m unittest discover -s rust/crates/doc-read/tools \
  -p 'test_family_audit.py'
```

The inventory contains `rows` with `path` and `sha256`; the intake delta contains
`new_sources` with `sha256`. Preview rows identify `source_sha256`, `reference_path`,
`reference_sha256` and optional `crop`. The helper is copied beside the report;
source and preview hashes are verified. Reports retain hashed features, counts,
source identifiers and thresholds. They contain no document text or filenames.
`--reuse-features` accepts a previous report only when its inventory and helper
hashes match; preview hashes are checked again.

Features include five-token text shingles, five-paragraph control/length patterns,
and three-record sequences of raw PAPX modifiers with stylesheet indices renamed
by first occurrence. Adjacent identical PAPX fragments are collapsed. These are
source fingerprints; they do not resolve stylesheet inheritance or paragraph-end
property selection. Each feature family retains its 128 lowest 64-bit hashes.
Long text uses the first and last portions and is marked incomplete. Optional
saved previews supply a 16-by-24 grayscale ink profile, computed with Pillow.

Candidate pairs share rare sketch anchors or have saved previews. Common posting
lists are skipped, except exact-body groups remain connected. Links require high
text overlap or combined text, structure and visual evidence. The report lists
all numeric thresholds and every link reason. Connected components are
conservative candidate families, including transitive links through old sources.
A visual resemblance or numerical threshold alone does not prove common ancestry.

Every component touching new source bytes is reserved from future tuning. A
component containing an old source is excluded from the strict unseen-family
candidate set: previous exposure cannot be undone by reserving it later.
Components with unavailable metadata or sampled text also remain unresolved.
Remaining components are **provisional candidates**, requiring manual link and
leakage review before certifying a held-out score. Freeze the final assignment and
record its hash before inspecting parser quality on those documents.

A missing link does not prove independent templates. Style resolution gaps,
renamed or substantially rewritten templates, limited preview coverage, feature
sampling and common-anchor pruning can hide relationships. Report unsupported
sources and unresolved components separately; do not silently remove them from
coverage statistics or call the remaining set representative of the full corpus.
