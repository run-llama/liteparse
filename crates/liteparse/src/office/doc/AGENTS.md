# Binary Word reader (`office::doc`)

Read `../docx/AGENTS.md` before changing how `.doc` content reaches the Word layout.

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

## Module-wide rules

Binary decoding preserves UTF-16 character positions across compressed and uncompressed pieces;
source controls and field instructions remain in raw inspection output until structure and
revision semantics are decoded.

Keep projected text separate from raw stories and account for every source CP, including
suppressed instructions and unresolved tails; cached field projection requires field-type
semantics and cannot establish revision visibility.

Use root-directory ownership for OLE streams, bound allocation-chain work and expanded text, and
report unsupported or malformed structures explicitly.

`docx_export` is the best-effort path liteparse uses: it converts DOC to a DOCX package for
`office::docx`, degrades unsupported structures to text or skips them with a diagnostic, and
fails only on unreadable containers. The strict rules above apply to the inspection and
`render_model` modules.

## Fixtures

Keep synthetic fixtures in source; document samples stay outside the repository.
