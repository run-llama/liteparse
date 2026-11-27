# Binary Word reader

This helper extracts raw text stories and partial source-backed transcription from Word 97+ compound files. It reads the effective FIB revision, selects the source's table stream, and decodes the CLX piece table into body, footnote, header, comment, endnote and textbox stories. JSON retains source control characters and field instructions. A separate `transcription` object applies supported cached fields, accepted revisions, symbols and list labels, with UTF-16 source spans and explicit unresolved diagnostics. Unsupported formatting, numbering and story assembly remain gaps. The helper is not connected to production routing.

```sh
cd rust
cargo build --release --locked -p doc-read
./target/release/doc-read input.doc > inspection.json
cargo test -p doc-read --locked
cargo clippy -p doc-read --all-targets --locked -- -D warnings
```

## Content evaluation

Content coverage is evaluated independently of page rendering. Use an explicit approved
development roster; each JSONL row supplies `source_sha256` and `source_path`. Family
holdouts must remain outside that roster and outside development batch selection.

```sh
python3 rust/crates/doc-read/tools/content_scorecard.py \
  --roster /path/to/development.jsonl --expected-count 1085 \
  --binary /path/to/optimized/doc-read \
  --cache /path/to/evaluation/cache --output /path/to/evaluation \
  --renderer-census /path/to/renderer/census.jsonl
python3 -m unittest discover -s rust/crates/doc-read/tools -p test_content_scorecard.py
```

The harness verifies source hashes, pins the executable and roster, and caches full
inspection JSON outside the repository. `rows.json` retains per-story accounting;
`summary.json` and the self-contained `report.html` separate raw UTF-16 source units,
projected output, unresolved ranges, visibility uncertainty and zero-CP generated
labels or form values. The document status describes reported logical extraction
uncertainty. Raw retention and complete CP partitions do not establish semantic
accuracy, visible reading order or current field values. Schema 2 also retains explicit
component errors and unavailable metadata in each row. These make a document partial
even when its raw source accounting is complete; they do not prove missing visible
text. Summary counts report unique affected documents and overlapping components.
Compare both helper caches with the same evaluator version when measuring parser
gains, and retain the evaluator hash separately from the parser executable hash.

Suppressed field instructions, deleted revisions and hidden text are counted by
reason. They are not missing visible words. Unclassified empty nonhidden lexical
ranges are review candidates, and unsupported generated labels need separate
anchor/family evidence because they consume no authored CPs. Parser metadata is
source evidence rather than independent Word ground truth. Rendering completion,
fidelity diagnostics, and reference comparisons remain separate measurements.

`representative-batch.json` selects deterministic mixed-component cases using
component/diagnostic pairs and optional renderer first stops. It is a development
coverage batch, not a random accuracy sample. For offline reproduction, use
`--cached-only --cache-meta /path/to/cache-meta.json` with the exact original roster
and cache plus its `cache-manifest.json` payload hashes; the report explicitly labels
the historical helper identity. Optional `--reference-manifest` entries bind a source
hash to an evidence file, SHA-256, kind and scope. Missing or changed references are
reported separately and cannot become an accuracy score. Use `--baseline-rows` to
compare the same source cohort; raw story identities must match, and generated-anchor
deltas remain separate from source CP counts. Exact added-label correctness still
requires source-family and text-inverse proof. Missing
inputs or invalid accounting remain per-document errors. No document text or
private dataset paths belong in committed scripts or reports.

Inputs are limited to 64 MiB and text to 16 million UTF-16 units. Malformed allocation chains, invalid text, unsupported revisions and encrypted inputs fail explicitly. Only root-level document streams are read; embedded objects have separate ownership. Success is labeled `raw_stories_only` and is not a production completeness signal.

Top-level `LISTNUM` fields with absent or empty cached results report `listnum_empty_result_unresolved` at the field's source position. This includes fields containing nested arguments. A `LISTNUM` nested inside another field remains suppressed; nonempty cached results retain their existing text. Reconstructing uncached numbers still requires source list and sequence resolution.

The development `doc-render` helper maps qualified source formatting and structure into the shared document model and writes page images, PDF, page JSON and source-ownership ledgers. It supports a bounded body, header/footer and table scope; unresolved components reject the whole document. Validation tracks source text, geometry, page counts, failures and performance. Saved previews can be stale; LibreOffice remains a comparison baseline.

Format references: [MS-CFB](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cfb/53989ce4-7b05-4f8d-829b-d08d6148375b), [FIB](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/9aeaa2e7-4a45-468e-ab13-3f6193eb9394), [compressed character mapping](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/aa2e55a2-f4f2-4795-bab5-6d9d7a0ed249). The mapping deliberately preserves U+0080 rather than applying Windows-1252's euro substitution.

Character-formatting primitives decode CHPX pages into source-addressed property runs, preserving ordered SPRM operands and style-relative toggles. Revision evidence resolves supported direct and piece properties. Both projection and formatting metadata have explicit resource bounds.

The CLI exposes per-story `fields` metadata, cross-checking indexed markers against UTF-16 source positions. Missing and malformed tables have separate statuses; some field types are intentionally absent from the index. `revisions` reports source-property evidence. The transcription applies known deletions for the accepted-revision view, retaining zero-output source spans and preserving raw inspection. Unknown visibility and revised field structure remain diagnosed. Failed revision projection retains the preceding text and reports the error.

`hidden_properties` resolves supported ordinary hidden-text formatting from paragraph styles, character styles and ordered direct properties. Direct paragraph-end properties can establish a non-table baseline even when unused table styles exist; conditional table contributions remain unresolved. Valid logical table alignment, repeat-header, preferred leading-indent, anchor and wrapping-distance properties preserve a table visibility baseline only when the selected style contributes no text properties and physical row ownership and revisions are resolved. A language-only selected-style contribution retains ordered complex-script language metadata and can independently qualify hidden visibility. It remains nonempty for list and capitalization consumers. This qualifies text visibility; table positioning and header repetition still require rendering support. The final `transcription` suppresses proven hidden ranges for a hidden-text-display-off view, while `accepted_transcription` retains the preceding text. Suppressed ranges keep zero-output source spans. Unknown styles, conditional table formatting and ambiguous generated labels remain explicit; a partial visible result is not a completeness guarantee. Textbox byte ranges are calculated after this projection. If metadata or projection fails, the preceding transcription is retained with a diagnostic.

Uncached `EQ` overstrike fields recover a single literal operand when the other operands are blank. Indexed EQ fields with an empty cached result can also recover that literal after separator ownership and accepted/hidden visibility checks; nonempty cached results retain priority. EQ literals and ruby bases share one projection pass so typed ruby guide annotations reference final output byte ranges. Recovered text keeps its original source positions. Nested equations, composite glyphs, unsupported syntax and visual placement remain unresolved.

Legacy checkbox payloads can use a versionless flags header and byte-counted strings. A bounded compatibility decoder accepts fully framed ASCII checkbox records with supported flags and retains their raw current state, default, and encoding provenance. Its layout and state interpretation follow Apache OpenOffice's WW8 importer; exact-source Word validation remains open. Current state 25 renders unchecked under that compatibility interpretation, independently of the retained default. Other legacy form types, non-ASCII strings and unsupported payload variants remain unresolved; existing field ownership and visibility checks still govern projection.

The localized checkbox token `FORMCASECOCHER` has a bounded compatibility path: exact instruction syntax, indexed checkbox type and independently owned matching binary form data must agree. Raw instruction units and field-result flags retain identity evidence. This path accepts no cached result, nested field, edited result or unsupported flags; identity qualification remains separate from revision and hidden-text eligibility. Other localized tokens remain unresolved. This combined evidence supports compatibility interpretation, not a claim of independent Word validation. Modern checkbox state 25 is treated as unchecked under [MS-DOC FFDataBits](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/22a1f1e5-f4b2-4a7d-9e10-3afa26056122); the reset default remains separate.

`structure` maps physical FKP paragraph endpoints through the source piece table and records per-story source ranges and projected byte ranges. Interior control characters do not independently create boundaries. Suppressed terminal marks and unexpected terminators remain diagnosed; source paragraph ownership does not establish visible paragraph merges, table topology or visual reading order.

`table_structure` assigns physical paragraphs to source cells and rows, with nested-row references and explicit unresolved ranges. Each paragraph has one direct owner; enclosing ranges can overlap nested content. Cell definitions preserve merge flags and width units without suppressing content. Adjacent-row table grouping and visible merged/revised topology remain unresolved. Metadata failures leave transcription intact.

`paragraphs` preserves source PAPX fragments, and `styles` preserves stylesheet identities, inheritance links and raw property groups. These remain inputs to paragraph and numbering resolution. Invalid metadata is reported independently so it does not block raw story extraction.

Default body `AUTONUM` fields generate ascending labels with periods after revision projection when source styles, paragraph properties, field markers and list headers establish that sequence. Unsupported heading resets, switches, custom list emulation, hidden properties or revisions leave the sequence unresolved. Generated labels retain their complete source field ranges. `fonts` exposes bounded source font-table metadata for subsequent symbol interpretation.

Explicit symbol characters use source font/code evidence and published Adobe/Unicode mappings. Unsupported slots retain the original marker with `unresolved_symbol` provenance; unrelated text and generated labels remain unchanged. `list_definitions` decodes source levels and overrides, including their raw property groups, for subsequent ordinary-list generation.

`ordinary_symbols` resolves supported private-use text through its effective non-ASCII font and published symbol mappings. The initial scope requires known Western language, source formatting and visibility, an empty default character style, and no embedded fonts or table font contributions. Unknown contexts retain their source characters and diagnostic evidence. Unicode replacements keep their original source positions and run before final form, equation and structure byte ranges are calculated; raw stories and the preceding accepted-revision view retain their source encoding.

`capitalization` preserves logical spelling while reporting independently resolved all-capital and small-capital presentation properties. It records source CP ownership and final projected byte ranges, together with hidden/deleted state and unresolved style histories. A physical paragraph with a proven empty table-style text contribution supplies the initial capitalization baseline before paragraph, character and direct formatting. Coalesced source controls and field-display literals use source-verified scalar mappings with a sparse boundary index. Generated labels require their own formatting context; ambiguous partial transformations retain source ranges without guessing output slices. Visible Unicode casing, font shaping and small-cap sizing remain renderer work.

`render_model::sections::resolve` preserves ordered source page geometry in twips, specified defaults and per-property limitations. It retains inactive grid properties and checks that margins and column gaps leave room for text. The CLI does not invoke this library resolver yet; geometry resolution does not qualify a whole document for rendering.

`render_model::settings::inspect_settings` reads versioned FIB/DOP layout inputs, source compatibility bits and feature restrictions. It retains source offsets, flags installation overrides and withholds extension semantics when the recorded version and length disagree. The recorded installation language may be normalized by Word; content-dependent compatibility and rendering-view qualification remain required before layout.

Ordinary list projection supports source-proven decimal, Roman and English alphabetic templates, restart counters and bullets. Multilevel placeholders use the closest prior ancestor in the same list override, separately from counters shared by list identity. A linked paragraph style is supported when it already matches the physical paragraph style and bounded ancestry, font, language, visibility and direct-mark checks establish the label properties. Simple bullet formatting overrides select the complete overriding level when the entire list family uses bullets and symbol-font identity is established. Missing ancestors, different linked styles, mixed numeric/bullet override families and unsupported properties leave the affected sequence unresolved. Table-cell lists require exact physical paragraph ownership in a complete, unmerged depth-one row whose selected table style contributes no text properties. Counters follow source paragraph order across cells and outside paragraphs; row marks and unqualified members keep their list family unresolved. This provides logical labels, while visual table order remains unresolved. Generated labels have zero-width source anchors, so consumers must distinguish generated output from source CP ownership. Label expansion and template sizes are bounded.

Proofing-exclusion toggles preserve logical list text after exact operand validation. Spelling and grammar analysis settings do not resolve visibility, font identity, revision history or list-family membership; those checks still apply.

Valid complex-script bold and italic toggles preserve logical list labels after exact operand validation. Font, direction, visibility and list-family guards still apply; rendered weight and slant remain unresolved. Section-only numbered paragraphs retain an explicit uncertainty diagnostic: their exclusion from counters has LibreOffice corroboration but lacks independent Word validation.

Picture-bullet properties are evaluated in source order within the paragraph-mark and list-level property groups. Ordinary labels remain available when each group resolves to inactive picture use; active picture bullets still require image ownership and rendering support. The picture size bit and undefined bits do not enable a picture, while invalid source positions or operands keep the sequence unresolved. These properties remain forbidden in style groups.

Numeric list overrides preserve the shared list counter across override switchbacks. A start-only override applies its scalar at the first source use of that override and level; combined start and formatting flags use the embedded level's start. A formatting-only override changes presentation while retaining the base list's counter start, following the [Microsoft Word 97–2007 specification](https://download.microsoft.com/download/0/b/e/0be8bdd7-e5e8-422a-abfd-4342ed7ad886/word97-2007binaryfileformat%28doc%29specification.pdf), page 48. Lower-level restart rules, source membership and visibility checks still apply. Counter history and label expansion are bounded; unsupported formats and uncertain family members remain explicit diagnostics.

`notes` links body references to footnote and endnote ranges, distinguishing automatic labels from custom source marks. Supported modern section numbering and legacy Word 97/2000 document numbering generate decimal, Roman or Chicago-symbol labels after checking revision and special-character properties. Body and note-side markers resolve independently, so an absent note-side marker can leave a known body label intact. Page restarts, unsupported number formats and ambiguous markers remain unresolved.

`textbox_anchors` links source textbox ranges to shape anchors where the binary records establish ownership. `textbox_flow` adds UTF-8 byte ranges into existing projected stories, reusable-record accounting and source anchor order. It preserves the transcription and reports unresolved linked shapes. Source anchor order does not establish visual reading order: wrapping, pagination and shape overrides still need resolution before assembling textboxes with body content.

The [transcription comparison harness](tools/TRANSCRIPTION.md) freezes helper binaries and verifies source/conversion hashes. Its agreement diagnostics are separate from independent correctness evidence and held-out family evaluation.

The development `doc-render` crate consumes `render_model::plain::resolve` to render
complete qualified Western documents with plain headers/footers through the shared
paginator. Source run/paragraph/section properties, slot guards and character
ownership remain explicit. Actual font and kerning qualification happens in the
renderer before export; model validation alone does not establish layout fidelity. Unsupported
components reject the whole document; this does not change the inspection CLI or
install a production worker route. See `../doc-render/README.md` for the artifact
contract and validation limits.

The Western rendering repertoire includes en/em dashes, curly single/double quotes,
bullets and ellipses under the resolved default font hint. Source font selection and
actual glyph/text accounting remain required; directional controls, discretionary
breaks and special spacing characters retain explicit unsupported diagnostics.

Uncached numeric `SYMBOL` fields with an explicit published symbol font can emit a
source-anchored logical character after indexed identity, visibility, revision and
embedded-font checks. Cached output remains unchanged. Copyright serif/sans slots
normalize to `©` while retaining their source code and variant basis; glyph appearance
is unresolved. Unsupported instruction switches, fonts and encodings retain diagnostics.

Table rendering resolves ordered cell insertion and physical width changes separately from preferred widths. Selected styles can contribute empty text properties, padding and supported row borders. A style reset retains explicit uncertainty for earlier cell-width preferences until later definite assignments cover those cells. Undefined padding is accepted only when current, selected-style and standard margins coincide. Ordered autofit and cell no-wrap preferences feed the shared intrinsic-width layout; absolute preferred cell widths disable the no-wrap influence. Mixed row layout modes and inconsistent table-width preferences remain unsupported. This component support does not qualify documents with unresolved borders, shading, indentation or paragraph layout.

Decimal-zero list numbering prefixes values 1–9 with zero and retains ordinary decimal text for zero and values of ten or more. Legal-number formatting preserves this format for ancestor labels. Source counter, visibility, linked-style and font checks still apply; missing ancestors retain explicit diagnostics.

Rendering retains a known hidden, empty non-table paragraph as a source-owned CR with zero flow height. Hidden text in nonempty paragraphs and table cells remains unsupported. The table line-height compatibility bit is inactive only when the resolved section has no document grid; its raw source value stays in the model.

List transcription accepts valid document-grid participation toggles while retaining source font, counter, visibility and style checks. Grid spacing is a rendering property; this support preserves logical label text without qualifying grid layout.

Single underline rendering accepts ordered CKul values 0 and 1 through paragraph and character styles and direct formatting. The shared renderer paints the rule in the source text color, including decorated tabs. Other underline styles and independent underline colors remain unsupported; omitted source spaces require separate decoration qualification.

Rendering admits source-owned manual line and page breaks within body paragraphs. Source character positions survive the shared layout projection, including consecutive breaks and empty pages. Style separators, clear directives and column breaks remain separately guarded.

Paragraph rendering resolves automatic-spacing switches through inherited styles and direct formatting in source order. When both switches end disabled, stored explicit before/after distances apply. Active automatic spacing remains unsupported; a disabled switch on one paragraph does not qualify other paragraphs in the document.

Run rendering resolves selected character-style ancestry and subsequent relative toggles from the selected style. Source direction and hint exceptions survive style selection. Style expansion and recorded property evidence share bounded work across body and header stories. This source-property support does not qualify footnotes or other secondary content for rendering.

Text rendering resolves source palette colors and explicit RGB values in formatting order, preserving automatic color separately from explicit black or white. Palette index 13 remains unsupported pending independent qualification of conflicting reference information. Colors alone do not qualify a document's remaining fonts, lists or layout.

Modern table rendering supports ordered cell-side border assignments, clear-pattern RGB cell backgrounds and signed preferred table indentation. Style selection resets these properties; patterned shading, diagonal borders and mixed row indentation remain explicitly unsupported. Preferred indentation is checked against actual laid-out widths.

Logical list transcription accepts a printable ASCII bullet without an explicit level font only when source defaults, style ancestry, all paragraph character spans and list properties agree on one ordinary font. Installation overrides, external template updates, embedded fonts, alternate names, conflicting font identities and unresolved piece properties keep the family unsupported. This source agreement establishes logical label identity; it does not qualify list rendering or font appearance.

`notes::inspect_note_ownership` returns source-owned footnote ranges, automatic labels, authored reference code units and separator contents for documents with qualified section-specific settings. Custom markers preserve their authored text; the API does not infer marker length or glyph appearance. Consumers must separately qualify visibility, formatting, separator geometry and pagination before rendering.

Valid emboss toggles preserve logical list eligibility across style, paragraph-mark and list-level properties. Toggle operands remain validated, and the full list-family visibility and font checks still apply. Emboss appearance requires separate renderer support.

Footnote ownership records each note’s section and retains settings for sections without notes. `FootnoteDocument.position` and `position_basis` are optional summaries: use `sections[note.section_index]` for authoritative settings when sections differ. Continuous and per-section numbering preserve source counters; page-based restart remains unsupported.

Table borders support source-qualified double and triple bands with per-stripe widths. Adjacent effective edges must agree when either uses multiple bands; unresolved conflicts and thin/thick variants remain rejected.

Footnote ownership retains coexisting endnote links and their source-story range. Automatic labels are matched by exact body-reference ownership, keeping footnote and endnote sequences separate. Rendering must independently qualify the retained endnote story before export.

Automatic note-marker lookup excludes ordinary U+0002 text only when source special-character, revision and field flags are fully known. Ambiguous or unknown candidates remain unsupported; a unique special marker retains its generated note label and source anchor.

Footnote ownership distinguishes automatic special-character markers, explicitly absent automatic markers and authored references. An absent note-body marker retains the complete authored note and does not authorize synthesizing a prefix. Source ownership qualifies 70 of 72 note-bearing development inputs; this adds no transcription labels or complete renders.

Legacy section outline definitions use shared exact operand validation for header and note ownership. Their ordered source properties remain intact while note numbering qualifies independently. This expands source-owned notes to 71 of 72 development inputs; unsupported section-position values remain explicit.

Modern table-look operands retain historical auto-format indexes and validate their flags after the selected style is qualified without conditional contributions. They preserve explicit cell geometry and paint; unsupported conditional table styles remain guarded.

Word 2000 physical table grids support ordered row borders, per-cell border colors, preferred widths and indentation, autofit switches, table-look flags and side-specific default padding. Legacy gap-derived padding is retained on untouched sides. With qualified row borders, a cell border marked None inherits while Nil suppresses; later grid definitions replace earlier cell properties. Earlier Word 97 inputs retain their existing version-specific path.

### Best-effort extraction summary

The CLI returns raw stories and logical transcription independently of page-render qualification. `extraction_summary` accounts for source and output ranges, suppressed segments, unresolved controls, and generated anchors. Existing text and visibility evidence remain available in their original fields. These counts describe source accounting; they do not establish visible-text accuracy, reading order, or complete feature support.

Cached HYPERLINK rendering uses the saved display result for supported field envelopes. Instructions and delimiters retain source ownership without painting. This does not evaluate links or establish that Word would display the saved result under every viewing mode.

Text-box rendering currently supports one foreground, column/paragraph-relative box anchored at the start of the body. The source ledger retains authored text and reusable storage separately. The renderer checks the painted glyph bounds and emits explicit fidelity diagnostics for inferred line metrics. Linked, header and multiple text boxes and other placement contexts remain unsupported. Cached body hyperlinks can coexist with the supported text-box anchor. Table-only compatibility settings do not block documents that contain no tables.

The CLI's `linked_notes` object exposes existing logical footnote and endnote text in body-reference source order. Each entry includes `body_cp`, the note's UTF-16 `source_cp` interval, its `logical_utf8_range` within the corresponding transcription story, and `text`. Automatic/custom reference metadata remains separate from displayed labels. Global source-relationship limitations and local visibility or projection uncertainty remain explicit. A suppressed body reference does not silently discard its linked note; the entry reports that uncertainty. This output does not establish page placement or add text absent from the existing transcription. If the extension cannot be decoded, existing inspection and transcription output remain available.
