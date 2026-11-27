# Word decoder (`office::docx`)

Native `.docx` decoder. It lays a Word document out into pages the way Microsoft Word
does (styles, line breaking, pagination, tables, headers and footers, notes, floating
objects) and describes every page in the extract format liteparse reads: an index object
plus one record per page holding text runs with their boxes, vector paths, picture
positions and, on request, a rendered screenshot. Binary `.doc` input is converted to a
`.docx` package in memory by [`office::doc`](../doc/README.md) first.

## Usage

`office::docx::extract::read(config_json, bytes)` decodes a document and returns the
index and the page records. The request format is `office::pptx`'s (`pptx/config.rs`);
its `presentation` block is ignored. The index has `sourceFormat: "docx"`, and each page
record a `document` block (section, page number). liteparse calls it through
`office::adapter`, which maps the records onto its own pages.

## One rendering path

Rendering behavior is selected by document properties, with no environment-variable
feature switches. Bidirectional text, merged-cell continuation allocation, column
balancing and column-width measurement use the same path for every document.
Ruby, drop caps, endnote placement and automatic hyphenation follow their source
properties. Existing unsupported cases remain fidelity work; there is no alternate
experimental runtime mode.

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

`office::docx` uses `office::pptx` for the package, XML tree, fonts, scene,
rasterizer, outputs, screenshot thread and PDF writer. Its own work is Word layout:

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
`metrics/word_tables.rs` stores **numeric metrics only**: OS/2 win and typo vertical
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
from them are stored, as factual measurements rather than the font software. No metric comes from fonts embedded in documents. Not covered: Yu Mincho
(only a Windows 8 build was found).

### Hyphenation patterns

`hyph/` vendors two pattern files from the hyph-utf8 package
(github.com/hyphenation/tex-hyphen), unchanged and with their headers kept; licences are
listed in `hyph/LICENSE.md`.

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

Checked by per-document A/B against Word's page-1 thumbnails (69 documents;
colour SSIM, ΔE, worst tile, missing/extra ink, line positions):

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
`office::pptx`'s preset geometry and fill/line readers; text box contents are laid out in the
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
A consumer rebuilding HTML tables reserves those omitted grid slots with empty HTML cells, excluding
slots already covered by incoming row spans.
It emits HTML row spans only across rows accepted on the current page,
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
warnings identify incomplete output; they do not trigger a LibreOffice
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
trigger a fallback and do not establish complete HTML or Word fidelity.

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

A consumer must retain the target through its final output before emitting a fragment URL.
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
