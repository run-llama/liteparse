# Binary Word format notes

Read the section for the area you are changing; the crate-wide rules are in
[`AGENTS.md`](./AGENTS.md).

## Container, pieces and source positions

- Validate every referenced sector against the input bounds; report unreferenced partial tail
  bytes separately from truncated allocation chains.
- CLX may end one CP beyond the authored FIB stories; preserve that piece provenance while
  requiring character-format coverage only within authored stories.
- Derive paragraph boundaries from physical FKP endpoints mapped through CLX pieces; interior
  controls and piece cuts do not establish paragraph ends or visible revision-aware topology.
- Expand PAPX references only within authored stories; retain terminal nonstory modifiers in raw
  inspection and keep physical FKP paragraph endpoints unchanged.
- Paragraph properties come from the physical terminating FKP and PCD even when earlier edited
  pieces lie beyond PlcBtePapx; use canonical endpoints for control ownership and preserve raw
  coverage gaps.
- Map partial transformed output only after verifying the source-to-output scalar rule; build
  sparse formatting-boundary indexes in one pass and preserve unknown symbol or generated-number
  ownership.

## Character properties and SPRMs

- Resolve implicit property effects in order: CSymbol and CPicLocation set CFSpec, and later
  explicit modifiers can override that state.
- Identify character SPRMs by their exact opcode: 0x0868 controls document-grid participation,
  while 0x0818 controls style separators; preserve their distinct grammar and logical-text
  effects.
- CIstd resets paragraph presentation before applying the selected character style; retain its
  property exceptions, update the direct-toggle baseline, and charge ancestry and replay across
  document stories.
- Capitalization preserves logical spelling; resolve default character style 10 before direct
  toggles, and let CPlain reset to paragraph properties while retaining unresolved output
  ownership explicitly.
- U000B defaults to an ordinary line break; retain paragraph-style uncertainty only when its
  CFSpecVanish property is true or unresolved after style and own-piece formatting.
- Resolve Ico text colors with exact one-byte operands and ordered RGB overrides; an explicit RGB
  value in the same property group prevents a saved preview from independently qualifying the
  palette entry.
- Underline admission preserves ordered source resets and painted tab/space effects;
  omitted-source-space proofs require separate decoration qualification.

## Symbols and fonts

- Keep generated symbol-label Unicode text separate from the source symbol cmap code; validate
  the exact selected font glyph and advance before painting.
- Ordinary private-use symbols require ordered effective font selection and embedded-font checks;
  preserve source character ownership when Unicode replacement changes output byte length.

## Fields and form fields

- Resolve field display semantics before projecting instruction literals; nested EQ fields inside
  other fields have different display rules from top-level equations.
- Distinguish a field nested inside another field from a top-level field containing nested
  arguments when applying suppression rules and unresolved-result diagnostics.
- EQ ruby fields preserve literal base CP ownership and guide annotations separately; indexed
  field boundaries and accepted/hidden visibility establish projection eligibility.
- Project indexed EQ literals and ruby bases in one source-owned pass so both annotation sets
  describe final output byte ranges, including empty cached-result separators.
- Generated SYMBOL values require inspector-owned eligibility and the same accepted source view;
  recheck the field envelope during projection and preserve glyph variants separately from
  logical Unicode normalization.
- Form fields retain current values separately from defaults; require source binary-data
  ownership and accepted/hidden visibility before projecting a selected dropdown value.
- Legacy form payloads may start with flags rather than a version DWORD; require complete
  format-specific framing and retain current state separately from the reset default.
- Localized form identity requires exact instruction syntax, indexed type and owned matching form
  data together; cached-result flags and type identity do not establish visible eligibility.

## Lists and numbering

- Generated list labels use explicit zero-width source anchors; apply source-length-dependent
  projections before inserting them and validate source coverage separately from generated
  output.
- Evaluate generated labels at unique physical paragraph anchors; a marker belonging to a whole
  paragraph cannot be reassigned to an interior reference fragment.
- Ordinary list ancestor lookup uses the closest prior valid paragraph with the same iLfo; level
  counters share LSID identity and retain next values separately from ancestor values.
- Resolve numeric counter starts separately from presentation overrides: format-only LFOLVL
  records retain the base LST start, while combined flags use the embedded LVL start.
- Formatting overrides replace the complete list level; when enabling a previously blocked bullet
  family, validate font identity for every newly eligible member sharing its LSID.
- Inherited ASCII bullet identity requires agreement across source defaults, styles, complete
  body/mark spans and level properties, with one shared work budget; installation overrides and
  template updates invalidate that agreement.
- Resolve picture-bullet enablement in property order; disabled state can preserve ordinary
  labels, but size and unused bits do not establish enablement and style-origin picture
  properties remain invalid.
- Presentation-only list modifiers need exact operand validation while preserving family-wide
  visibility and font guards; recovering labels also requires rebasing capitalization, structure
  and table ranges.
- Treat complex-script weight and slant as presentation properties during logical list
  transcription; validate toggle operands and keep routing, font and visibility guards separate.
- Proofing-exclusion toggles preserve logical list text; validate their operands while retaining
  visibility, font and history guards.
- Alphabetic list labels depend on the resolved language and repeat letters after Z (AA, BB);
  preserve unresolved-language diagnostics instead of assuming a universal alphabet.
- ArabicLZ prefixes only values 1–9 with zero; preserve that format during legal ancestor-number
  conversion and retain source-linked font/style guards.
- ArabicAlpha numbering uses NFC46 with a trailing U+200C; resolve label and punctuation fonts
  from style, terminating-mark and LVL properties while keeping visual bidi shaping separate.
- Keep RTL source text in logical order and retain physical paragraph direction separately;
  unresolved RTL numbering blocks its shared list sequence while unrelated sequences retain their
  own guards.
- Keep NumRM snapshots as historical evidence; accepted numbering follows current list
  properties, and numbering revision properties are forbidden in paragraph and table styles.

## Tables

- Word 97/2000 may omit default table styles; validate the binary version and absence of
  table-style definitions before applying modern table-style requirements.
- TDefTable can omit TC80 records (default cells) or include ignored extras; validate complete
  records and row cell counts before inferring Word 97 depth-one table boundaries.
- TDefTable is valid beyond Word 97; modern outside-table list validation requires explicit
  depth-one row marks and matching physical cell counts.
- Nested table row marks can carry both PFInnerTableCell and PFInnerTtp; give row ownership
  precedence and bound expanded cell definitions across the document.
- PIpgp and PFLocked preserve physical table membership; retain grouping/frame uncertainty
  separately and keep invalid context values local to their paragraph.
- Apply table insertion and merge modifiers in source order when validating physical row counts;
  unqualified nested or merged members block their list family, and admitted cell paragraphs
  require typed empty-text style and exact physical ownership excluding row marks.
- Table layout properties can preserve outside-table numbering only after table-style numbering,
  visibility and conditional formatting are resolved; keep the default-style guard when accepting
  table options.
- Resolve table hidden-text baselines from the physical row's ordered style selection and
  complete revision coverage; empty selected-style text groups do not imply that every table
  style is safe.
- Reuse an empty table-style text baseline through its typed physical paragraph and row ownership
  proof; hidden-text values alone do not establish capitalization defaults.
- Table-style language-only CHPX is a nonempty text contribution; retain ordered source language
  metadata and qualify visibility independently while keeping empty-text consumers guarded.
- Logical table alignment and repeat-header modifiers preserve text visibility only with valid
  operands and a complete empty selected-style text proof; conditional text groups remain
  unresolved.
- Preferred table-part indentation preserves visibility only with valid unit-specific widths and
  a complete selected-style text proof; Nil width bytes are ignored according to the source
  grammar.
- Table anchor and wrapping-distance operands can preserve character-style neutrality after
  scalar validation; keep selected-style, physical ownership and revision proofs, and resolve
  page bounds and overlap separately in layout.
- Qualify table-look flags only after selected-style contributions are known; historical
  auto-format indexes do not replace saved row/cell formatting or resolve unsupported conditional
  styles.
- Keep physical table widths distinct from preferences across style selection; require definite
  later cell assignments to resolve uncertain width resets, and preserve unknown Nil semantics
  explicitly.
- Autofit keeps cached cell edges separate from preferred widths; preserve table width and autofit
  across TIstd, and ignore cell no-wrap when the preferred cell width is absolute.
- Modern cell shading arrays replace the first 22 cells, including omitted unshaded tails;
  validate preferred table indentation against actual laid-out widths and preserve exact Nil
  border sentinels.
- Multi-line table borders require effective neighboring-edge agreement across physical cell and
  row spans until conflict resolution is supported; overlapping incompatible edges can fill the
  gaps between stripes.
- For Word 2000 tables, keep explicit default padding across grid replacements, preserve
  physical/preferred widths separately, and distinguish inheriting None borders from suppressing
  Nil before validating painted edges.
- For table rendering, retain physical cell and row-mark ownership; replay each cell at its
  resolved width and validate source-version padding before applying the shared model's
  table-indent convention.
- Public table-ledger validation must charge recorded style replay against the document-wide work
  budget before decoding or cloning each row.

## Revisions, visibility and comments

- Paragraph and table revision annotations preserve provenance; accepting their current
  properties still requires explicit history-wall, visibility, style-origin and table-membership
  guards.
- Compute textbox projected-byte ranges after hidden-text suppression; retain the accepted-source
  transcription separately so visibility changes remain auditable.
- Comment reference CPs and annotation bookmark limits are distinct source positions; preserve
  revision-deleted references with their property evidence.

## Footnotes and endnotes

- Bind footnote references to exact body section intervals, retaining note-free sections and
  their ordered settings; mixed section settings require per-note context rather than a
  document-wide position assumption.
- Partition footnote and endnote labels by their exact body-reference CPs; retain coexisting story
  relationships and reject overlapping reference ownership.
- A raw U+0002 is a note-marker candidate only after its character properties are considered;
  skip proved ordinary text while retaining unresolved or revision-sensitive candidates in
  ambiguity checks.
- Automatic body-reference numbering and note-body marker presence are separate evidence; retain
  absent markers explicitly and preserve authored note prefixes without matching or synthesizing
  digits.
- Custom footnote references retain their source code unit and complete authored text; a PLC
  reference anchors ownership without establishing the length or appearance of a custom marker.
- Share exact section-operand grammar between ownership consumers; qualifying outline definitions
  for note numbering preserves all section intervals and ordered properties without claiming
  outline rendering.

## Sections, headers and document settings

- Header/footer ownership distinguishes empty inherited slots from explicit blank stories; retain
  guard characters and the undefined final PLC value separately, and keep physical section
  ownership distinct from rendered page instances.
- Decode SBOrientation as the one-based portrait/landscape enumeration while retaining
  independently stored page width and height.
- Qualify document layout settings and source ownership separately from section geometry; modern
  margins require explicit source values, and inactive grid settings retain provenance without
  enabling grid layout.
- Keep DOP compatibility options separate from feature-restriction masks; FIB language
  identifiers can be normalized and cannot establish installation-dependent defaults by
  themselves.

## Pictures

- Resolve pictures through ordered CFSpec, CFData and CPicLocation properties; floating OfficeArt
  delay offsets use WordDocument, and duplicate shape IDs leave anchor ownership unresolved.

## Rendering qualification and layout

- DOC rendering qualification must retain paragraph-mark CP ownership and reject missing font
  styles, text/order mismatches or clipping; a family-name match and successful pagination alone
  do not prove source-faithful output.
- Retain hidden paragraph-mark CPs in the renderer ledger and prove zero flow separately; hidden
  nonempty or table paragraphs need their own topology semantics, and table line-height
  compatibility depends on the resolved document grid.
- Header rendering must account for slot guards, repeated page instances and blank/tab line
  metrics; retain consumed source spaces and qualify active ANSI kerning against actual font
  shaping before export.
- Leading source spaces may have no paint item while retaining advance; qualify them with exact
  CP ownership, selected-font no-ink metrics and the actual continuation-line origin, preserving
  logical text and indentation evidence.
- Manual line breaks use the same non-ASCII font slot in source qualification and layout; bind
  replay byte ranges to original CPs and admit an empty range only for the final line after a
  trailing break.
- Qualify added punctuation through the source non-ASCII font route and actual output text
  accounting; directional controls, special spaces and discretionary breaks need separate layout
  semantics.
- Resolve paragraph automatic-spacing switches after ordered style and direct properties;
  disabled switches retain explicit distances, while active states require separate layout-policy
  qualification.
