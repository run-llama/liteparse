# Transcription comparison

`transcription_baseline.py` compares native raw inspection and, when available,
provisional native transcription against saved LibreOffice OOXML exports. It
verifies the current DOC, frozen original DOC and converted DOCX SHA-256 values
before comparing. It requires only the Python standard library.

```sh
python3 rust/crates/doc-read/tools/transcription_baseline.py \
  --binary /path/to/release/doc-read \
  --dataset /path/to/doc-corpus \
  --corpus /path/to/raw-story-evaluation.json \
  --previews /path/to/thumbnail-pool.json \
  --conversion-manifest /path/to/converted-preview-manifest.json \
  --conversion-manifest /path/to/converted-pagecount-manifest.json \
  --output /path/to/transcription-baseline.json

python3 -B -m unittest discover -s rust/crates/doc-read/tools \
  -p 'test_transcription_baseline.py'
```

Use `--include-report /path/to/previous-report.json` when expanding the corpus or
replaying an established cohort; it retains every previously selected source.
The helper binary is frozen beside the output report before the run.

Selection includes qualified previews and six hash-ordered examples per available
binary revision and nonempty story. The source SHA is the identifier, so duplicate
source bytes count once. A revision without eligible historical conversions is
absent; create fresh isolated-profile LibreOffice exports with timeouts and record
both source and output hashes before adding them. Historical converter versions
are only as trustworthy as the manifests that recorded them.

Reports contain counts, hashes, diagnostics and local source paths. They retain no
source text. Missing and extra character occurrences are symmetric disagreement
counts, not accuracy scores. Normalization removes whitespace/control characters
and applies NFC. Exact normalized order is measured separately; sequence matching
is bounded to short stories. Tables remain in their enclosing story; textboxes
are separated, including header textboxes. The XML reader selects one
`mc:AlternateContent` representation to avoid counting fallback shapes twice.
It retains OOXML soft and nonbreaking hyphens. Font-coded `w:sym` records are
counted under `lo_diagnostics.unresolved_font_symbols` and omitted from the text;
comparisons with these records are incomplete until font mapping is attributed.

The Python cached-field projection is a diagnostic baseline. The helper's own
projection is compared separately with its unresolved semantics and diagnostics.
The text-only diagnostic omits only source segments marked `unresolved_control`;
it preserves literal U+FFFC text and reports the excluded source-unit count.
This score does not measure object-content completeness.
Each projected story must also pass source/output partition validation: positive-width
UTF-16 spans cover the source once, UTF-8 spans cover output once, and generated list
labels use explicit zero-width anchors. Gaps, overlaps and split Unicode characters
make the comparison fail. This accounting check does not establish semantic accuracy.
Neither establishes revision visibility, hidden text or intended displayed field
values. Header/footer sharing and story relocation by conversion also require
source attribution. A zero character difference can still conceal order defects.

Before claiming better transcription than LibreOffice:

1. Audit each discrepancy against source positions, fields, formatting/revision
   properties, and saved Word evidence where available.
2. Add source-authored synthetic fixtures with independent expected visible text
   for body/table content, nested fields, notes, headers and textboxes.
3. Establish template families from structural/visual evidence and reserve entire
   families as holdout. The initial corpus intentionally has no holdout label.
4. Track proven omissions, additions, order errors and unsupported cases separately
   by story and revision. Retain uncertain disagreements as an explicit category.
5. Expand to fresh conversions for missing revisions and record converter version,
   source/output hashes, timeouts and failure outcomes.
