# Word decoder (`office::docx`)

Usage and confirmed Word behavior are in `README.md`.

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

Word on Windows, not LibreOffice, is the fidelity reference. DOCX rendering has one
source-driven path; do not add environment-variable feature switches.

Word's implementation notes can override OOXML schema defaults; record the source in
`README.md` when a rule depends on one.

Classify differences from LibreOffice output separately from Word layout defects; explicit
no-wrap with noAutofit can intentionally place text beyond the physical page.

## Tests

Derive cross-platform wrapping test thresholds from shaped font metrics, and assert that the
tested content crosses the covered-line boundary and changes the measured span.

Run `cargo test -p liteparse --lib office::` and Clippy with all targets before pushing.
