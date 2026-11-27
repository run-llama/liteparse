# Binary Word decoding

Build, run and evaluation commands are in `README.md`. Read `../docx-read/AGENTS.md` before
connecting the shared document model or renderer.

## Format notes

Area-specific decoding rules are in [`FORMAT_NOTES.md`](./FORMAT_NOTES.md). Read its section
before changing that area:

- Container, CLX pieces, FKP/PAPX paragraph boundaries, source-to-output mapping
- Character properties and SPRMs (CFSpec, CIstd, capitalization, U000B, Ico colors, underline)
- Symbols and fonts
- Fields and form fields (EQ, ruby, SYMBOL, dropdowns, legacy payloads)
- Lists and numbering (labels, LFOLVL/LST, LSID, ArabicLZ/ArabicAlpha, RTL, NumRM)
- Tables (TDefTable, TC80, nested rows, table styles, widths, autofit, shading, borders)
- Revisions, visibility and comments
- Footnotes and endnotes
- Sections, headers and document settings (DOP, SBOrientation)
- Pictures
- Rendering qualification and layout

DOC rendering rules (cell replay, generated labels, retry bundles) load from
`rust/crates/doc-render/AGENTS.md`.

## Crate-wide rules

Binary decoding preserves UTF-16 character positions across compressed and uncompressed pieces;
source controls and field instructions remain in raw inspection output until structure and
revision semantics are decoded.

Keep projected text separate from raw stories and account for every source CP, including
suppressed instructions and unresolved tails; cached field projection requires field-type
semantics and cannot establish revision visibility.

Use root-directory ownership for OLE streams, bound allocation-chain work and expanded text, and
report unsupported or malformed structures explicitly. Corpus success at the raw-story stage does
not qualify a document for production rendering.

`docx_export` is the best-effort path: it converts DOC to a DOCX package for the `docx-read`
renderer, degrades unsupported structures to text or skips them with a diagnostic, and fails only
on unreadable containers. The strict rules below apply to the inspection and `render_model`
modules. Development tool: `doc-to-docx IN.doc OUT.docx` (`--paragraphs` dumps paragraph
modifiers).

## Fixtures and evaluation

Keep synthetic fixtures in source; document samples and private corpus manifests stay outside the
repository.

Freeze evaluation sources before building and derive provenance from that snapshot; concurrent
edits in the shared checkout can otherwise invalidate separate-change comparisons. Use separate
Cargo target directories for baseline and candidate snapshots; copied timestamps can reuse a stale
library even when source hashes differ.

## Before pushing

Run crate tests and Clippy with all targets and warnings denied before each push.
