# Native DOCX fidelity

Usage and confirmed Word behavior are in `README.md`. Worker integration (conversion, bookmarks,
table output): `llamaparse/worker/docs/native-document-integration.md`.

## Fidelity notes

Area-specific layout and export rules are in [`FIDELITY_NOTES.md`](./FIDELITY_NOTES.md). Read
its section before changing that area:

- Package parts and imports (OOXML detection, alternate content, CSS selectors)
- Fonts, symbols and shaping (embedded fonts, symbol provenance, hyphenation)
- Bidirectional text
- Tabs and character grid
- Colors, shading and borders (theme colors, underline, shading)
- Lists
- Tables (cell margins, row heights, AutoFit, noWrap, floating tables)
- Vertical merges and merged-cell continuation
- Pagination and columns (rollback, column balancing, page-top spacing)
- Footnotes and endnotes
- Frames, shapes and drawings (drop caps, shape autofit, rotation, charts)
- Pictures
- Fields, bookmarks, links and math
- Hidden content, export and attribution

## Rendering rules

DOCX rendering has one source-driven path; do not add environment-variable feature switches.
Compare immutable binaries when evaluating changes.

Word's implementation notes can override OOXML schema defaults; record the source
in the crate README and verify changes against the full A/B sets and thumbnails.

## Evaluation

Follow the evaluation and recovery procedure in
[`llamaparse/docx-native/PLAN.md`](../../../llamaparse/docx-native/PLAN.md).
Saved page counts and break markers can disagree; report reference uncertainty alongside raw
cohort scores and preserve the fixed input population when assessing regressions.

Classify native/LibreOffice text differences separately from Word layout defects; explicit
no-wrap with noAutofit can intentionally place text beyond the physical page.

Run native builds/tests from `rust/` or select `+1.99.1` explicitly: root `--manifest-path`
does not select the nested toolchain; frozen artifacts record both `rustc --version` and
`cargo --version`.

## Tests

Derive cross-platform wrapping test thresholds from shaped font metrics, and assert that the
tested content crosses the covered-line boundary and changes the measured span.

Before pushing DOCX changes, run Clippy with all targets and warnings denied alongside the crate
tests; test compilation alone does not exercise the CI lint gate.
