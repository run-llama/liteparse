# Native DOCX fidelity

Read the section for the area you are changing; the crate-wide rules are in
[`AGENTS.md`](./AGENTS.md).

## Package parts and imports

- Classify native Word OOXML by leading ZIP magic and the root officeDocument relationship
  resolving to its Word main part; `zipfile.is_zipfile` can recognize an embedded ZIP inside an
  OLE container.
- Alternate-content imports resolve internal relationships and content types before strict
  decoding; preserve host paragraph/cell ownership and distinguish unsupported, invalid, decoding
  and resource omissions in warnings without fetching external targets.
- Determine imported host-table eligibility from materialized active content and completion
  status; retain original source ownership and keep incomplete imports, images, nested tables and
  blank rows out of text-only replacement.
- Bound CSS selector callback traversal and comparison work, including ancestor backtracking;
  exhaustion must abort the complete import rather than silently change selector matches.

## Fonts, symbols and shaping

- Register embedded font styles before layout using font-table-relative relationships and
  canonical XML attribute names; validate GUID decoding independently and preserve document
  isolation, installed-family precedence, and nonfatal diagnostics.
- Explicit symbols keep source font/code separate from exported Unicode; verify exact-family
  glyph selection, symbol-cmap alias boundaries, visible fallback, and repeated-symbol source
  offsets when changing shaping.
- Consume symbol provenance on shaped atoms while retaining it on source inlines for reflow;
  compare raw exported spaces and nonbreaking spaces because equivalent page images and
  normalized text can hide run-boundary losses.
- Whitespace-only anchor slots need explicit source-order retention; validate coincident slots,
  empty-string exclusion, and unchanged glyph paint separately from mixed-font text.
- Validate hyphenation split selection against saved Word line endings; extra legal dictionary
  points do not establish a better Word match. Resolve supported language regions through
  optional script subtags and stop before extension/private-use subtags.

## Bidirectional text

- Bidi placement must preserve logical cluster text, symbol-font Unicode translation, and source
  byte mappings across caps/symbol expansion; compute run and decoration bounds from visual
  positions.
- Apply Unicode L1 resets using original character classes when directional analysis substitutes
  strong marks for explicit RTL runs.
- Resolve logical side indents to physical sides after paragraph direction inheritance; preserve
  negative values and explicit zero overrides.
- Validate RTL first-line bounds and numbering anchors together after justification, including
  suffixes, empty lists, mirrored glyphs, symbol mappings, and exact-prefix continuation while
  preserving logical source text.
- RTL trailing-tab repair must reset both visual advances and the visible alignment extent;
  checking only page visibility can hide a large shift in body ink.

## Tabs and character grid

- Literal plain tabs use surrounding text or paragraph-mark line metrics; preserve generated
  numbering suffixes and leader/decorated-tab behavior, and verify larger marks, visible text and
  anchored-picture ownership when changing metric participation.
- Character-grid tracking must refresh section context, preserve signed charSpace/4096 and
  inherited run snapToGrid, and keep snapToChars unit alignment distinct from additive tracking.
- Character-grid compatibility for unsigned 32-bit spellings must preserve signed inputs, bound
  the recovered integer range, and validate converted-source text independently of cached Word
  page counts.

## Colors, shading and borders

- Theme run colors cascade as one property with their transforms; preserve already-transformed
  cached RGB for system/unresolved-theme fallback, and quantize only resolved HSL transforms.
- Underline pattern and color inherit independently; only explicit color/themeColor replaces
  inherited underline color, and tint/shade-only attributes leave it intact.
- Retain shading source presence through inheritance, resolve both theme channels before pattern
  mixing, and distinguish an omitted element from an explicit clear/automatic fill; validate nil
  separately in each style context.
- Resolve border theme paint across every supported border projection while retaining color
  source identity for grouping; equal resolved RGB alone does not establish identical border
  definitions.

## Lists

- Apply hierarchical list start overrides in ascending level order: each ancestor resets deeper
  counters, so unordered traversal changes labels between identical renders.
- Resolve picture-numbering assets from their numbering part, validate with the caller image
  limit before replacing literal label characters, and update accepted merge-fragment
  eligibility; picture sizing is a qualified font-relative approximation, including retained
  outline geometry.

## Tables

- Word ignores nil top/bottom cell-margin exceptions, preserving inherited padding; explicit dxa
  zero clears it (README, "Nil vertical cell margins").
- Word adds the largest resolved bottom cell padding to positive exact row heights; validate this
  separately from automatic and minimum row sizing (README, "Exact row height and bottom
  padding").
- Automatic/minimum table rows share their largest resolved vertical margins; carry the owning
  pair through rotated merges and split fragments, while keeping exact-height sizing separate.
- Terminal cell paragraph spacing belongs to the fragment carrying its final line; distinguish it
  from minimum row height and preserve shared note/frame slice heights when changing table split
  eligibility.
- Cell stacking flattens paragraph pagination properties; preserve supported row-start page
  boundaries before stacking, with explicit scope for nested and floating tables.
- Retain nested rows in local coordinates through recursive cuts, pass full fresh-area capacity
  separately from remaining room for cantSplit, and resolve cell picture wrapping and painting
  from the same reflowed anchor owner.
- Body-row progress fallbacks must retain the unconsumed tail, reset stalled retry limits after
  actual source or height-debt progress, and avoid empty nested continuations; test exported text
  and physical bounds beyond 200 fragments.
- Word table and row-exception shading applies only to cell spacing; qualify actual gap geometry
  before painting, and verify zero-spacing cell interiors, automatic text color and visible
  overflow independently of extracted text.
- AutoFit measurements must share paragraph shaping, cell break and tab semantics; solve
  spanning-cell minima jointly on one grid, preserve source growth settings across stories, and
  verify painted image clearance independently of text inventory.
- Nested intrinsic widths share final grid normalization; rotated bounds use the transverse axis
  and resolved exact-row capacity, while legacy percentage bases include only the current table's
  outer margins (README, percentage table grids).
- Retain inline raster clipping as owning-paragraph source policy through cell/merge
  remeasurement; relax only raster width demand while preserving glyph, padding and fixed-child
  floors, and verify clip bounds after narrowing and widening.
- Auto table cell preferences negotiate within the saved-grid/available-width envelope; preserve
  its partition when full, and keep explicit table widths separate from content-driven growth
  limits.
- Resolve source noWrap before width negotiation: dxa preferences have priority over breakable
  text, Auto/percentage use explicit-line extents, and Word conditional noWrap applies row-wide
  before direct cell overrides.
- Proportional AutoFit redistribution uses immutable preferred-width weights for both interval and
  total projections; preserve joint span minima, noWrap priority and hard object floors, including
  zero-width source columns.
- Validate cumulative table grid endpoints and shared document work before layout or merge
  provenance; retain exact valid spans and report resource failure explicitly, including numeric
  overflow and isolated note stories.
- Resolve numeric body floating-table anchors at each accepted page/column destination; preserve
  local cell and merge coordinates through retries, repeated headers, and merged-fragment paint.
- Resolve floating-table vertical alignment against its anchor before layout: text anchors ignore
  tblpYSpec and retain numeric/default offsets; test page/margin precedence and subsequent flow
  independently.

## Vertical merges and merged-cell continuation

- Vertical-merge border experiments must retain original fragment edges and match both grid start
  and span; saved single-page truth does not validate page/column-break edge semantics.
- Vertical-merge continuation must carry the starting cell's margins and distinguish columns as
  well as pages; top-merge anchors and footnotes need explicit placement ownership before moving
  their slices.
- Repeated table headers have independent merge owners per copy; nested merged content needs
  outer-slice continuation metadata before changing its owning row.
- Merged continuation items must paint after their owning row's cell shading and before its
  perimeter strokes; JSONL text presence alone does not establish visible raster content.
- Merged-cell continuation capacity must be allocated from whole source slices during pagination,
  including per-fragment margins and line-fitting slack; consuming slices resets retry guards,
  and paint plans belong to appended placements so trial rollback remains safe.
- When spreading merged-cell paint, transfer anchors with the same source-slice transform as text,
  including anchor-only slices; retain per-slice reading order independently of floating-object
  paint order and leave note reservation changes to pagination.
- Allocate body merge references from the accepted physical slice prefix and commit its cursor
  and note plan atomically; a fresh splittable row is not an indivisible first line, and
  simultaneous merges retain independent terminal height demand.
- Fit moved local anchors against declared and rendered occupied bounds independently of text
  wrapping; distinguish an occupied-bound rejection from a short merged row awaiting cumulative
  height, and allow intrinsic overflow only at the actual fresh destination origin.

## Pagination and columns

- Pagination rollback must restore the surviving PageInfo as well as truncating appended pages;
  assert both accepted reference metadata and rendered note counts after keep-chain retries.
- Preserve the unconditional minimum page-body capacity when changing header/footer limits;
  oversized reservations need an independent content-preservation regression.
- Column-balance trials must restore starting page note/carry metadata, reserve pending notes
  before clamping the trial height, and validate actual normal-flow extent across columns;
  floating-table cursors have separate flow semantics.
- Leading column-control extent experiments must distinguish inkless transition lines from real
  blank paragraphs, preserve spacing and decoration, and preserve balanced-section placement.
- Retain page-wide floating-object exclusions across column transitions, filter them at the
  destination, and preserve freshly recomputed tail clearance when replaying consumed paragraph
  prefixes; reset stale source-column clearance.
- Whole-paragraph retries must recompute destination float clearance; continuous columns starting
  below the physical page top must advance before applying the oversized-first-line progress
  fallback.
- Automatic before-spacing at an unforced physical-page top needs separate controls from explicit
  spacing, forced breaks, column transitions, and inherited spacing overrides.
- Modern forced-page automatic spacing uses pending section-content state, independent of
  header/footer first-page classification; preserve it across forced retries and parity padding,
  clear it on placed lines or rows, and test automatic overflow separately.

## Footnotes and endnotes

- Endnote placement comes from document settings; section-level `endnotePr/pos` is ignored
  (README, "Section-aware endnotes").
- Treat explicit `w:type="normal"` and an omitted type equivalently for footnote/endnote
  definitions; test note text and numbering while retaining special-type and unknown-type
  exclusions.
- Retain special-note XML with its source part and settings selection before layout;
  selected-empty stories and duplicate definitions must survive without invoking mutable
  numbering or graphics parsers.
- Custom endnote references retain body registration and source section/order while suppressing
  automatic numbering; preserve literal markers without guessing a following-text marker
  boundary.
- Final continuous-section note synchronization copies only note IDs and cut/carry state;
  preserve the output page's original section, header selection and geometry.
- Record footnote ownership and body geometry from accepted placements after balancing; keep
  source section/body column separate from header ownership and footnote-area column policy.
- Footnote reservation, separator sizing and drawing must share captured area width/grid context;
  preserve first accepted reference ownership and global note order across areas, and intersect
  picture clearance with each area's horizontal span.
- Footnote continuation trials must start from immutable page-entry cursors, preserve accepted
  note reservations, and commit the exact accepted body prefix once; test later references,
  incoming pictures, manual boundaries, and intentional parity blanks together.
- Footnote destination areas must preserve global reference order, intersect trial pictures by
  horizontal span, and forward page-break queues through unused columns; source ownership and
  destination geometry are separate.
- Note merged-cell trials clone both row and allocation history, key deferred paint by source
  occurrence and destination, and reserve initial/repeated headers with data against fresh
  capacity; source slice keys keep late anchor text beside its owning line.
- Retained row text and anchors must share cell/slice/anchor source keys with checked traversal
  cardinality; preserve source subslots through note decoding and keep ordinary fragment ordering
  scoped to each note occurrence.
- Whole-note painting must retain accepted slice anchors with recursive source order and distinct
  paragraph/line origins; keep origin deltas translation-invariant through nested-row cuts and
  preserve existing carry/cut selection.
- Note picture reservations track occupied extent separately from text advance, including
  note-cell slices; recompute accepted occupied height after rollback and keep source paragraph
  origins consistent between whole and fragmented drawing.
- Occupied note slices can extend below later side-flow text: use the maximum accepted bottom for
  row and merge fragments, and reserve only physical fresh-area capacity for a diagnosed oversized
  atom while retaining its owner and source extent.
- Selected continuation stories use physical-page occurrence ownership and the same accepted
  layout for reservation and paint; preserve selected-empty paragraph geometry, internal spacing,
  and finite special-story carry without recursively generating stories.
- Unequal-width note paragraphs resume from logical source boundaries, including non-text atoms
  and transformed hyphen ranges; preserve accepted prefix measures, destination-fragment anchor
  origins, and deferred inline textbox payloads through every reflow pass.
- Whole notes, retained rows and fragmented paragraphs share inline text/anchor source slots; keep
  floating paint order separate and reinsert deferred inline textbox text at its original source
  position.
- Note-cell remeasurement preserves in-cell context, cross-paragraph wrapping and remaining
  height debt; reserve fixed-grid occupied spans, and validate percentage resizing in both
  directions across ordinary and selected-story notes.
- Merged note history matches logical grid intervals and retains each accepted cell width and
  fresh capacity; recursive child paint keeps source descriptors through outer merges and scopes
  final backfill to the root note occurrence; coalesce physically contiguous cell geometry
  independently of fresh-capacity metadata to charge merge padding once.
- Beneath-text note origins use final accepted body extents and captured reference ownership;
  sweep the full note rectangle through foreground exclusions, and keep source body-column counts
  separate from destination columns.

## Frames, shapes and drawings

- Keep scoped page-frame inheritance separate from legacy frame state so enabling body-frame
  placement does not change header/footer or table-cell flow.
- Injected frame anchors must survive paragraph reflow and section tails; page counts and
  first-page checks alone do not establish text preservation.
- Footer frame placement must preserve nested anchors and local indents; distinguish paragraph
  frames from ordinary drawings when bounding numeric page-relative origins.
- Frame line visibility needs both advance and glyph-ink bounds at final frame width; preserve
  uncertain bounds, crossing lines, line height and paragraph spacing when omitting wholly outside
  text.
- Drop-cap line counts refer to the following non-frame paragraph's wrapped lines; preserve cap
  reading order and distinguish inside/outside-margin placement (README, "Drop-cap paragraph
  pairs").
- Preserve delegated graphics warnings through layout and index output, with source-part and
  relationship context and deterministic deduplication.
- Deferred group graphics must retain empty-group reachability, source paint order, and composed
  child/ancestor transforms until fonts are available.
- Wrapped shape autofit shares resized geometry and anchor height with body exclusion; verify
  visible fill, complete text and following-body clearance, and distinguish explicit
  sibling-anchor collisions from ordinary flow overlap.
- Explicit no-wrap shape autofit shares its unwrapped text projection for sizing and paint; retain
  paragraph/hard-break height, saved horizontal extent and foreground exclusions independently of
  unresolved page-width limits.
- Upright shape text resolves its frame only after composing child and ancestor placement; retain
  shape paint, transformed center, source writing direction and false/omitted behavior
  independently of text orientation.
- Inline rotation layout and original paint geometry share one rebasing transform for shape items,
  deferred text and pending graphics; keep source effect sides and anchor positioning distinct
  from inline allocation.
- Text-box picture anchors retain deferred page references and local transforms through
  projection and reflow; generated inline owners must stay separate from source anchor ordinals
  and add no flow extent.
- Retain direct-chart provenance through graphics rendering before applying picture-fit retries;
  grouped shapes and text frames keep their own policies, and chart retries preserve source
  offsets and note ownership.

## Pictures

- Picture fitting must reserve pending and later footnotes, distinguish fresh-page progress from
  later-column retries, and preserve source positions already outside the first-column page
  extent.
- Behind-text side-wrapped decoded pictures retain background paint order while sharing
  foreground page/note clearance; test disjoint and spanning note areas, and distinguish decoded
  image content from its original raster or EMF format.
- Picture-fitting validation must compare drawn image parts, page ownership and geometry alongside
  text output; text-only JSONL can remain identical while a picture moves or clips.

## Fields, bookmarks, links and math

- Field-link tokenization must retain quoted/empty arguments and decode only escaped quotes and
  backslashes before assigning source-owned targets.
- Reference-field navigation must distinguish actual switches from quoted or format arguments,
  preserve cached results, and retain enclosing link ownership through nested result fields.
- Carry bookmark boundaries independently of shaping and flow extent; resolve global source
  identities from accepted placements, preserve exact exported text-item boundaries, and retain
  first/final table edges through splits and note remeasurement.
- Test explicit kerning and tracking across internal-link boundaries in plain paragraphs and
  tables; retain original pre-link style identity for shaping, with logical ownership spans inside
  shared painted glyph clusters.
- Keep retained shaping identity out of formatting comparisons; internal-link attribution must
  preserve original fitText and hyphenation groups, including ownership and bookmark offsets after
  reshaping.
- Direct block OMML shares the DOCX accepted-revision linearizer with inline math; retain direct
  math runs and source justification while distinguishing linear text preservation from
  two-dimensional equation layout.

## Hidden content, export and attribution

- Fully hidden non-cell paragraphs retain bookmark ownership with zero flow and no decoration;
  preserve visible content, visible paragraph marks and required cell marks independently.
- Text deduplication must preserve distinct logical fragments of one condensed painted run;
  consume transient paint identity before export and test true overlay copies separately.
- Off-page text diagnostics must match the final exporter’s transformed-boundary predicate, keep
  exclusions explicit, and report counts without source text; internal run retention alone does not
  prove exported content preservation.
- Physical-page clipping diagnostics include source-owned headers and footers; generated reading
  keys alone cannot distinguish document text from line numbers.
- Native worker attribution uses source-owned link/cell/paragraph metadata and page-local emitted
  content IDs; preserve ownership during run coalescing, and gate text-table HTML substitution on
  source eligibility so images, nested tables and blank rows remain in native page output.
- Vertical-merge HTML spans follow accepted physical row fragments while retaining declared cell
  ownership; preserve independent logical-grid chains, page-local text and ordinary empty cells
  through merge backfill and repeated headers.
- Preserve omitted row grid columns separately from physical cell indices in source attribution;
  HTML gap cells must respect incoming row spans and page-local merge fragments.
