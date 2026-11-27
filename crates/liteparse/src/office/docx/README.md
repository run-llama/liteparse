# docx-read

Native `.docx` decoder. It lays a Word document out into pages and writes the worker's
`extract` file protocol (`index.json` + `pages.jsonl` + rasters), plus a screenshot-only
PDF, like `pptx-read` does for presentations. Plan: [`llamaparse/docx-native/PLAN.md`](../../../llamaparse/docx-native/PLAN.md).

## Usage

```
docx-read -c config.json input.docx index.json pages.jsonl
```

The config is `pptx-read`'s (`rust/crates/pptx-read/src/config.rs`); its `presentation`
block is ignored. Exit codes and error codes match `pptx-read`. The index has
`sourceFormat: "docx"`, and each page record a `document` block (section, page number).

## One rendering path

Rendering behavior is selected by document properties, with no environment-variable
feature switches. Bidirectional text, merged-cell continuation allocation, column
balancing and column-width measurement use the same path for every document.
Ruby, drop caps, endnote placement and automatic hyphenation follow their source
properties. Existing unsupported cases remain fidelity work; there is no alternate
experimental runtime mode. Evaluate changes by comparing immutable binaries.

`DOCX_DEBUG` and `DOCX_IMG_LOG` only write diagnostics. `DOCX_READ_WORKERS` controls
render concurrency; it does not select a different layout algorithm. Output options
in the public JSON configuration remain supported.

Paragraphs continuing into a different-width column replay their consumed line
intervals before measuring the remaining text at the new width. Full-paragraph
shaping preserves bidirectional context, list labels, notes and anchor ownership;
earlier page placements retain their original line data. Generated drop-cap
paragraphs still need continuation support for their custom source mapping.
Oversized header/footer reservations retain a minimum 12-point body interval so
pagination can make progress without dropping body content.

Splittable body table rows retain their remaining source slices when the first
indivisible line exceeds a fresh page's available body interval. The progress
fallback consumes that first slice and carries the tail to the next destination.
Successful source consumption or reduced minimum-height debt resets the stalled
retry counter, allowing rows to continue through more than 200 page fragments.
A completely consumed nested row retains its natural final edge without an empty
continuation. Explicit exact-height row behavior remains separate.

Percentage table grids resolve in both directions, preserving explicit
preferences above 100%. For compatibility modes below 15, the percentage basis
includes this table's outer cell margins in addition to the available story or
inner-cell width. Saved Word grids at 100%, 103.5% and 112.5% support this legacy
calculation; it is an empirical compatibility rule rather than a normative
specification claim. Modern documents use the available width alone.

AutoFit applies shaped text and inline-object width constraints to the shared
column grid, including spanning cells. Nested measurements synthesize missing
grid columns with the same rules as final layout. Rotated cells contribute their
transverse stacked extent; exact-row measurement includes neighboring bottom
padding and cell spacing in the available longitudinal capacity. Fixed-layout
row exceptions do not add content-driven width requirements.
Non-percentage AutoFit resolves direct cell-width preferences on the shared grid
before content constraints. Absolute table preferences can span page margins;
the available-width limit governs content-driven growth separately. Automatic
width negotiates cell preferences within the larger of the available width and
the resolved saved grid. Conflicting requests share the remaining space without
shifting a saved grid that already fills this envelope. Without cell preferences,
the available width bounds an oversized saved grid. Nested preferred widths contribute soft requests;
fixed nested extents remain indivisible. An omitted width type means twips; zero and `nil` select automatic
width. Direct table width and the first row's width exception determine the table
preference; later row exceptions and table-style width properties are ignored.
The source `growAutofit` setting permits content-driven growth beyond the available
width. Without it, non-percentage AutoFit jointly relaxes word and raster width
requests toward glyph/padding floors, and clips direct inline raster paint at the
owning cell width while retaining source image dimensions, text glyph floors and
cell padding. Nested tables retain their own row policy, including fixed layout;
merged-cell remeasurement retains clipping at the destination width.
Percentage tables, vector objects and fixed nested tables can still overflow
when their combined indivisible extent exceeds available space. Exact Word
mixed-media column allocation and full maximum-content negotiation remain fidelity work.
Column redistribution uses fixed preferred-width weights, so unconstrained columns
yield space proportionally while shared spanning-cell and indivisible-content
bounds remain satisfied. Zero-width source columns use the mean positive preferred
width as their redistribution scale. This follows the proportional reduction in
ECMA-376 Part 1, section 17.18.87, as algorithmic guidance; it does not establish
exact Word allocation for all tables.
Source `noWrap` gives dxa cell preferences priority over other cells' word-width
requests; these preferences yield only when the remaining cells reach their hard
floors. Auto/percentage cell widths treat each explicit line as unbroken content
for sizing. Table-style and conditional values are inherited, conditional values
apply to the affected whole row, and direct cell values can reset them to false.
These are source properties, not runtime switches. See the [noWrap semantics](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.nowrap)
and [Word conditional-row rule](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/348cf5dd-492c-4e2b-9fe3-c4996f8c52e5).
Table layout comes from direct table properties and row exceptions; Word
disallows `tblLayout` and `tblW` in table style properties
([MS-OI29500, section 17.7.6.4](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/9110e397-e8e2-4e23-af71-2fa84eab74f5)).

Bidirectional reordering applies Unicode L1 whitespace resets using original
character classes. Explicit RTL run analysis can substitute strong directional
marks for spaces, so the analysis string alone cannot supply those classes.
Paragraph side indents resolve logical start/end after style inheritance;
Transitional left/right attributes are aliases of those logical edges (ECMA-376
Part 4, section 14.3.1.2; Part 1, section 17.3.1.12). Negative values retain their
outward displacement. Conflicting same-element legacy/modern aliases remain
separate fidelity work.

### RTL first-line indentation and numbering

First-line and hanging indents apply at the paragraph's logical start edge
(ECMA-376 Part 1, section 17.3.1.12). RTL paragraphs change the first line's right
boundary; later lines use normal paragraph bounds. These intervals also govern
wrapping and exact-prefix column continuation.

Numbering shapes in its own bidi context before placement, preserving logical
label text and mirrored bracket glyphs. After body justification, labels and
suffixes use the resolved start anchor. Emission keeps a label separate when its
visual position would give a following body run an invalid advance.

Legacy numbering `left`/`right` interpretation retains the existing policy.
[MS-OE376's numbering note](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/6246bd07-7da6-4295-98d3-526d9bdd2adf)
refers to [the paragraph alignment note](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/26ecf09a-0f0b-4574-9907-ebd1ddf3015f),
whose bidi reversal is documented for Word 2007/SP1/SP2. Independent LibreOffice
controls use physical left/right label alignment. Newer Word alias semantics
remain visually unverified. Omitted numbering alignment retains the paragraph's
leading-side default. There are no environment-variable feature switches.

## How it works

`docx-read` depends on `pptx-read` as a library for the package, XML tree, fonts, scene,
rasterizer, outputs, screenshot worker and PDF writer. Its own work is Word layout:

| Stage                                                                 | Files                      |
| --------------------------------------------------------------------- | -------------------------- |
| Properties and the style cascade (docDefaults, table, paragraph, run) | `props.rs`, `styles.rs`    |
| Body, sections, runs, fields, pictures, tables; list numbering        | `parse.rs`, `numbering.rs` |
| Line layout: shaping, breaking, tabs, alignment, line heights         | `text.rs`, `metrics.rs`    |
| Paragraph spacing and borders, table rows and cells                   | `layout.rs`                |
| Pagination, headers and footers, floating pictures                    | `paginate.rs`              |
| Driver and outputs                                                    | `extract.rs`               |

Microsoft Word on Windows, not LibreOffice, is the fidelity reference. Rules are checked
against the page counts (`docProps/app.xml`) and page-break markers
(`w:lastRenderedPageBreak`) Word saves, and against page-1 thumbnails.

## Document-embedded fonts

Before layout, the parser registers font-table relationships for regular, bold,
italic, and bold-italic faces in the document-local font system. Registration is
automatic, including when system fonts are disabled. Installed matching families
retain the shared font engine's precedence; embedded subset faces are registered
under the family declared by the document.

Obfuscated font parts use the reversed GUID key repeated over the first 32 bytes.
Raw TrueType/OpenType parts require Word's zero key, following the
[Word font-part implementation notes](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/1663dabc-5d98-463f-889e-bcd9b77c3d34).
Malformed keys, relationships, and font payloads produce nonfatal
`EMBEDDED_FONT_INVALID` warnings. Package size limits still apply. Font data stays
in memory for the document; missing glyphs and fonts still depend on fallback.

Explicit `w:sym` characters retain their source font and hexadecimal code separately
from accessible Unicode text. When that exact installed or embedded family has the
requested glyph, its outline and advance are used. Windows Symbol character maps
support their low-byte/private-use aliases; Unicode character maps require the exact
code. Missing families or glyphs retain Unicode fallback, and configured font
substitutions retain the ordinary resolution path. The explicit symbol code is
independent of surrounding run fonts and capitalization, following Microsoft's
[SymbolChar semantics](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.symbolchar?view=openxml-3.0.1).

Text export preserves interior source spaces and nonbreaking spaces across font
and style boundaries. Whitespace stays in its logical source slot, including
slots between anchored objects. Adjacent text can carry it; a standalone run
retains its source position when needed. Glyph positions and the existing
line-edge whitespace handling stay intact. Some leading spaces at paragraph
or wrapped-line starts remain omitted from text export.

Text export also omits runs whose transformed typographic bounds lie wholly
outside the physical page. Partially intersecting runs retain their full Unicode.
`TEXT_OUTSIDE_PAGE` reports the number of nonempty body/note runs and affected
pages subject to this clipping rule across the full laid-out document, before
page selection or text-output settings. The diagnostic excludes header/footer
and generated line-number reading keys, fully transparent runs and whitespace-only
runs; it contains no source text. It identifies potential content loss without changing page layout
or exported text. Absence of this warning does not establish complete content
preservation, including overlapping or partially clipped text.

## Microsoft font metrics

Word paginates with the metrics of the fonts the document asks for. Many Microsoft fonts
have no metric-compatible open clone in the production font set. For those,
`src/metrics/word_tables.rs` stores **numeric metrics only**: OS/2 win and typo vertical
metrics, hhea values, underline and strikeout metrics, advance widths per codepoint
(Latin, Latin-1, Latin Extended A/B, Greek, Cyrillic, general punctuation, currency,
letterlike, arrows, math operators, box drawing, shapes, dingbats; for proportional East
Asian fonts also the 4,344 CJK codepoints the evaluation corpus uses), kerning pairs over
ASCII, and the family's localized names. No outlines or font program data are stored,
and no font file is committed. When such a family is not installed, line heights,
advances and kerning come from the table and glyphs are drawn from the open substitute
at the table's positions (`render.wordMetrics`, on by default). Monospaced East Asian fonts (MS Gothic, SimSun, NSimSun) keep Latin
ranges only; their ideographs are one em wide.

The table is generated by `scripts/gen_word_metrics.py` from a local manifest of font
files (kept outside the repository, with each file's sha256 in the generated header).
Per family: source, commit and font version.

| Family                  | Source                                     | Commit       | Version      |
| ----------------------- | ------------------------------------------ | ------------ | ------------ |
| Aptos                   | Microsoft Aptos download                   |              | Version 2.01 |
| Aptos Display           | Microsoft Aptos download                   |              | Version 2.01 |
| Aptos Narrow            | Microsoft Aptos download                   |              | Version 2.01 |
| Aptos SemiBold          | Microsoft Aptos download                   |              | Version 2.01 |
| Aptos Light             | Microsoft Aptos download                   |              | Version 2.01 |
| Segoe UI                | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.65 |
| Segoe UI Light          | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.65 |
| Segoe UI Semibold       | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.65 |
| Segoe UI Semilight      | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.65 |
| Segoe UI Black          | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 2.02 |
| Calibri Light           | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 6.26 |
| Verdana                 | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.33 |
| Cambria                 | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 6.98 |
| Cambria                 | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 6.99 |
| Georgia                 | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.59 |
| Tahoma                  | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 7.01 |
| Century Gothic          | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 2.35 |
| Trebuchet MS            | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.15 |
| Century                 | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 1.20 |
| Bookman Old Style       | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 2.35 |
| Bookman Old Style       | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 2.36 |
| Garamond                | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 2.40 |
| Book Antiqua            | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 2.35 |
| Candara                 | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.62 |
| Palatino Linotype       | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.03 |
| Comic Sans MS           | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.14 |
| Corbel                  | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 6.01 |
| Corbel Light            | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 6.01 |
| Franklin Gothic Medium  | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.01 |
| Franklin Gothic Medium  | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.02 |
| Arial Black             | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.23 |
| Consolas                | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 7.00 |
| Constantia              | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.93 |
| Impact                  | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.11 |
| Lucida Sans Unicode     | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.01 |
| Lucida Console          | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.01 |
| Microsoft Sans Serif    | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 7.02 |
| Sylfaen                 | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.07 |
| Bahnschrift             | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 2.07 |
| MS Gothic               | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.32 |
| MS PGothic              | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.32 |
| MS UI Gothic            | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.32 |
| SimSun                  | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.21 |
| NSimSun                 | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.21 |
| Yu Gothic               | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 1.93 |
| Yu Gothic Light         | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 1.93 |
| Yu Gothic Medium        | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 1.93 |
| Microsoft YaHei         | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 6.30 |
| Malgun Gothic           | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 6.68 |
| David                   | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 1.00 |
| Leelawadee UI           | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.05 |
| Nirmala UI              | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 1.45 |
| Ebrima                  | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.16 |
| Gadugi                  | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 1.13 |
| Segoe UI Symbol         | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 6.23 |
| Segoe UI Emoji          | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 1.51 |
| Segoe UI Historic       | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 1.07 |
| Cambria Math            | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 6.99 |
| MS Reference Sans Serif | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 2.00 |
| Wingdings 2             | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 1.55 |
| Wingdings 3             | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 1.55 |
| Webdings                | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.01 |
| Segoe Print             | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.04 |
| Segoe Script            | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.03 |
| Gabriola                | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.93 |
| Sitka Text              | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 2.00 |
| Ink Free                | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 2.00 |
| Microsoft Himalaya      | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.23 |
| Mongolian Baiti         | github.com/MauCariApa-com/windows-11-fonts | 96cc06871ca9 | Version 5.53 |
| MS Mincho               | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 5.10 |
| MS PMincho              | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 5.10 |
| Meiryo                  | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 6.20 |
| Meiryo UI               | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 6.20 |
| FangSong                | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 5.01 |
| KaiTi                   | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 5.01 |
| SimHei                  | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 5.03 |
| DFKai-SB                | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 5.00 |
| Franklin Gothic Book    | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 2.01 |
| Gill Sans MT            | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 1.65 |
| Tw Cen MT               | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 1.02 |
| Century Schoolbook      | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 2.35 |
| Lucida Sans             | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 1.67 |
| Rockwell                | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 1.65 |
| Arial Rounded MT Bold   | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 1.51 |
| Agency FB               | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 1.01 |
| Franklin Gothic Demi    | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 2.00 |
| Angsana New             | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 5.01 |
| Cordia New              | github.com/pjobson/Microsoft-Fonts         | 80ab10a56664 | Version 5.01 |

The two GitHub repositories redistribute Microsoft's fonts. Only numeric metrics derived
from them are stored, as factual measurements rather than the font software; legal
reviewed this use (2026-10-15). No metric comes from fonts embedded in corpus documents. Not covered: Yu Mincho
(only a Windows 8 build was found).

### Hyphenation patterns

`src/hyph/` vendors two pattern files from the hyph-utf8 package
(github.com/hyphenation/tex-hyphen), unchanged and with their headers kept; licences are
listed in `src/hyph/LICENSE.md` and were covered by the same legal review.

| File             | Language                                       | Copyright                     | Licence               |
| ---------------- | ---------------------------------------------- | ----------------------------- | --------------------- |
| `hyph-en-us.tex` | English, US (also en-AU, en-CA, other English) | Gerard D.C. Kuiken            | All-permissive notice |
| `hyph-en-gb.tex` | English, GB (also en-ZA, en-IN, en-IE, en-NZ)  | Dominik Wujastyk, Graham Toal | MIT                   |

Other languages are not hyphenated. Automatic hyphenation follows the document's
`w:autoHyphenation` setting. The pattern approximation still needs Word fidelity
work: historical tests on 54 such documents had zero page-count improvements
and three regressions. Keep those cases in the unified-path regression set.

## Confirmed Word rules

Checked by per-document A/B against Word's saved page counts and page-break markers
(593-file set) or against page-1 EMF thumbnails:

- Widow/orphan control is on unless a style or paragraph sets `w:widowControl w:val="0"`
  (breaks on Word's page 41.2% with, 35.2% without).
- A page or column break that ends a paragraph keeps the paragraph mark on its line:
  the next page does not start with an empty line (+2.2 points page-count match).
- The body is pushed below a header (or above a footer) only when the page has a header
  (footer) part; the header distance alone does not move the body.
- HTML auto spacing (`beforeAutospacing`/`afterAutospacing`, 14 pt) collapses with the
  neighbouring paragraph's spacing: the gap is the larger of the two.
- HTML auto spacing before the document's first paragraph is dropped (a page-1
  thumbnail: title 17 pt high to in place, SSIM 0.55 to 0.94; at every page top it
  lost two A/B documents).
- Paragraphs on both sides of a floating table are neighbours for paragraph and
  contextual spacing: the table is out of the flow (a form's thumbnail: everything
  below a text-anchored floating table 4 pt high to in place).
- An empty paragraph closing a table cell right after a nested table takes no height
  (page-1 thumbnail: rows were 17.5 pt too tall).
- Custom tab stops beyond the right indent keep their text on the line (TOC page numbers).
- A font with OS/2 `USE_TYPO_METRICS` (Aptos, recent Google fonts) sizes its single line
  from the typo metrics, not the win metrics: median break drift 0.4 pt/page against
  16 pt/page for Aptos; for installed fonts 6 documents better, none worse.
- Floating tables anchored to the page or margin sit at their position (three page-1
  thumbnails; mean line delta 12.3 to 8.7 pt); text-anchored ones stay in the flow.
- Without `w:doNotUseHTMLParagraphAutoSpacing`, compatibility-mode-15 documents collapse
  paragraph spacing HTML-style: the gap is the larger of space after and space before
  (page count exact +2.7 points, 40 documents better, 8 worse; 7 of 8 page-1 thumbnails
  agree). Older modes and documents with the flag add them up.
- The paragraph mark's font does not size the last line of a paragraph in a table cell
  (9 documents better, 2 worse; a form's page-1 thumbnail matches within 2 pt).
- Footnotes take their full height at the bottom of the page of their reference
  (breaks on Word's page +10 points).
- Text in cells with `w:textDirection` btLr/tbRl runs along the rows' height and does not
  make the rows taller (5 documents better, none worse).
- A line holding only a page or column break stays on its page even when it overflows
  the bottom: no blank page follows (21 documents better, 6 worse).
- A table row whose paragraphs all keep with the next stays on the page of the next row
  (4 documents better, 1 worse; "any paragraph" lost: 5 better, 7 worse).
- A row split across pages keeps widow and orphan control in its cells (breaks per file
  +0.8 points, count neutral).
- On a document grid, lines in table cells snap to the grid only with
  `w:adjustLineHeightInTable` (3 documents better, none worse).
- Wrapped floating objects in headers and footers push the body text like objects in
  the body (a header logo's page-1 thumbnail: body 58 pt high to in place).
- Text-anchored floating tables (`vertAnchor="text"`) sit at their offset too (two
  page-1 thumbnails move 39 and 44 pt into place; A/B 2 better, 3 worse).
- Table rows grow by half of their top and bottom border widths, on top of a minimum
  row height (page-1 thumbnails of forms and calendars: SSIM +0.018, mean line delta
  1.31 to 1.12 pt, no per-row drift; A/B 8 better, 7 worse).
- `w:tblPrEx` overrides the table borders and cell margins of its row.
- Footnotes referenced in table cells (nested tables included) go to the page's footnote
  area and reserve their room before the row is placed.
- A nested table row taller than the room left splits between its text lines.
- Page fields in the body keep the result Word saved; headers and footers compute them.
- A table wider than its column keeps its left indent whatever its alignment (off-page
  text items on the A/B set 18 to 6).
- The document's final empty paragraph may overflow the page bottom by half its height
  (full set: 30 documents better, 5 worse; exact page count 76.8% to 77.4%).
- An empty paragraph that ends a section before a section starting on a new page stays
  on its page past the bottom: no blank page follows a full page (A/B: 593 set 13
  better, 1 worse, exact 77.2% to 78.5%; new 300 14 better, 1 worse, exact 74.0% to
  76.3%).
- A blocking (top-and-bottom or wide) object positioned from the page or margin above
  where its anchor paragraph starts also pushes the text placed earlier on that page
  below it: Word lays the page out again (a cover photo's page-1 thumbnail: titles
  384 pt high to in place; A/B page counts unchanged, overprinted text pairs 397 to 389).
- Floating-object anchors do not count as text on a line: a picture wider than the line
  after them stays on the first line (a full-page cover picture); a space does count.
- Paragraph-anchored objects are positioned from the paragraph's top, not from a line
  pushed below them. The top is above the paragraph's space before (A/B: page counts
  unchanged; 4 page-1 thumbnails closer, none further).
- Away from the top of a page, a table row whose text all fits but whose height (a
  minimum row height, margins or borders) does not moves to the next page whole; it does
  not split into an empty remainder (A/B: 593 set breaks per file 71.3% to 73.9%; new
  300 neutral). At the top of a page a row taller than the page still splits.
- `w:hideMark`: a cell holding only an empty paragraph takes no height; the row sizes
  to its other cells and its minimum height (an empty paragraph after text in such a
  cell still counts: dropping it moved a form's thumbnail lines 11.5 pt). A paragraph
  holding a space is not empty: its line keeps the row's height (three calendar
  thumbnails: mean line offset 12.8 / 11.7 / 6.8 pt to 0.5 / 0.4 / 0.5 pt).
- A page or column break inside a table cell does nothing: the cell's text stays on
  its line (a form's header bar in a page-1 thumbnail).
- The edge between two table rows grows the lower row by half of the wider of the
  upper row's bottom border and its own top border (new 300: per file 76.0% to 77.3%;
  thumbnails of ruled tables stop drifting 0.25 pt per row).
- Away from the top of a page, a row splits only when each cell that continues on the
  next page keeps at least two of its lines (A/B: 593 set 3 better, 2 worse, per file
  +0.9 points; new 300 2 better, none worse).
- A line holding only an inline picture shorter than the paragraph mark's line keeps the
  mark's height (VML horizontal rules, `o:hr`; A/B: per file +0.4 points on 593, +0.9 on
  new 300).
- A continuous section break into a different page size or orientation starts a new
  page (a landscape section after portrait pages).
- `beforeLines`/`afterLines` count in lines of the document grid when the section has a
  line grid (`lines`, `linesAndChars`), else in 12 pt lines: Word's saved `w:before`
  twips agree in 757 of 796 grid paragraphs and 663 of 718 others (A/B: 593 set 2
  better, none worse; new 300 1 better, 2 worse).
- In headers and footers too, a line holding only an inline picture is the picture's
  height, the paragraph mark's descent inside it (page-1 thumbnails of header logos: 8
  documents closer, 1 further, mean line delta 4.98 to 4.82 pt; A/B: 593 set 3 better,
  none worse; new 300 1 worse, a borderline page with no break truth).
- `w:autoSpaceDE`/`w:autoSpaceDN` (on by default): where an East Asian character meets a
  Latin letter or a digit, the earlier one is a quarter em wider (CJK thumbnail 9e6b:
  87% to 100% same line breaks; A/B 2 better, 1 worse).
- Justified lines in compatibility mode 15 fit when their spaces shrink to 0.776 of
  their width (page-1 thumbnails: Word accepts a line at 0.779 and refuses one at 0.774;
  same line breaks 93.3% to 95.2%, A/B 6 better, 5 worse). Older modes do not squeeze.
  A word is squeezed in only when the overflow is at most 0.4 of its own width: Word
  refuses to pull in a short word that would hang mostly past the margin (full set:
  line starts at Word's page breaks 89.8% to 91.2%, breaks on Word's page 61.6% to 62.3%).
- No line break after `/` before a Latin letter or digit: URLs and paths move whole
  (thumbnail line delta mean 3.6 to 2.2 pt; breaks per file +0.8 points).
- compressPunctuation in Japanese text (East Asian language `ja`): punctuation ending a
  line takes half its width and the punctuation inside it gives up to an em; Chinese
  keeps half an em in all (202 CJK documents: 6 better, none worse; page-1 CJK
  thumbnails are Chinese and stay at 95%).
- `w:softHyphen` is a break point that shows a hyphen only at a line end;
  `w:fitText` runs sharing an id take exactly the given width, spread evenly over their
  characters (175 documents: page counts unchanged).
- `PAGE`, `NUMPAGES` and `SECTIONPAGES` follow their `\* Format` switch (Arabic, roman,
  Roman, alphabetic, ALPHABETIC, ArabicDash, Ordinal, CardText, OrdText) over the
  section's `w:pgNumType` format; `cardinalText`/`ordinalText` lists spell English words.
- Runs that differ only in markup (revision ids, proofing marks, language) draw as one
  glyph run, so text items hold whole words (348-document sample: mid-word item splits
  6,142 to 1,606, items 195k to 161k).
- Text items follow reading order, not drawing order: headers, then body lines and table
  rows (cells left to right; a vertically merged cell's text with the row where the merge
  starts), each floating object after its anchor line, then notes and footers. Identical
  text items stacked at one position are reported once (348-document sample: word-order
  agreement 97.5% to 98.0%, documents under 95% from 46 to 21; pagination unchanged).
- The GB fonts FangSong (仿宋), KaiTi (楷体), SimHei (黑体) and DFKai-SB are monospaced:
  Latin characters take half an em. `_GB2312` names (仿宋_GB2312, 楷体_GB2312) use the
  same metrics (96 documents with break truth: count exact 63.5% to 67.7%, 6 better,
  3 worse).
- compressPunctuation compresses only justified lines (`both`, `distribute`): a
  left-aligned Japanese table keeps full-width punctuation (its page-1 thumbnail: 20% to
  100% same line breaks; 202 CJK documents: breaks on Word's page 59.0% to 61.3%).
- Single lines of Times New Roman, Arial, Calibri and Tahoma match GDI's
  `winAscent + winDescent + externalLeading` within the thumbnails' 0.75 pt resolution.
- A Symbol bullet label sizes its line with Symbol's own win metrics (2059 above and 450
  below the baseline, in 2048 units): in Calibri text the bullet lines are 16.5 pt apart
  at 1.08 spacing, against 15.8 pt for the lines without a label (page-1 thumbnails: 16
  documents closer, 5 further; A/B: 593 set 3 better, none worse; new 300 2 better, none
  worse).
- Beside text, a list label adds only its ascent (with line gap) above the text's: its
  descent and the line multiple stay the text's (Word for Mac probes: 13 and 20 pt Arial
  labels on 10 pt Arial at 1, 1.15 and 1.5 lines match to 0.1 pt; A/B: DOCX 0.8475 to
  0.8481, DOC 0.7911 to 0.7924). A label alone on a line sizes it like text.
- Symbol-font characters (bullets drawn through their Unicode equivalents) keep regular
  weight in bold runs: Symbol has no bold face (a bold paragraph mark's bullets in a
  page-1 thumbnail).
- Franklin Gothic Book, Gill Sans MT, Tw Cen MT and Century Schoolbook lay out with
  their own advances and line metrics (the 104 documents using them: page count exact
  64.4% to 79.8%, 19 better, none worse; line starts at Word's page breaks 59.1% to
  77.3%). So do Lucida Sans, Rockwell, Arial Rounded MT Bold, Agency FB and
  Franklin Gothic Demi (83 documents: page count exact 69.9% to 73.5%, 9 better, 1
  worse).
- The Thai fonts Angsana New and Cordia New (Windows 8 builds) lay out with their own
  advances and line metrics (60 documents: page count exact 40.0% to 41.7%, 1 better,
  none worse). Mangal and Calisto MT tables made 3 documents worse and are not used.
- A family drawn through a scaled substitute ("Sarabun*0.65" for TH SarabunPSK) takes
  the substitute's line height at the same scale: unscaled, 16 pt TH SarabunPSK lines
  were 29.7 pt apart (60 Thai documents: page count exact 41.7% to 65.0%, 20 better,
  none worse; the A/B sets hold one such document, unchanged).

## Confirmed drawing rules

Checked by per-document A/B against Word's page-1 thumbnails (69 documents,
`tools/vis_score.py`: colour SSIM, ΔE, worst tile, missing/extra ink, line positions):

- Automatic text colour is white on a dark cell, paragraph or text box fill and black
  otherwise; text in a styled shape takes the style's `a:fontRef` colour.
- Table styles apply every conditional region by `tblLook`: banded rows and columns
  (`tblStyleRowBandSize`), first/last row and column, then the corner cells, in that order.
- Section page borders (`w:pgBorders`) are drawn, measured from the page edge with
  `offsetFrom="page"` and outside the text otherwise; art borders are not drawn.
- A vertically merged cell aligns its content (`vAlign`) over all the rows it spans;
  when those rows sit on several pages, the content runs down the cell's part on each
  page from the top.
- Consecutive paragraphs with the same borders share one box that also covers the
  spacing between them.
- Shapes and groups turn by `rot` and flip by `flipH`/`flipV` about their centre; text in
  them turns but does not mirror, and `flipV` turns it a further half turn.
- Floating objects in front of the text are drawn after all of it, in `relativeHeight`
  order; objects behind it are drawn first, in the same order.
- Text boxes of inline shapes and groups are drawn; lines starting below the bottom of a
  text box that does not grow with its text (no `spAutoFit`) are hidden.
- Underlined or shaded tabs carry the underline and shading across their width
  (signature lines).
- Tabs at the end of a paragraph do not wrap to a new line.
- `wp14:sizeRelH`/`sizeRelV` sizes are shares of the page or margin of the section and
  replace the saved extent; text boxes that fit their text keep their saved height.
- Line numbers (`w:lnNumType`) sit right-aligned 0.25 in (or `w:distance`) left of the
  column, on each line's baseline, in the `LineNumber` style over the default font.
- Page numbers take their section's `pgNumType` format (`numberInDash` is `- 1 -`).
- An empty paragraph that only carries a section break shows no list label.
- Legacy check box fields are drawn in automatic colour without the run's shading.
- Shading patterns (`w:shd` `pctN`, `solid`, stripes) mix the pattern colour (automatic:
  black) over the fill (automatic: white): `pct5` over white is light gray.
- Picture recolouring (`a:grayscl`, `a:lum`, `a:duotone`, `a:clrChange`), `a:alphaModFix`
  and picture outlines (`pic:spPr/a:ln`) are drawn.
- A VML group has no outline of its own; its child shapes sit in its `coordsize` space.
- The page background (`w:background`) is not drawn, even with `displayBackgroundShape`:
  Word's thumbnail of a dark-background document is white.
- In table cells, a glyph drawn from a substitute face sits centred in the advance Word
  gives it, so narrower substitute capitals do not leave a gap after them.
- A list paragraph that opens with a page or column break shows its label on the line
  after the break, with its text; one holding only the break shows no label.
- Table of contents entries are drawn without the `Hyperlink` character style.
- A list style's own left indent (set by the style carrying `numPr` or a style based on
  it) replaces the level's indents; an indent it inherits gives way to the level's.
- A frame paragraph laid out in a header or footer whose frame sizes to its text and is
  aligned right or centred on the margin takes that alignment (page-number frames).
- Legacy drop-down form fields (`FORMDROPDOWN`) show their selected entry (`w:result`,
  else `w:default`, else the first), whatever result is saved.
- A supported single, whole-box `spAutoFit` shape grows vertically to its laid-out
  text and insets. Its background, text-box bounds and anchor exclusion share the
  resulting height, so following body text clears the grown box. This follows
  [Office's shape-autofit interpretation](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/19d31037-28bf-4693-85be-3a37bfe82d51).
  Explicit no-wrap uses the same longest-line layout for sizing and painting,
  preserving paragraph boundaries and hard breaks. The saved width remains intact;
  horizontal page-limit behavior is unresolved. Transformed boxes and text-fitting
  shapes retain their existing behavior. Saved heights taller than the laid-out
  text shrink when the reduction is at least one point. Historical broader growth moved two saved page counts away from Word;
  page-count and visible-content regressions require separate attribution.
  Authored overlapping foreground fills still cover background shapes. Font
  substitution can make those fills taller and hide neighboring text; an isolated
  declared-font replay resolves one observed case, while the frozen substitute-font
  result remains a visible regression in the evaluation report.
- The footnote separator (\`w:separator\`) is a 2 in rule, 0.75 pt, about mid-line.
- Struck spaces inside a line carry the strike (tracked deletions of a space); a
  struck space ending the line does not.
- Objects positioned from the left or right margin strip (\`leftMargin\`, \`rightMargin\`)
  sit in that strip measured from the page edge; headers and footers take the page
  geometry of the section that uses them.

## Drawing

DrawingML shapes, groups and text boxes (`wps:wsp`, `wpg:wgp`) are drawn with
`pptx-read`'s preset geometry and fill/line readers; text box contents are laid out in the
box. Floating pictures in headers, footers and cells are drawn; legacy check box fields
show a box glyph. Wrapping of text around floating objects is partial (see below).

Anchored pictures inside text boxes retain their source paragraph and line origins
until the containing page is accepted. Horizontal, vertical and fitted text projection
carry these objects alongside text; clipping an anchor's source line also hides its
object. Inline containers use a zero-sized, non-wrapping owner that is regenerated
when their paragraph is remeasured. Page and margin references use the actual page,
consistent with the [DrawingML anchor contract](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.drawing.wordprocessing.anchor).
Local placement follows the text-box transform; mixed global/local axes keep the
global reference independent. Page and margin alignment use the transformed
declared picture bounds, including numeric offsets without page clamping.
Rotated mixed-axis placement has source-invariant
coverage but no saved Word geometry oracle. This repair has synthetic picture
coverage; it does not establish support for nested rich-text boxes, which
[Word disallows](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/dc83bfa3-db21-4eb4-b3c0-63b11f2575d3).

Explicit DrawingML `bodyPr upright="1"` keeps the text frame upright after composing
its shape and ancestor-group placement. Shape paint retains its original transform;
text keeps the transformed frame center, source insets and vertical-writing setting.
The [primary upright contract](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.drawing.bodyproperties.upright)
defines orientation suppression. Exchanging frame dimensions in the 45–135° and
225–315° sectors follows [LibreOffice's recorded interoperability observation](https://cgit.freedesktop.org/libreoffice/core/commit/?id=7e23cbdbb6ec0247a29ed8a8f744c01e10963ea0),
which is secondary evidence rather than an exact Word geometry oracle.
Explicit false and omitted values preserve existing behavior. Theme body-property
inheritance and independent `bodyPr rot` remain outside this repair; the
[Word rotation exception](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/b3da041f-d56f-464b-a9b3-2e3274de9caf)
is version-qualified, so the current omission of that separate rotation is not a
claim about all current Word versions.

Rotated inline shapes and pictures use a separate layout rectangle: quarter-turn
sectors exchange its width and height, while the original paint, deferred text and
graphics move together into that rectangle. Source effect extents remain on their
declared sides. This follows the inline-specific interoperability rule in
[LibreOffice's importer](https://raw.githubusercontent.com/LibreOffice/core/c0df28f957b43f0ecbffdc994ee0702dbe5bc33f/writerfilter/source/dmapper/GraphicImport.cxx),
not an arbitrary-angle bounding-box fit or page-edge clamp. Anchored drawings retain
their source positioning. Older Word effect-extent compensation and general affine
group geometry remain separately qualified; saved Word references are needed to
establish exact placement across those variants.

- A family that is not installed, has no known substitute and no metrics table takes
  the `altName` Word recorded for it in `fontTable.xml` (`Serif` → Times New Roman on a
  page-1 thumbnail; 593 set 4 better, 2 worse). Mapping by font class lost its A/B.

## Historical evaluation notes

The following records describe earlier frozen builds and experiments. All environment
feature switches mentioned below have been removed; their names identify historical
comparisons and are not supported configuration. Current behavior is described above.

### Unconfirmed rules at the time of evaluation

Frame anchor defaults follow [Word's compatibility notes, MS-OE376 §2.1.48](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/6ff920ba-f035-407d-aa15-c4e2ad916cb6):
omitted `hAnchor` and `vAnchor` are `text`. The 593/new-300 default-mode scores
and all 158 thumbnail images are unchanged by this correction. The three-switch
frame experiment (`DOCX_FRAMES`, `DOCX_FRAME_MERGE`, `DOCX_FRAME_SPAN`) with these
defaults still regresses one page break; it remains off.

- Square-wrapped objects in headers and footers wrap body lines one by one like body
  objects, whatever their width: a wide logo reaching into the body's first lines no
  longer pushes the body below it (page-1 thumbnail: body 10 pt low to in place, SSIM
  0.611 to 0.785; page counts unchanged on the 593 and new 300 sets).
- Spaces and tabs after a line's last visible text do not size the line: an 18 pt
  trailing tab after 11 pt words leaves an 11 pt line (page-1 thumbnail: last line
  6.5 pt low to in place; A/B on 593 / new 300 / 158 thumbnails neutral).
- The paragraph mark's font sizes a paragraph's last line only when that line has no
  visible text (an empty paragraph, a picture-only line): Word for Mac probes in modes
  11-15 keep a 10 pt text line at 10 pt under a 20 pt mark. A/B: DOCX visual 0.835 to
  0.848, DOC 0.782 to 0.787.
- Lines wrap one by one beside square-, tight- and through-wrapped objects, keeping the
  objects' wrap distances; a line with no gap of 18 pt beside them moves below (A/B: 593
  set 5 better, none worse; new 300 1 better, none worse).
- Cell spacing (`w:tblCellSpacing`) takes its value once between rows and above the
  first row (1 better, none worse on the 12 A/B documents with it).
- Endnotes follow the body of the last section, numbered lower-case roman by default.
- Right-to-left tables (`w:bidiVisual`) mirror their cells.
- Text boxes with `wrap="none"` (VML `mso-wrap-style:none`) break lines only at paragraph
  ends and widen to their longest line (no thumbnail or page count changed).
- Experiments behind switches, off: footnotes continued on the next page
  (`DOCX_NOTE_CONT=1`, 1 better, 2 worse; on the fixed build 1/1 and 0/2), text frames
  as floating boxes (`DOCX_FRAMES=1`, 0 better, 1 worse), a negative fit tolerance
  (`DOCX_FIT_EPS`), paragraph-anchored objects moving their line to the next page
  (`DOCX_ANCHOR_FIT=1`, 1 better, 4 worse), objects behind the text keeping their
  wrapping (`DOCX_BEHIND_WRAP=1`: 2 better, 3 worse; thin rule shapes over-push), a
  split row's last cell line carrying its paragraph's space after (`DOCX_CELL_TAIL=1`:
  593 set 1 worse, new 300 1 better), floating tables with `tblOverlap="never"`
  moved below wrapped objects (`DOCX_FLOAT_CLEAR=1`), document defaults beating a
  table style under non-default paragraph styles (`DOCX_TBL_UNDER=1`), and for text
  frames: `w:framePr` attributes merged over the style's (`DOCX_FRAME_MERGE=1`), a
  margin default for `vAnchor` (`DOCX_FRAME_VMARGIN=1`) and auto-width frames as wide
  as their widest line (`DOCX_FRAME_SPAN=1`).
- The four frame switches together remain off after the 2026-10-16 A/B at
  `daa95cf379`: page counts unchanged; 593 set breaks 1 document better, 1 worse
  (per-file agreement 77.1% to 76.7%); new 300 unchanged. All 158 thumbnails:
  mean SSIM 0.7489 to 0.7495, with one cover improving 0.754 to 0.845. The break
  regression removes an empty frame paragraph from the flow and moves a heading
  onto the preceding page. Results: `/Volumes/T7/docx-eval/codex_frames_ab.log`.
- A between border (`w:pBdr/w:between`) joins paragraphs into one box even without
  other borders, and keeps its space above and below its line (a page-1 thumbnail:
  text after two such empty paragraphs 2.2 pt high to in place; page counts unchanged).
- The part of a split row left on a page ends below its last line and draws the cells'
  bottom borders (a form's page-1 thumbnail; 2 thumbnails closer, page counts
  unchanged).
- A cell's final line fits on a page only with the row's border extent below it; a row
  whose lines fit but whose bottom border does not leaves its last line for the next
  page (a form's page 1-2 Word render).
- Between list items the automatic space after is dropped, and the next item's explicit
  space before still applies (Word for Mac probes: 6 pt gaps between items).
- A row splits across pages only when every cell with content keeps a line (A/B: 593
  set 4 better, none worse; new 300 1 better, none worse).
- Lines on a document grid take whole pitches times the proportional spacing.
- A line on a document grid takes the whole pitches of the taller of its plain height
  and its tallest single font including the East Asian 1.3 extra (Word for Mac probes:
  13 and 14 pt SimSun take two 15.6 pt pitches, 12 pt one; 14 pt MS Mincho two 18 pt
  pitches, 13 pt one).
- Off the grid, a line holding East Asian text is the taller of its tallest East Asian
  run (1.3 em) and the plain height of all its runs; a Western font's deeper descent
  fits inside it (Word for Mac probes: Century beside MS Mincho at 10.5, 12 and 14 pt;
  A/B DOCX 0.8517 to 0.8534, DOC 0.7944 to 0.7951).
- In table cells, a table style's paragraph and run properties sit between the document
  defaults and the paragraph style chain, Normal included. Below compatibility mode 15 a
  12 pt size that comes from the default paragraph style yields to the table style's size
  (Word for Mac probes: mode 11-15 documents, Normal sizes 11/12/16 pt, styles based on
  Normal and with their own size).
- VML WordArt (`v:textpath`, text watermarks) is drawn stretched over its shape in the
  fill colour and opacity; VML shapes take named colours, `mso-position-*` alignment and
  `rotation`. No thumbnail in the evaluation set has a watermark.
- Underline styles: double, thick/heavy (twice the width), dotted, dashed and dash-dot
  patterns, words-only underlining and `w:u w:color`.
- CJK counting, full-width and circled numbers for list and page number formats.
- Border styles: double, triple and thin-thick borders draw their stripes; dotted, dashed
  and dash-dot borders their dashes. Run borders (`w:bdr`) box consecutive runs;
  diagonal cell borders (`tl2br`, `tr2bl`) are drawn.
- VML shapes take the DrawingML preset of their element or shape type (`#_x0000_tNN`);
  `v:line` runs from `from` to `to`; VML gradient fills are linear (`angle`, `focus`)
  and frame/tile fills draw their picture. Drawing canvases (`wpc:wpc`) are drawn.
- Charts, chartEx and SmartArt are drawn by `pptx-read` in a one-frame slide on the
  Word part; equations are written in Office's linear format.
- Shape and picture shadows are implemented but off (`DOCX_SHADOW=1`): the only page-1
  thumbnail with a shadowed picture shows no shadow.
- VML custom paths (`path`, `v:path`) are drawn; pictures inside VML groups sit at their
  group position. Picture fills and gradients move with the shapes they fill.
- Floating objects block text only with top-and-bottom wrapping or square wrapping over
  half the column; narrower square-wrapped objects are ignored (no text beside them).

### Side wrapping behind text

Side wrapping now applies to objects painted behind text. Set
`DOCX_BEHIND_SQUARE=0` to restore the earlier behavior. The initial rectangular
experiment had 17 improvements and 12 regressions; contour support recovered five
of those regressions. Top-and-bottom wrapping behind text remains experimental.

### Picture-only line leading

Unused automatic leading below a picture-only line may cross the page bottom
while retaining its normal flow height. Set `DOCX_PIC_AUTO=0` to restore the earlier
fit rule; `fit` explicitly enables the adopted behavior. It restores a cover's
lower logo strip and changes 12 pages to Word's saved 11.

The paired 7,454-document run improves 13 documents and regresses none. Exact
page counts rise from 5,953 to 5,963; absolute page-count error falls from 9,609
to 9,596; same-page breaks remain 9,391 / 13,935. Standard A/B improves three
files and new300 one. Only two of 166 thumbnails change: the restored cover and
a footer total-page field. Mean SSIM rounds to 0.7513; the cover's score slightly
falls despite its restored content. These scores use the preserved e3a68b24c20b
binary with the flag off/on and the recovered 312-file font snapshot, whose
baseline reproduces the historical full score rows and all thumbnails exactly.

`DOCX_PIC_AUTO=1` tests reducing picture-only line height throughout the document.
It improves seven A/B documents and one new300 document, but reduces mean
thumbnail SSIM from 0.7513 to 0.7444. It remains off: the visual evidence rejects
that general spacing rule. Default and page-boundary modes passed 90 tests before adoption.

### Comment content

With `annotation.enabled=true`, `index.json` includes `document.comments` for
legacy comment parts related to the main document. Each entry uses the shared
Text annotation keys: `subtype`, `contents`, author as `title`, and date as
`created`; `id`, `initials` and `sourcePart` preserve Word metadata. Missing ids
are `null`; unreferenced and empty comments remain present. Disabled annotation
extraction omits this document metadata. External relationships are ignored.
The [Word comments reference](https://learn.microsoft.com/en-us/office/open-xml/word/how-to-retrieve-comments-from-a-word-processing-document)
describes the comment part and its author, date, id and text.

Comment text preserves run text, paragraph and explicit line breaks, tabs,
cached field results, and inserted text; deleted and moved-from text are skipped.
Equations use the existing linear representation. Rich formatting, automatic
list labels, drawings, inherited visibility, modern thread/reply state and page
anchor mapping remain incomplete. Comments are document metadata; page selection
does not filter them, and no balloon geometry or Word visual fidelity is claimed.
Comment metadata does not participate in pagination or page rendering.

### Ruby content

Ruby base runs (`w:ruby/w:rubyBase`) retain their text and direct formatting,
including visibility rules. The structure follows Microsoft's
[rubyBase reference](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.rubybase).
`DOCX_RUBY=1` enables experimental horizontal phonetic annotations (`w:rt`).
The guide and base are shaped separately, aligned within their shared width,
and kept together during line breaking. Center, left, right, distribute-letter
and distribute-space alignment are implemented. Guide height contributes to
line metrics; both tiers retain their text in the extracted output.

The guide's `hps` overrides run size
([Microsoft hps reference](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.phoneticguidetextfontsize));
base run properties retain precedence over `hpsBaseText`. `hpsRaise` is read in
half-points, including Word's signed values
([implementation note](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/22f2ace9-7cdb-4742-a9ff-caf620a17baa)).
Interpreting raise as baseline displacement still needs full Word visual truth.
Vertical ruby, overhang into neighbouring text, mixed non-text ruby content,
and annotation-aware reading order remain incomplete. Unsupported groups retain
their ordinary base-text rendering. The experiment remains off by default.

Validated on 12 ruby documents (19 elements): 28 base characters restored, with
exact character-count deltas and unchanged pagination. Both standard pagination
sets are unchanged; all 166 thumbnails are byte-identical. None of those thumbnail
documents contains ruby, so they provide regression coverage only. The annotation
experiment also leaves both standard score tables and all 166 thumbnail images
unchanged; 80 tests pass with the experiment both off and on.
The full-page annotation A/B restores exactly 60 guide characters across the 19
groups, with no base-character loss. All 12 documents retain their page counts;
the nine Word-scored files retain their counts and saved-break scores. The local
`docx-read-ruby.html` gallery includes every affected page, with the missing Word
visual reference stated explicitly.

### Contour wrapping

Explicit distances on `wrapSquare`, `wrapTight`, `wrapThrough`, and
`wrapTopAndBottom` override the corresponding anchor distances independently.
An explicit zero removes that edge's gap. This follows ISO/IEC 29500-1
§20.4.2.3 (`anchor`, distance attributes), also published in
[ECMA-376 Part 1](https://ecma-international.org/publications-and-standards/standards/ecma-376/).
Parser and page-layout fixtures cover precedence, inheritance and zero values.
The 8,463-file corpus scan found no differing overrides in its top-level Word
XML parts; existing corpus scores cannot validate this feature's visual fidelity.

Stored DrawingML tight/through polygons now guide side wrapping, including
header/footer objects. `DOCX_CONTOUR_WRAP=0` restores rectangular wrapping.
Office uses a 21600×21600 coordinate space for these points
([Microsoft implementation clarification](https://learn.microsoft.com/en-us/answers/questions/405731/unit-for-coordinates-of-points-in-wrappolygon)).

The contour becomes conservative horizontal strips, normally at most 0.5 pt tall;
large objects use at most 4,096 regular subdivisions plus vertex boundaries.
Tight wrapping fills internal gaps; through wrapping retains even-odd interior
intervals. Rectangular contours collapse to one strip. Invalid/extreme contours
and quarter-turned objects retain rectangular wrapping. Arbitrary rotations,
VML contours and Word-reference validation of strip approximation remain open.

The complete 7,454-document paired evaluation uses the same binary and input
list with both switches disabled/enabled. All documents render without failure.
The combined behavior improves 17 documents and regresses 8: exact page-count
matches rise from 5,950 to 5,953; total absolute page error falls from 9,617 to
9,609; same-page break matches rise from 9,374 to 9,391. These are net gains,
with the eight regressions retained in the raw score and regression queue.
The regression set includes header objects, VML text boxes and a near-rectangular
through contour whose shorter exclusion moves two saved page breaks.

Both standard A/B sets have no pagination regressions (one break improvement on
the 591 scored documents; new 300 unchanged). Across 166 thumbnails, mean SSIM
is 0.7513 versus 0.7512; the overlap case improves 0.620 to 0.634. Contour-only
results were neutral on the small pagination sets. All 77 tests pass with both
switches enabled and disabled. Reference and run provenance live under
`/Volumes/T7/docx-eval/codex_contour_manifest.json`; full result tables are
`det_codex_contour_default_full.tsv` and `det_codex_contour_combined_full.tsv`.

### Section-aware endnotes

`DOCX_ENDNOTE_SECTIONS=1` enables experimental section-end placement and
section-specific numbering. A document-level `w:endnotePr/w:pos` of `sectEnd`
appends each section's notes before the next section; omitted placement or
`docEnd` keeps all notes at document end. Section-level `pos` is ignored, as
specified by the [EndnotePosition reference](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.endnoteposition).
Section `numFmt`, `numStart` and `numRestart` override document defaults.
`eachSect` restarts at the configured start; continuous numbering retains its
counter through format changes. Each destination area receives its separator.

The audit snapshot has five documents with explicit section-end placement;
three reference 107 endnotes before their final section. On all 50 endnote-reference
documents, the default output matches the adopted baseline byte-for-byte; all 166
default thumbnails also match. The experiment changes the three predicted documents:
one improves from 120 to Word's saved 119 pages, and two retain their page counts.
The 48 Word-scored documents have one improvement and no regressions; saved-break
scores are unchanged, but the three changed documents have no eligible break markers.
All 93 tests pass with the experiment off and on. Full-cohort and Word visual
validation remain outstanding before adoption. Endnote suppression,
continuation separators, custom marks, and detailed note-area layout remain
incomplete. The default behavior still places endnotes at document end and uses
document-wide numbering.

### Row-wide vertical cell margins (experiment)

`DOCX_ROW_MARGINS=1` applies the largest top and bottom cell margins in each row
when sizing and placing its cells. It remains off by default pending full-cohort
pagination validation. The four affected saved Word thumbnails improve by
0.048–0.053 SSIM; the other 162 are byte-identical. All 891 scored documents in
the two small comparison cohorts retain their page-count and break scores.
This is an empirical candidate; merged cells and exact-height rows need wider
validation before adoption.

### Nil vertical cell margins

The default ignores `w:tcMar` top/bottom exceptions whose type is
`nil`, retaining the inherited margin. Explicit `dxa` zero still clears a margin.
This follows Microsoft's [top-margin implementation note](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/8446a422-0fe8-401b-b11f-a2d4933ee83c)
and [bottom-margin note](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/8301b45d-ae90-45f1-8fca-f3e3bd7d9c49).
The frozen 7,454-document corpus contains 123 such entries. Horizontal margins
and table-level defaults are outside this correction. `DOCX_CELL_MARGIN_NIL=0`
restores the old numeric interpretation. Scanning all
Word XML parts in the frozen corpus identifies 13 affected documents. All 13
retain identical saved-page and break scores; one changes text placement on
pages 17–18. All 166 thumbnails remain byte-identical, and all 891 standard-cohort page-count
and break-score rows are unchanged.

### Header/footer frames (experiment)

`DOCX_HF_FRAMES=1` places non-inline, non-drop-cap frame groups on the following
ordinary paragraph in a header or footer. Both extent measurement and drawing
use that anchor; the following empty paragraph remains in flow. Groups without
such an anchor retain the previous layout. Body frames keep their separate switch.
A saved Word footer confirms the page-number baseline moves down about 13.8 pt
into alignment. Combined with row-wide cell margins, the experiment restores
one-page output for nine forms whose footer extent previously forced an extra
page. Broader pagination and thumbnail validation remain outstanding.

### Natural table outer borders (experimental)

`DOCX_TABLE_OUTER_BORDERS=1` reserves half the painted top border before a
natural table and half its bottom border after it. This is an empirical geometry
hypothesis from Word thumbnail edge measurements, not a confirmed specification
rule. It stays off by default pending paired visual and pagination validation.
Row heights remain distances between border centerlines; compound border stripes
and gaps determine the separate outer extent. Nested tables, header/footer tables,
page fitting, floating table bounds and column balancing include that extent.
Floating anchors position the painted outer box. Repeated headers repeat their
natural leading extent. Split rows retain the leading extent on the first piece
and trailing extent on the final piece; an outer edge alone creates no empty final
piece. Extra reservation for strokes at internal page cuts remains unverified and
is not added by this experiment.

### Vertical merge interior borders

Horizontal rules are suppressed by default within valid vertical merge
chains, matching the saved Word page-1 thumbnail of Japanese case
`237f89a99c20`. Its family/employment labels and transport cell acquired
extra `insideH` rules in the previous implementation. Merge membership requires a restart followed by
continuations with the same grid start and span. Explicit and inherited interior
borders are suppressed; the final bottom and neighboring cells retain theirs.

Original borders and row heights are preserved. Final pagination suppresses a
marked bottom only when the following source row is adjacent on the same page
and at the same column x position. Existing fragment-bottom painting remains at
page/column boundaries. Stacked nested tables suppress their logical interiors;
subsequent clipping of a nested-table slice does not synthesize a new closing
edge. Repeated headers and merges split across pages still need Word visual
validation. `DOCX_VMERGE_BORDERS=0` restores legacy interior-border painting for comparison.

On 186 frozen thumbnail cases, all 11 changed images improve SSIM and the other
175 are byte-identical; mean SSIM rises from 0.746135 to 0.746569. The paired
1,928-input check preserves every emitted page record byte-for-byte (19,676
pages, including text geometry), all per-document pagination scores, and six
pre-existing broken-file errors. This covers the standard A/B cohorts and 942
new inputs, not the entire historical 7,454-file benchmark. All 110 tests pass
with the candidate enabled and disabled. Full Word exports remain needed to
verify fragment-edge semantics beyond the preserved prior behavior.

### Exact row height and bottom padding

Positive exact row heights include the largest resolved bottom cell padding in
the row. `DOCX_EXACT_ROW_PADDING=0` restores the previous calculation. Automatic
and minimum row heights retain their existing calculation.

[MS-OI29500, section 17.4.80](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/5919e0bd-e6ce-477e-8d66-0e5282f5c506)
documents this Word-specific interpretation of `w:trHeight`. The discrepancy
accumulates over successive exact rows even when their content fits. Resolved
padding includes table defaults and cell exceptions, including the existing
nil-versus-explicit-zero behavior. Tests check the maximum across cells, avoid
adding top padding, and keep minimum heights unchanged.

Across 271 paired thumbnails, one improves and 270 are byte-identical. The
1,928-input pagination gate preserves page counts; one saved-break match is
lost because a repeated header interrupts its text, with all text preserved.
A further 129 component and recovered-preview inputs have byte-identical full
output. The 28 affected character-grid/exact-row cases reproduce their isolated
results when combined. Native and converted legacy scores remain separate.

### Empty no-number label suffix (experiment)

`DOCX_NONE_SUFFIX=1` suppresses the label and its suffix when the resolved
numbering format is `none` and its label text is empty. Level paragraph
properties, direct formatting and numbering counter updates are preserved.
Literal labels and empty labels in other numbering formats retain their behavior.

Microsoft's [NumberFormatValues reference](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.numberformatvalues)
defines `none` as no numbering. The suffix behavior is supported by the saved
Word thumbnail of `c34c4ae2032d`: its empty heading label has an implicit tab,
but its text starts at the paragraph's explicit zero indent. The prior renderer
inserted the tab and shifted only the first line by 36 points. The experiment
remains off pending paired corpus validation.

The adopted explicit-empty suffix rule requires
`numFmt=none` and an explicit empty `lvlText/@val`. The rule preserves suffixes for
placeholder patterns such as `%1`, omitted text values, and null-character
patterns. The original broad flag retains its behavior.

The [LevelText definition](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.leveltext)
distinguishes literal text from placeholders. Its omitted-element semantics do
not establish a suffix exception; the adopted rule therefore requires explicit
source evidence. Set `DOCX_EMPTY_LEVEL_SUFFIX=0` to opt out. The broad
`DOCX_NONE_SUFFIX` experiment remains off by default. The [LevelSuffix definition](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.levelsuffix)
defaults an omitted suffix to a tab. The saved thumbnail supports suppressing
that tab for the explicit-empty case; placeholder-derived empty labels need
separate Word validation before changing their behavior.

### Character-grid tracking

The renderer applies signed `w:docGrid/@charSpace` tracking to full-width
East Asian clusters in `linesAndChars` sections. Run-level `w:snapToGrid` inherits
through styles and defaults to enabled; its exemption is separate from paragraph
line-grid snapping. The active character grid follows each section, including
its body text boxes and tables. Glyph contours and font sizes remain unchanged.

The initial scope includes shared symbols selected by an East Asian hint and
excludes ASCII, half-width forms, and standalone combining marks. Mixed-script
unit alignment and `snapToChars` remain unsupported. Word's
[specific grid-type rules](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/ad4d9201-2051-4b7d-8f9c-ba019f0ae7a6)
require distinct handling of those modes. The signed value is divided by4096,
following the [character-pitch definition](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.docgrid.characterspace?view=openxml-3.0.1).

The motivating saved Japanese textbox has9pt runs,167.1pt available width and
charSpace1276. Additive tracking predicts Word's17-character first two lines;
the prior renderer fits18. The frozen1928-input cohort contains22character-grid
documents,14with nonzerocharSpace; only one of186thumbnail references covers
this feature. Tracking is enabled by default; `DOCX_CHAR_GRID=0` restores legacy
spacing. The paired gate had one strict page-count improvement, no strict
regressions, and one thumbnail improvement with 185 identical images. A separate
71-input native OOXML cohort preserved all 47 strict reference scores. One input
without Application provenance moved from four to five pages against saved
Pages=1; its saved break markers align better, but its reference remains uncertain.
Font substitution still affects checkbox outlines and glyph widths.

### Floating table side gaps (experiment)

`DOCX_TABLE_SIDE_WRAP=1` sends floating table rectangles to the existing
line-by-line wrapper, which measures the remaining gap. The default keeps the
older rule that treats a table at least half the column width as a full-width
exclusion band. `DOCX_WRAP=paragraph` keeps that older classification in either
mode. Full-width tables still push subsequent text below them.

A legacy document converted to DOCX has a 257.4 pt floating table followed by
nine empty paragraphs. Both its original saved Word preview and LibreOffice's
converted-DOCX rendering place the next heading just below the table. The
half-width rule pushes the empty paragraphs below the table first, adding
120.55 pt of blank space. Measuring the available side gap removes that space.
The original/converted source bytes and independent rendering evidence live in
`legacy-doc-ingestion-20261016/lo-paired-preview` under the evaluation directory.
The experiment remains off pending the paired pagination and thumbnail gates;
LibreOffice's default-font diagnostic lane is separate from native font scores.

### Explicit page-relative body frames

Body paragraph frames with a resolved `vAnchor="page"` and explicit numeric `y`
float by default; `DOCX_PAGE_FRAMES=0` restores legacy flow. Consecutive paragraphs with the same resolved frame
share one box. Frame attributes inherit individually through styles; the scoped mode
keeps that resolved copy separate from legacy frame behavior. Headers, footers,
table-cell paragraphs, drop caps, and text-relative frames retain their existing
layout. The broader `DOCX_FRAMES=1` mode remains a separate experiment.

Word defaults an omitted vertical anchor to text, per
[MS-OE376 2.1.48](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/6ff920ba-f035-407d-aa15-c4e2ad916cb6).
The scoped mode does not reinterpret text-relative positioning as page positioning.
A saved cover preview places logos and a disclaimer on page one; legacy flow puts
them on page two. Broad frame mode restores them but overlaps a text-relative subtitle.
The scoped candidate restores that content on the original unmodified source,
improves saved-thumbnail SSIM from 0.7543 to 0.8100, and changes 461 pages to 460
against Word's saved 426. The first corpus gate found two content-loss cases despite
unchanged page counts and improved cover similarity, and was not adopted.
Repairs retain injected frames during paragraph reflow, anchor trailing groups even
without ordinary body text, and defer nested drawings until the frame's page is known.
Page/margin-relative children retain page coordinates; paragraph/line-relative
children use their local origins inside the frame. Column alignment uses frame width.
These reference types follow the [DrawingML positioning definitions](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.drawing.wordprocessing.verticalrelativepositionvalues?view=openxml-3.0.1);
nested Word placement still needs visual evidence. The repaired candidate passes
271 paired previews (one improvement, 270 identical) and 2,056 source-ID comparisons
(2,055 distinct source hashes; 2,057 cohort memberships). All default controls match
the prior adopted renderer. Four changed documents preserve all normalized text
items and characters, with no page-count or saved-break-score changes. Six existing
error records are unchanged. Adopted commit `20fbf397b1` uses pinned Rust 1.89; fresh
flag-free renders match all 271 candidate previews, 2,056 full-document outputs,
and the separate motivating document byte-for-byte. Both default and opt-out pass
135 tests. Text-relative frames and nested-object wrapping remain incomplete.

### Experimental Mac font metric profile

`DOCX_METRIC_PROFILE=mac` explicitly selects an experimental line-metric profile;
absence or any other value preserves the default Windows-oriented behavior. No
application metadata, `usePrinterMetrics`, or `noLeading` flag enables it. An
existing `DOCX_FONT_FIT` override still takes precedence.

The profile reads hhea ascent, descent, and nonnegative line gap from the actual
requested installed or embedded face, preserving the requested bold/italic
selection. When a substitute draws the glyphs, preserved macOS metrics cover
Helvetica and Palatino's regular, bold, italic, and bold-italic faces. Unknown
substituted families retain the default calculation. Glyph selection, widths,
and font-loading policy remain unchanged.

The preserved values were measured from macOS `Helvetica.ttc` and `Palatino.ttc`
(all four principal faces, 2048 units/em): Helvetica `(1577, -471, 0)` and Palatino
`(1685, -568, 0)` for hhea ascent/descent/gap. The separate style slots currently
share values. Only numeric metrics are included; no font data is bundled.
Microsoft's [hhea reference](https://learn.microsoft.com/en-us/typography/opentype/spec/hhea)
describes these fields as Apple-specific; it does not establish a Word
compatibility-flag mapping or guarantee identical layout across renderers.

Frozen-font configuration probes reduced one converted Mac document from twelve
to eight pages and improved its saved-preview score. Another remained three
pages and scored slightly worse. The profile remains off pending broad paired
validation; tiny legacy previews and conversion effects limit visual claims.

### Graphics nested inside shape text

Deferred chart and SmartArt processing runs by default inside
shape text-box blocks, including nested tables. Detection and materialization
traverse the same block tree, so a document containing only nested graphics
still runs the graphics pass. Shared shape data uses copy-on-write mutation.

A frozen source contains an inline chart with four cached series inside a text
box. The previous pass skipped the chart; moving the same drawing to a body
paragraph rendered its content. Synthetic nested-only and mixed direct/nested
chart tests cover pass detection, rendered text, and vector output. Grouped
`graphicFrame` dispatch remains a separate gap. Graphics warnings now propagate
to the index through the shared renderer. `DOCX_NESTED_GRAPHICS=0` restores
the previous traversal.

The paired gate preserves all 271 first-page images and 2,055 of 2,056 full
document outputs. The remaining document gains its nested chart on page four:
67 text items are added, with all previous text, order, geometry and styling
preserved. Page counts and existing errors are unchanged. Component ablation
isolates the restored chart; a paired Word export is still unavailable.

### Drop-cap paragraph pairs (experimental)

`DOCX_DROP_CAPS=1` places a text-only drop-cap frame beside its immediately
following ordinary left-to-right body paragraph. It preserves the saved run
size and baseline shift, anchors the cap to the following paragraph's first
line, and derives its exclusion height from the requested number of wrapped
lines. Mixed run heights are measured again until the span stabilizes; an
unstable pair retains its original flow. `drop` excludes text inside the margin;
`margin` places the initial outside it. Generated initial text reads before the
following paragraph and survives its reflow.

[ECMA-376 Part 1, fifth edition](https://ecma-international.org/wp-content/uploads/ECMA-376-1_5th_edition_december_2016.zip),
§17.3.1.17 (printed pages 210–214), specifies the following non-frame paragraph as
the anchor, `lines=1` when omitted, and zero omitted `hSpace`. Explicit line count
takes precedence over other vertical positioning. Horizontal spacing applies to
`wrap=around`. The parser retains drop/margin mode and line count, including
frame-property inheritance.

This first experiment supports adjacent paragraph pairs. Intervening frames,
right-to-left paragraphs, table cells, headers/footers, multi-line cap content,
and orphan cap frames retain their original flow. A short anchor paragraph uses
its last line metrics for the remaining requested lines. Font scaling, exact
Word glyph alignment, external-object interactions, and drop-cap spans crossing
page/column boundaries remain unvalidated. The sole audited native drop-cap
source has saved page-count metadata but no saved Word thumbnail; page-count
agreement does not establish visual fidelity. The experiment stays off by default.

### Grouped graphics

Grouped graphics retain chart, chartEx and diagram graphic frames inside
DrawingML groups until fonts are available, then materializes them through the
shared renderer. `DOCX_GROUPED_GRAPHICS=0` restores the legacy omission. Pending records preserve source-part
relationships, insertion order, mapped child coordinates and the accumulated
child/ancestor rotations and flips. Missing references use the existing graphics
diagnostic path. Groups containing only deferred graphics remain reachable.

Microsoft's [GraphicFrame schema](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.office2010.word.drawinggroup.graphicframe?view=openxml-3.0.1)
specifies a direct `wpg:xfrm` child and `a:graphic` content. Tests cover that form,
nonuniform child coordinates, composed rotations/flips, adjacent chart paint
order, repeat materialization and missing-reference diagnostics. Generic
unsupported graphicData is unchanged. Full scene transforms also transform chart
text; this has no paired Word visual validation yet. The paired 271-preview and
2,056-document gates preserve default output and page counts; only the known
four-chart source gains content, with every original text item retaining its
order, geometry and style. Individual component ablations isolate each restored
chart. A scan of 9,511 native packages found no other active grouped graphic
frames, so synthetic tests cover rotation/flip diversity beyond the corpus.

### Continuous-column balancing experiment

`DOCX_COLUMN_BALANCE=1` enables a default-off repair for continuous section
transitions. Paragraph and row fitting reserve page-wide footnotes before applying
the trial column-height limit. Each trial restores the starting page's note and
continuation metadata, including notes from earlier sections. Acceptance checks
the actual normal-flow cursor across all columns, including forced oversized
content. Continuous fallback also uses that maximum cursor. Floating-table
cursors remain outside this normal-flow extent; anchor-fit checks retain the
physical page boundary. Final note metadata is synchronized before notes are drawn,
so references introduced by later continuous sections retain their note text.

Synthetic tests cover tall lines and rows, pending notes, repeated failed-trial
rollback, floating-table flow, and column/page resets. Corpus and visual gates
are required before adoption. The motivating saved two-page source has no Word
thumbnail, so removal of its native overlap does not establish Word visual
fidelity. `DOCX_BALANCE=0` still disables balancing; with the repair enabled, the
continuous transition retains the maximum occupied normal-flow extent.

### Continuous-section footnote synchronization

The final footnote metadata refresh is enabled by default.
`DOCX_CONTINUOUS_NOTES=0` retains the legacy behavior unless the column-balance
experiment is enabled, which already requires this refresh.
Later continuous sections can add notes to a page whose output record was
created earlier. Before drawing notes, copy the final ordered note IDs and
continuation cut/carry state into that record, preserving its original section,
page number, header selection and body geometry. This happens after pagination
and cannot itself change the page count.

A single-column, two-section fixture isolates the content defect without column
balancing: the later note is restored on the same page while body and header
positions remain unchanged. A separate invariant test covers continuation state
updates/clearing and retention of original page settings. With the existing
`DOCX_NOTE_CONT=1` continuation flag, a two-page fixture verifies all twelve
note fragments render exactly once and the cut/carry state agrees. Independent
271-thumbnail and 2,056-document gates show no page-count or error changes. Only
one document gains its missing note, preserving every original text item’s
order, geometry and style. The broader column geometry repair retains its
separate flag and evaluation.

### Leading column-transition extent experiment

`DOCX_COLUMN_TRANSITION_FLOW=1` is a separate default-off extension to
`DOCX_COLUMN_BALANCE=1`; alone it has no effect. A paragraph's leading column-break
control line contributes its top position to occupied column flow, preserving
space before and top padding while omitting the unpainted line height. Existing
line placement and spacing after the paragraph remain unchanged. The balance
trial estimate uses the same extent rule.

Eligibility requires an empty first line ending in a column break, followed by
actual paragraph text. Lines with text (including spaces), notes, anchors, inline
boxes, or painted items retain their full extent, as do paragraphs with frame
anchors or decoration. Standalone breaks, real blank paragraphs, mid-paragraph
breaks, and picture/shape-only following content retain existing behavior.

The motivating native heading and converted three-column form both charge a
blank transition line using the following text's font metrics. OOXML specifies
restart in the next column, but does not establish the exact consumed blank-line
height; this remains an empirical experiment pending paired corpus/visual gates.

### Leading table-cell paragraph page breaks

Table layout preserves a resolved
`pageBreakBefore` on the first direct paragraph of a row's first cell. It starts
that ordinary body-table row on a new page when flow already occupies the page.
At page top it adds no blank page. Existing repeated-header and split-row handling
continues after the boundary. Later cells, interior paragraphs, nested tables,
header/footer tables, floating tables and vertical-merge continuations retain
existing behavior; their break semantics need separate reference evidence.

The [Open XML paragraph property](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.pagebreakbefore)
requires the paragraph to start on a new page and takes precedence over keep-next.
Saved Word cover-page evidence confirms that a leading first-cell paragraph can
move its table off the cover. This evidence does not establish all within-row or
repeated-header behavior. Tests cover the component boundary, page-top handling,
content preservation and continued header repetition. `DOCX_TABLE_PAGE_BREAK=0`
restores the earlier behavior that ignored these table paragraph boundaries.

Validation covers 271 historical previews, 2,056 historical pagination IDs,
741 additional native files and 283 legacy conversions, kept as separate cohorts.
The saved cover is corrected and an independent valid Word page-two marker moves
to the expected page. All changed documents preserve original text items; extra
pages repeat their existing footers. One raw saved-count error worsens from
27 to 36 against a saved count of 1. Source inspection finds 23 explicit boundaries;
a secondary LibreOffice render also produces 36 pages, with all 20 table-start
headings on matching pages. Removing those properties reproduces the earlier
native output. The saved count and raw worsening remain in the report.
LibreOffice ignores the three interior-row properties in that source, so it is
not a reference for those boundaries; the separate valid Word marker supports
interior-row handling. The signed-grid composition check matches all 20 affected
grid inputs, six changed table inputs and 23 incremental previews.

### Unsigned spelling of signed character-grid spacing

Character-grid parsing interprets integer `docGrid/@charSpace` values in
2147483648–4294967295 as signed 32-bit values before dividing by 4096.
The Microsoft Open XML API defines `CharacterSpace` as `Int32Value`:
<https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.docgrid.characterspace>.
This compatibility recovery handles out-of-range unsigned spellings;
`DOCX_GRID_SIGNED32=0` restores the prior parsing. Valid signed values and values
outside that unsigned interval retain their existing parsing behavior.

An input-only control changing 4294964838 to its signed equivalent, -2458,
reduces a converted document from 996 pages to 38 with the same renderer and
fonts. The original cached one-page count is not a verified layout reference.
All 20 affected packages reproduce signed-input controls exactly. The 15 active
grids improve page counts; five inactive-grid controls remain unchanged. The
271 historical thumbnails and 2,056 full outputs remain byte-identical. Independent
source attribution finds no removed body characters and 74 recovered characters
in the eight cases with changed text streams; table reading order can change.
These checks establish compatibility recovery, with exact Word layout unverified.

### Vertical-merge fragment flow (experimental)

`DOCX_VMERGE_FLOW=1` lets eligible top-aligned vertical-merge content use the
existing moved-slice continuation path. Physical fragments distinguish page,
column and discontinuous row positions. Source cell margins and paragraph
spacing travel with the content; text and ordinary inline drawing items remain
in their slices. The switch is off by default while corpus checks are pending.
This is a content-preservation repair, with no claim of exact Word positioning.

Top-aligned merges in nested or header/footer tables, or containing floating
anchors, footnotes or vertical text, keep
their existing placement owners. Nested table slices need continuation ownership
before content can move to a later inner row; the current guard preserves their
existing output. Their fragment-aware continuation remains an
uncovered case. Center/bottom merges already used moved slices; the experiment
also applies fragment-boundary detection and source-margin preservation to them.
Rows in the actual leading repeated-header prefix keep an independent
merged-content copy at every placement. New top-flow promotion also leaves
merges crossing that prefix into body rows with their original owner; mixed
repeated/non-repeated continuation remains uncovered. Single-fragment geometry, cross-page/column text, and the conservative
anchor/note fallback have synthetic coverage. Column balancing and transition
flow remain independent, default-off experiments.

Relocated merge items paint after their final owning row's cell shading and
before its perimeter strokes. The background cannot hide continuation text or
inline drawings, and borders retain their paint precedence. Synthetic tests
check the actual raster colors of a shaded continuation and a border overlap. The switch
remains off pending corpus validation. Eligible terminal merge rows now allocate
whole source slices during pagination, retaining per-fragment margins and unused
line-fitting space. Each physical fragment owns an immutable paint assignment;
following flow starts after the allocated row extent. Progressing cells may span
more than 200 pages. Synthetic tests cover efficient partial-page use, bounded
text and long continuation streams. Exact-height terminal rows and merged slices
with anchors or notes retain their previous placement path; oversized indivisible
slices retain the pager's existing forced-placement policy.

### Experimental bidirectional line layout

`DOCX_BIDI_LAYOUT=1` enables paragraph-level Unicode bidi analysis and visual
cluster placement after wrapping. Logical text and shaped cluster internals remain
in source order. The experiment also retains inherited `w:rtl`, selects complex
script formatting, and applies Word's weak/neutral character overrides. Explicit
symbol encodings retain their symbol-family Unicode translation; shaping offsets
map back to source bytes even when that translation changes UTF-8 length.

The implementation uses the existing MIT/Apache-2.0 `unicode-bidi` dependency.
[Unicode UAX #9](https://www.unicode.org/reports/tr9/) defines paragraph analysis,
line whitespace reset and visual ordering. Microsoft's
[`rtl` reference](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.righttolefttext?view=openxml-3.0.1)
describes complex-script formatting and character overrides; its
[Word implementation note](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/a3ba403b-9e64-418e-9d3e-9bdba70fcbb0)
records paragraph `bidi` as the line-direction input for the listed Word versions.

This remains off by default. The 195-test default/enabled matrices cover mixed
scripts, numbers, punctuation, caps expansion, style boundaries, joining, controls,
mirroring, decorations and symbol byte offsets. Real-reference and broad corpus
validation remain required; RTL numbering placement and tab behavior need further
coverage. Font substitution and blank-paragraph pagination are separate fidelity
limits, so improved direction does not establish a Word page match.

Within the bidi experiment, trailing tabs in an RTL paragraph participate in the
line-end whitespace reset and are excluded from the visible alignment edge.
This preserves the last line's ink positions when trailing tabs are added. Two
regressions cover cluster origins and a wrapped Arabic paragraph with a footnote
marker; interior tab stops and RTL list-label positioning remain separate gaps.

### Wrapped pictures and column continuations

Ordinary wrapped pictures with nonnegative paragraph- or line-relative
vertical offsets travel with their anchor when their declared extent crosses the
physical page. Footnotes reserve a stricter lower boundary; pictures can occupy
the bottom margin when no notes are reserved. Later paragraph and table references
also check pictures already placed on the page. Fixed references, negative offsets,
behind-text objects without side wrapping, rotated shapes, text frames and
intrinsically oversized pictures retain their separate placement policies. An impossible first placement
on a fresh physical page retains the progress fallback.

Column retries preserve ordinary column-local placement. A column-relative picture
that fits the physical page from the first column but crosses its edge from a later
column advances with its anchor. Square-wrap exclusions persist across the page
and are filtered for each destination column. A continuation entering or leaving
those exclusions retains its consumed source prefix and remeasures its remaining
lines, including freshly calculated first-line clearance.

Microsoft's [shapeLayoutLikeWW8 implementation note](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/91215a74-39b8-4896-8735-f99cf8fb13cb)
illustrates wrapped-object and anchor movement to the next page, with a legacy
exception for negative offsets. That note applies to Word 2007 and does not
establish a universal physical-page boundary for current Word versions. Authored
geometry controls and actual-document raster review accompany corpus comparisons.

Tests cover existing, pending and later footnotes, table references, column-local
and spanning pictures, state rollback, oversized-note progress, exact continuation
text order, destination clearance and departure to a page without exclusions.
Injected carry-state coverage is separate from source-driven note continuation.

Behind-text basic decoded pictures with square, tight or through side wrapping use
the same page and footnote-area clearance as foreground pictures, while retaining
their background paint order. This includes supported EMF pictures decoded into
image content; it is not restricted to raster source formats. Clearance follows
the picture's horizontal intersection with each note area, so a disjoint column
can retain the picture while a spanning picture may require another page.
Native charts, grouped graphics and wrap-none objects keep their existing policies.

The current candidate passes 277 tests. Relative to the note-area checkpoint,
293 of 294 thumbnails and 3,078 of 3,080 full outputs are byte-identical, with the
same six invalid inputs. The changed thumbnail's first-page content agrees with
the saved Word preview; the other changed document moves from four to five pages,
matching its saved count. Complete changed-page audits remain pending; these
results do not establish general Word-equivalent object placement.

### Space before at page and column tops

Word for Mac probe renders (modes 11, 12, 14 and 15): a body paragraph reaching a
page or column top by flow loses its space before, explicit or automatic. After a
page break character it loses it from mode 14, and on `pageBreakBefore` from mode 15. The document's first paragraph and a section's first paragraph keep explicit
spacing, as does a paragraph after a column break. On a section's new page, the first
paragraph keeps only the part of its space before that exceeds the previous paragraph's
space after (modes 11 and 15: 48 pt before after 20 pt after gives 28 pt; A/B DOCX
0.8511 to 0.8517, DOC 0.7927 to 0.7944). Probe DOCX files at modes 11 and
12 also drop spacing after a page break, but Word 97-2003 `.doc` files (converted at
mode 11) keep it, so legacy modes keep it. First-cell paragraphs of a table keep
their spacing. A/B against Word renders: DOCX visual 0.818 to 0.834, DOC 0.777 to
0.782. Automatic spacing at the document's first paragraph is dropped. A pending first paragraph of a section retains automatic
spacing across a forced retry. The paginator tracks pending section content
separately from header/footer first-page selection, including odd/even padding and
continuous sections, and clears that state when a line or row is placed.

Authored LibreOffice controls distinguish a continuous section's first paragraph
requesting a page break, which retains spacing, from automatically overflowing,
which suppresses it in modes 12, 14 and 15. The existing automatic-overflow branch
is unchanged. The controls preserve preceding text and measure target-page geometry;
they provide interoperability evidence rather than generic Word visual truth.
The focused real-source case recovers its saved six-page count from seven pages
while preserving all normalized text in order. Corpus results remain separate
from these focused controls in the evaluation reports.

### Terminal paragraph spacing in split table cells

An ordinary cell's final paragraph carries its after-spacing with its last line
when the row splits. Its painted lines can fit the remaining page space while
that last line and spacing together require a continuation. Cell metadata keeps
the spacing on the fragment containing the final line and includes it in that
fragment's content height. Minimum row height retains its separate fitting rule.

The shared line slices used by notes and text frames keep their existing heights.
Vertical merges and rotated cells retain their existing fragment allocation;
this rule currently covers ordinary cells. Tests distinguish empty and nonempty
terminal paragraphs, minimum height, repeated splitting and note ownership.

### Horizontal visibility of paragraph-frame lines

After a legacy paragraph frame's width is resolved, a plain text line wholly
outside that width is omitted from its painted items. Both advance bounds and
actual glyph outlines must establish that the whole line is outside. A line
crossing the frame boundary retains all its text. Its height and paragraph spacing
remain part of layout even when its painted items are omitted.

Unknown outlines, non-finite positions, transformed or synthetic text, decorations,
anchors, notes and non-text objects retain their existing behavior. Ordinary body
paragraphs and nested table rows keep their separate layout. Original saved cover
evidence and authored LibreOffice controls support this behavior; generic Word
boundary geometry remains unverified.

### Accepted footnote ownership and geometry

Section footnote placement is retained as a typed source property. Word's documented
`sectEnd` behavior maps to `beneathText`; unsupported `docEnd` remains distinguishable.
Document-level placement does not override section placement.

For a single-column page with one note-owning section, beneath-text placement uses
the final accepted body extent, excluding terminal paragraph after-spacing. The
complete note area clears intersecting foreground wrap exclusions. Separator and
continuation content move with the accepted area; note allocation and pagination
reservations stay unchanged. Structured continuation retains its source owner.
Note-free single-column sections on the same page contribute to the body extent.
Authored exact separator line heights remain in effect. A short line can paint
above its allocated box and touch preceding body or picture ink; ordinary
automatic-height separators retain their full line metrics.

Multiple body columns, mixed note owners and unfragmented carry with uncertain
ownership retain bottom alignment with a diagnostic. An area that cannot clear
the accepted body exclusions within its capacity also retains bottom alignment.

Accepted placements record note-reference ownership by source section and body
column after balancing, before drawing consumes text items. Body extents and
foreground wrap exclusions remain separate from header/footer ownership. Source
body columns do not determine footnote-area columns: `w15:footnoteColumns` can
specify a different count. Mixed-section arbitration, custom note columns and
exact Word clearance in complex beneath-text layouts remain coverage gaps.

### Footnote state after pagination retries

A pagination snapshot includes the surviving page's complete metadata. Restoring
a rejected keep-chain or floating-object trial restores that page along with the
placement cursor and removes pages created by the trial. Accepted note references,
fragment metadata and captured section geometry return to their snapshot values.
A kept paragraph that moves to another page therefore emits its footnote only on
the accepted page, without leaving a note reservation on the rejected page.

### Footnote areas in equal-width columns

Equal-width body columns use a note area containing each note's first accepted
reference. The section's `w15:footnoteColumns` selects the displayed column count
when absent or zero; an explicit matching count uses the same layout. Measurement
and drawing share captured width and character-grid context, including separator
slices. Global reference order and repeated-ID ownership survive area grouping,
pagination retries and continuous sections. Picture clearance uses intersecting
area rectangles, so a disjoint column's notes do not consume its capacity.

Custom counts, unequal columns and incompatible active-note geometry across a
continuous chain retain a shared legacy area with an unsupported-policy diagnostic.
Resized sections begin a separate chain; note-free preludes do not force fallback.
Beneath-text placement, long-note fragmentation and general mixed-area allocation
remain incomplete. Existing carried fragments are retained with an owner-context
diagnostic. Intersections with accepted body flow bounds are reported for visual triage;
these bounds include empty lines and table extents, not only glyph ink.

[MS-DOCX defines column-count selection](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-docx/3b729d9f-9d6b-422c-862d-ec98184a3038).
Authored LibreOffice controls support reference-column allocation for default equal
columns, but ignore custom counts and do not establish Word's balancing geometry.
Validation passes 274 tests and 41 focused controls. The full fixed comparison has
294 identical thumbnails and 3,077 identical full outputs out of 3,080, with the
same six invalid inputs. Changed outputs have source-backed note geometry/content
attribution; one extra final page contains a source-requested empty paragraph
after a continuous column transition, with its repeated header/footer retained.

### Explicit normal notes

Footnote and endnote definitions with `w:type="normal"` are loaded identically to
definitions with no type attribute, preserving body reference numbers and note
text. Separator handling remains separate; continuation separators, continuation
notices and unknown explicit types are excluded from normal note content. This
fix changes definition eligibility, not continuation allocation or separator choice.

### Deferred special-note stories

The opened document retains typed separator, continuation-separator and
continuation-notice definitions for both footnotes and endnotes, together with
settings-selected IDs and each story's source part. Selected empty stories remain
distinct from missing selections. Canonical XML retains fields, relationship
attributes and unresolved marks without mutating numbering or graphics state.
Duplicate definitions remain in source order; this inventory does not decide
rendering precedence. Ordinary separator selection and rendered notes are unchanged.
Continuation rule width and special-story rendering remain unimplemented.
Footnote page fragmentation has the bounded support described below.

### Footnote page fragments

Footnotes retain paragraph units and ordinary table rows while continuing across
pages and compatible equal-width body columns. Each page plans from immutable
incoming source cursors
and commits one plan for the actual accepted body prefix. Later body text keeps
already reserved note space. Newly referenced notes start on their reference page
and column when feasible; an indivisible first line referencing an oversized group
preserves
source order through a continuation queue. Authored note page breaks retain their
boundary. Intentional parity pages forward pending notes without painting them.

Paragraph fragments retain line identity, keeps, widows, spacing and anchors.
Ordinary rows reuse the table splitter. The layout cache also retains the existing
full-note projection; this uses additional memory, and no independent memory or
throughput benchmark is claimed. Page trials share immutable plans for rollback.

Destination areas retain their own body and picture reservations. Equal-width
areas route in column order before continuing on the next physical page; closed
areas keep accepted fragments. Source page breaks skip remaining columns, while
column breaks advance to the next area. Accepted reference order is preserved
when continuous sections revisit an earlier column.

Ordinary nested rows retain local row structure and split recursively across note
areas. Cell margins, terminal spacing and late anchors follow their source
fragments. Rows marked cantSplit stay whole when they fit a fresh area; oversized
rows start fresh and can continue. Supported cell pictures use the same resolved
position for wrapping and painting, including later line-relative anchors.

Top-level vertically merged note cells keep a local source cursor and allocation
history across page and column fragments. Text and late anchors retain their
source-cell reading order and destination geometry. Ordinary nested rows inside
a merged cell split with fresh-area capacity. Repeated headers reserve space
with the next data fragment; an initial header rolls back when data cannot start.
Independent note occurrences have separate allocation histories, including when
they share cached layout.

Body merged cells that already spread text across physical fragments also retain
their anchored objects. Each object uses its source slice's destination transform
and paint order; textbox reading keys keep its text beside the owning line.
This draw-time repair leaves pagination and accepted note reservations unchanged.
Top-aligned body merges with anchors or note references now allocate source
slices before placing each row fragment. The accepted cursor, picture clearance
and footnote plan commit together and roll back together. Adjacent rows share
their merged-cell origin; page and column continuations rebase the remaining
source. Other merged cells retain their own height demand. Textbox reading keys
also distinguish multiple anchors within one source line from their paint order.
Locally anchored objects fit their declared and rendered extents within the
physical page; text wrapping remains governed by the source wrap mode.
Header-origin and exact-height chains retain their existing source policy; their
occurrence-aware continuation remains outside this repair.

Retained row textboxes keep source cell, slice and anchor order, including repeated
header occurrences and fragmented note tables. Floating paint order and positions
remain independent of exported reading order. Whole-note drawing retains anchored
objects from accepted slices, including nested-cell textboxes. It preserves the
existing note allocation and cut/carry selection, with separate paragraph and
line origins and source order independent of floating paint order. Repeated-header
footnote copies and header-crossing merge policy remain separate gaps. Floating
objects in note stories are retained using local source coordinates; this does
not establish Word-equivalent import or placement behavior. Supported local
picture exclusions use the same reflowed note units for whole-note height,
fragment planning and paint. Wrapped picture extent reserves space separately
from text advance, including note-cell row fragments; non-wrapping objects
retain source overlay behavior. A fresh oversized atom keeps its owner at the
physical note-area top and queues following notes; its image can still exceed
the page. Separator text reads after body text and before note content.

Validation: 347 crate tests pass with the same 23 Clippy diagnostics. All 294
thumbnails and 3,079 of 3,080 pagination outputs are byte-identical; the remaining
output changes only separator reading order. Page counts, errors and warnings
are unchanged. All 37 new controls retain source content, with clearance checked
for feasible geometry and finite overflow checked separately. Accepted carry/cut
filters remain unchanged but lack an authored fallback carry-anchor control.

Nested merge chains inside nested rows, root-floating note tables, unequal-width table continuation reflow,
and complete Word-equivalent continuation-story layout remain incomplete. Unsupported
structured fallbacks retain source units but can still clip when their geometry
exceeds a page. Oversized indivisible source content also retains a diagnosed
fallback. This checkpoint does not establish full Word continuation fidelity.

### Selected footnote continuation stories

Selected continuation separators appear on incoming fragments from a previous
physical page; selected notices follow outgoing note fragments. Same-page column
handoffs and untouched deferred notes do not create continuation stories. Empty
selections suppress fallback, while empty paragraphs retain their source height.
Accepted story layouts reserve space transactionally with normal note fragments.
Images, textboxes and source reading order survive selected-story drawing.

The typed continuation rule fills the accepted note container width. This follows
container-width behavior in LibreOffice's layout implementation; exact Word
behavior for multiple columns remains unverified. Ordinary separator rules retain
their existing width policy. Unequal-width table-note continuation remains incomplete.

An oversized special story uses an explicit native recovery policy: excess outer
paragraph spacing can be relaxed, while internal spacing and visible source are
retained. A story that cannot coexist with the normal note's first fragment flows
through separate story fragments. Those fragments do not generate new notices or
separators. Unsupported atomic rows terminate with a diagnostic and can still
exceed the page; this is not a Word-equivalent overflow guarantee.

Validation: 351 crate tests pass with unchanged Clippy warning signatures.
All 294 thumbnails and 3,074 of 3,080 pagination outputs are byte-identical.
The six changed documents preserve page counts, errors and warnings; source
selection ablations restore all six baseline outputs exactly. Source inventories
and separate body/note order are retained, including the added selected notice.
Independent controls cover selection, empty-story capacity, minimum-prefix
clearance, internal spacing, images, anchors and supported merged tables.
Selected real-page visual review found no new apparent clipping or collision.

Selected stories retain source exact line heights. Glyph ink can extend beyond a
short exact-height line and overlap adjacent story ink even when reserved flow
boxes are adjacent; this remains an explicit fidelity limitation.

### Unequal-width footnote paragraphs

Paragraph-only notes use the section's actual column rectangles and remeasure
unconsumed source at each destination width. Accepted prefixes retain their
logical source boundaries, including breaks, references, pictures and transformed
hyphenation text. Numbering, paragraph spacing and source occurrence ownership
remain attached to the original paragraph.

Paragraph-relative objects use the accepted fragment's paragraph origin, matching
native body paragraph continuation. Line-relative objects retain their owning
line origin. Local picture exclusion and note reservation use the same projected
geometry. Deferred inline textboxes are laid out after remeasurement and retain
their source position among ordinary text and floating anchors; whole notes and
retained rows use the same source slots. Floating z-order remains independent of
exported source order. This establishes native consistency, not exact Word anchor
placement for continued footnotes.

Ordinary and nested table notes retain remaining cell source through unequal-width
continuation. Remeasurement preserves paragraph source boundaries, cross-paragraph
picture wrapping, cell context, additive border/spacing extent and remaining
minimum row height. Accepted fragments retain their geometry. Fixed table grids
reserve their occupied horizontal span; conflicting later note areas defer their
unconsumed content.

Percentage note tables resolve their width in both directions against each
destination, preserving saved grid proportions and percentages above 100%. This
also applies to equal-width notes and selected special stories. The model does
not retain `tblLayout` or `tcW` preferences, so this remains a bounded grid-based
policy; conflicting cell preferences and unbreakable-content minima are not fully
implemented.

Vertically merged note cells now remeasure their remaining source across unequal
columns, including merged tables nested inside another merge. Accepted history
keeps each destination's width, margins and fresh capacity. Logical grid intervals
identify merge owners when row cell indices change. Child source descriptors
survive outer merge allocation so a later terminal row can fill earlier accepted
fragments; cached layouts keep separate note occurrences isolated.

Authored controls cover delayed child completion, source order, inline and floating
objects, mixed notes, section transitions and fixed-width collision avoidance.
The source census found no matching nested-merge footnotes, so this recursive
coverage is synthetic. Floating tables, cell text direction, custom note-column
counts and the ordinary fixed-width separator remain separate limitations. Exact
Word-equivalent continuation geometry is not established.

## Theme run colors

Run colors resolve theme references through the main document's theme relationship
and settings color mapping. Cached RGB, theme reference, tint and shade travel as
one property through the style cascade; an explicit RGB or automatic color clears
inherited theme transforms. This includes inherited run styles and list labels.

Tint and shade modify HSL luminance, with tint taking precedence when both are
present. The implementation retains full precision until truncating final RGB
channels. Word's intermediate quantization is not established: source-cache
comparisons still show one-channel differences, so exact Word color parity is
not claimed. Missing or malformed effective theme data falls back to cached RGB.
For system colors, an existing cached run color is retained without applying its
transform again; known window colors and then the theme's last color provide
fallbacks when no run cache exists.

Independent controls cover theme-only colors, stale caches, inheritance, explicit
overrides, mapping, transform precedence and system fallbacks. The full corpus
check preserves raw text, page counts and geometry, including color-driven text
item regrouping.

Independently specified underline colors use the same theme resolver. Underline
pattern and color inherit independently: an omitted pattern preserves the prior
pattern, and an omitted color preserves the prior color. An explicit cached color
or theme reference replaces the color property; tint/shade-only attributes do not
clear it. Explicit automatic color restores the existing automatic underline
behavior, and `none` disables the underline. This follows the
[Word underline implementation note](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/983b4d5a-53ba-4d79-89cb-fc13303af71b).

Shading retains its foreground, background and pattern through the existing
run and paragraph style cascade. Both theme channels resolve before the existing
pattern-color averaging. Cell shading follows the existing table-style,
conditional-style and direct-cell precedence, with explicit clear/automatic fill
able to replace inherited paint. The shared paragraph path also serves headers,
footers, notes and textbox content. See the
[shading property definition](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.shading?view=openxml-3.0.1)
and [Word cell-shading note](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/4f0cccae-70c2-492a-bc94-3ccc39172119).

Pattern averaging remains an approximation of Word's hatch rendering. The existing
cached-color handling of `nil` remains unchanged while its context-dependent
style/reset semantics need separate validation.

Word applies table shading and row table-property shading exceptions only to
cell spacing, as documented in its [table-shading implementation note](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/ee110ab2-af6a-4ee8-923d-9a12a2fe65ab)
and [row-exception implementation note](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/a7c8261d-ea1c-4fc6-a131-fc0e48c5176e).
Cell interiors use their own shading. Table-level shading therefore does not
establish a cell's background for automatic text contrast.

Table and row-exception spacing shading remains unsupported. Current layout adds
vertical cell spacing to row height but retains contiguous horizontal cell bounds;
accurate gap painting requires qualified spacing geometry and row-level spacing
precedence. Zero-spacing controls must preserve cell interiors and text colors.
Text extraction alone cannot verify visibility: background paint can obscure
retained text, particularly around exact-height rows and merged continuations.

Border colors use the same theme resolver for run and paragraph borders, table
and conditional cell borders, row border exceptions, inside edges, cell diagonals
and section page borders. Resolution keeps width, spacing, line style and the
existing nil-selection behavior intact. The retained color source attributes
remain part of border identity after paint resolution, so equal RGB alone does
not merge distinct themed and explicit border definitions. See the
[run-border definition](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.border?view=openxml-3.0.1).

## Plain tab line height

Literal plain tabs set horizontal positions without contributing their run font size to
line height. Surrounding visible text and the paragraph mark supply the metrics,
including larger marks and visible runs. Leaders, underlines, strike formatting,
highlighting and shading retain their existing metric behavior; the existing
trailing-tab rule is unchanged. Generated numbering suffixes keep their existing
metric contribution.

On the original source used to qualify this change, median absolute baseline
error across the same 14 saved Word text records fell from 5.30 to 0.80 points.
All four pages and the complete normalized text order were preserved. Authored
controls cover blank, leading and trailing tabs, larger marks and text, decorated
tabs, and inline and floating pictures. LibreOffice controls support the plain-tab
distinction, but do not establish Word behavior for every decorated-tab case.

Display equations directly in body, cell and story block containers use the same
linear math representation as paragraph-contained equations. Accepted insertions and
move destinations remain visible; deleted and moved-from equation subtrees are excluded.
Direct math runs in display containers retain source order. Local display justification
overrides `m:mathPr/m:defJc`, with centered fallback; `centerGroup` is represented as one
centered linear paragraph. This preserves text, not two-dimensional equation alignment,
per-math-run formatting or a full Word math layout. Bare block `m:oMath` uses ordinary
paragraph alignment. [Math paragraph schema](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.math.paragraph?view=openxml-3.0.1)
and [Word justification defaults](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/7ad89628-e7fc-407e-91d6-c4f18d57c757).

### Native vertical-merge table fragments

Source table provenance retains `w:vMerge` chains by logical grid start and span.
Row provenance retains `gridBefore` and `gridAfter` independently of cell spans.
The worker reserves those omitted grid slots with empty HTML cells, excluding
slots already covered by incoming row spans.
The worker emits HTML row spans only across rows accepted on the current page,
suppresses declared continuation cells, and retains ordinary empty cells. Merged
text keeps its declared source owner separately from its accepted row, including
pages that begin inside a chain. Paragraph and link content comes from emitted
content IDs; full source rows are not injected into page fragments. Pipe Markdown
retains cell text and paragraph separators but cannot represent HTML row spans.

Custom-mark endnote references retain their note bodies and source order without advancing the automatic counter. Automatic `endnoteRef` text is empty for those notes; literal marker text remains as authored. Copying a custom marker from following body text is not inferred because its text boundary is not established by the source reference element.

Table grid spans retain their full positive source value, including values above 64; zero and negative spans retain the existing one-column normalization. Layout and native table provenance share the same checked logical intervals. The parser rejects grids above 65,536 logical columns or a document-wide structural work budget of 16,777,216 units, with an explicit grid-resource error and no partial pages. Work counts `columns × (rows + cells + 1) + sum(spans)` for each parsed table, including nested and selected note stories. These are implementation resource limits, not Word span limits; trailing omitted columns count toward validation without changing their existing layout behavior.

### Plain-text alternate content imports

Internal `w:altChunk` parts declared as `text/plain` are imported at the anchor's
position through the normal paragraph/style/cell pipeline. The reader accepts
ASCII, Unicode BOMs, and explicit UTF-8/UTF-16 charset declarations, with strict
decoding and no code-page guessing. CRLF, CR, and LF delimit paragraphs; blank
lines remain, and a terminal delimiter does not add another empty paragraph.
Tabs and form-feed page breaks retain their controls. Imported text keeps the
host story's source ownership and follows the existing selected-branch traversal.

[ECMA-376 §11.3.1](https://ecma-international.org/publications-and-standards/standards/ecma-376/)
requires internal alternate-format parts and recommends UTF-16 text for
interoperability. [Word's implementation note](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/4822a0c9-64d9-4099-bf2b-80b1514c7674)
lists `text/plain` among supported types. Paragraph-per-line import agrees with
LibreOffice's plain-text reader; a fresh Word rendering oracle was unavailable.
This is content-preservation support, not a claim of matching Word's import typography.

Import expansion shares an 8 MiB source-byte budget and a 65,536 synthetic-node
budget across parsed stories and repeated anchors. A chunk is omitted whole if
it exceeds either budget. Unsupported types (including RTF), invalid
relationships, decoding failures, and resource limits produce distinct
`ALTCHUNK_*` warnings with part/relationship context and no payload text. These
warnings identify incomplete output; they do not activate the worker's LibreOffice
fallback. RTF import and legacy plain-text encodings remain unsupported. External
relationships are not fetched. The fixed 3,080-source evaluation contained no
alternate-content imports, so recovery evidence is limited to authored controls.

### Structured HTML alternate content

Internal `text/html` chunks use HTML5 parsing and a bounded CSS subset before
projection into the ordinary Word paragraph, table and drawing flow. Supported
content includes paragraphs and inline formatting, literal list markers, table
row/column spans, external link targets, and raster images from package-local
parts or base64 data URIs. HTML encodings honor BOM, MIME and meta declarations;
absent declarations use Windows-1252. Invalid encoded text is rejected without
replacement characters. Image bytes use a deduplicated in-memory package overlay;
the original ZIP and relationship cache remain unchanged.

Supported CSS includes basic font/color/decoration, alignment, spacing, simple
selectors and cascading priorities. Table row heights are minimum heights. A
complete first row of percentage cell widths can establish proportional columns;
conflicting or unresolved percentage constraints produce a diagnostic. Unsupported
selectors, properties, values and visible structures produce
`ALTCHUNK_HTML_UNSUPPORTED`, identifying incomplete import. This includes advanced
layout, captions and unsupported embedded objects. RTF, XHTML and MHTML are not
imported; remote and filesystem assets are not fetched. These warnings do not
activate worker fallback and do not establish complete HTML or Word fidelity.

DOM and generated Word nodes share the import node budget. Depth, selector work,
table expansion, decoded raster area and imported asset count/bytes are bounded
before their respective expansions. The HTML tokenizer checks its node budget
between small input batches, so a final batch can temporarily exceed that node
count. Table replacement eligibility is evaluated after active imports materialize:
complete text-only rows retain source-owned fragment HTML, while incomplete imports,
images, nested tables and blank rows retain ordinary native output. Each chosen
import is charged once per occurrence, including repeated anchors and special stories.

Direct ordinary charts with wrapping and nonnegative paragraph/line offsets share raster-picture page and footnote fit retries. Their source coordinates remain unchanged; fixed page/margin anchors, generic groups, text frames, chartEx and impossible fresh-page offsets retain existing placement policies. This is bounded source-preservation behavior, not an exact Word pagination claim.

## Internal bookmark destinations

Modern `w:hyperlink` anchors and `HYPERLINK \\l` fields retain typed source targets.
Names resolve across active document stories. A unique source bookmark has a stable
part-and-occurrence identity; repeated header and row placements share that identity.
The earliest accepted placement is canonical, even when only a later page is requested.
Missing names, duplicate source names and source targets without an accepted placement
remain distinct unresolved states. Existing external URI annotations retain their format.

Bookmark boundaries travel through layout without adding visible text or flow height.
Targets inside exported text carry an MCID and UTF-16 offset; glyphless targets carry a
boundary in the final raw text-item sequence. Block-level targets omit paragraph identity.
Table-edge targets belong to the accepted first or final row fragment, independently of
cell ownership. Coordinates use the native page's top-left origin and page indices refer
to the original physical document, before selection or crop.

The worker must retain the target through its final output before emitting a fragment URL.
Native destination metadata alone does not establish a usable final link. Standalone
`REF`/`PAGEREF` fields with an unquoted `\h` switch use the same bookmark target pipeline,
as documented by Microsoft's [REF](https://support.microsoft.com/en-us/word/field-codes-ref-field)
and [PAGEREF](https://support.microsoft.com/en-us/word/field-codes-pageref-field) references.
Their cached results remain unchanged; dynamic field evaluation and other navigation
field types, implicit REF instructions without the keyword, and evaluated nested
instruction targets remain outside this implementation. Format/separator arguments and quoted
switch-like text do not activate navigation; unknown reference switches remain unlinked.
Enclosing modern hyperlinks and HYPERLINK fields keep ownership of their cached reference
results. Nested REF-only fields use the innermost result scope. Logical source fragments
from one condensed painted run remain separate during text deduplication, including
repeated punctuation; duplicate overlay copies still follow the existing filter.
Transient painted-run identity is consumed before DOCX serialization and omitted
from the shared PPTX text export.

Internal-link ownership is exported as logical text fragments while shared glyphs
remain in one shaped paint run. Coalescence uses the retained pre-link style identity,
including the existing separate style identities for table-cell runs. Fragments crossing a glyph cluster share that
cluster's bounds. Destination `referenced` is computed from resolved links in the
full accepted document before page selection; unused and ambiguous definitions
remain distinguishable without requesting output anchor transport.

Formatting comparisons retain pre-internal-link source semantics for decoration,
fit-to-width grouping and automatic hyphenation. Hyphenated runs retain each logical
source interval; generated hyphens use the owner immediately before the split.

### Picture list bullets

Picture numbering resolves `lvlPicBulletId` through the numbering part, including level overrides. Each literal Unicode scalar in `lvlText` contributes one image with the source aspect ratio; an empty label contributes none. Image height is approximated as 0.8 times the resolved numbering-label font size, including paragraph-mark inheritance and explicit level overrides. Public Word screenshots support font-relative sizing but do not establish a universal formula: the candidate underestimates visible ink height by approximately 17% in one large-font reference after source-padding correction. This approximation avoids treating oversized VML source frames as literal list-marker extents. Missing definitions, unresolved assets, invalid dimensions, and failed image decoding retain the existing text label. Tables containing decoded picture labels remain in the ordinary text/image pipeline rather than text-only table substitution. This preserves authored images; it does not establish exact Word baseline alignment for every picture-bullet format.

### Fully hidden paragraphs

When all run content and the paragraph mark have `w:vanish`, ordinary story paragraphs occupy no space and paint no borders or shading. The accepted bookmark position remains in the story. A visible mark or visible content retains its paragraph, and table-cell marks keep their existing cell-layout behavior. This follows the saved Word preview and local LibreOffice agreement on a document containing hidden bordered form delimiters.

### Relative vertical alignment of text-anchored tables

For a floating table with `vertAnchor="text"`, `tblpYSpec` is ignored and the numeric `tblpY` offset (zero when omitted) positions the table against its text anchor. Page and margin anchors retain relative alignment precedence over numeric offsets. This follows [ECMA-376 Part 1, §17.4.57, `tblpYSpec`](https://ecma-international.org/wp-content/uploads/ECMA-376-1_5th_edition_december_2016.zip). Treating an ignored `outside` alignment as bottom-margin positioning can move otherwise fitting form fields onto the following page.
