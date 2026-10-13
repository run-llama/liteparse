# liteparse changes — review guide

This is an uncommitted working tree on top of upstream `main`
(`run-llama/liteparse`, commit `b754dc3`, 2026-09-22). Nothing is committed —
that's deliberate, so you can review the diff yourself before deciding how to
split it into commits/PRs.

```
git log -1        # shows you're on upstream's last commit
git status        # shows the changed/new files
git diff          # the full diff
```

## What changed, file by file

| File | What | Why |
|---|---|---|
| `crates/liteparse/src/markdown_layout/table_spans.rs` | **New.** Detects merged (row/column-spanning) cells in a ruled table from *missing* rule segments, and repeat-fills the merged label into every cell it covers. | A merged cell is drawn once, wherever the generator centred it. Without this, the label lands in one row/column and the rest of the span is blank. Uses the table's own drawn rules as evidence — no heuristics on text. |
| `crates/liteparse/src/markdown_layout/table_header.rs` | **New.** For a ruled table with no header row (because the header itself has no drawn rules — common for multi-level headers), walks upward from the table body through the preceding text lines and absorbs the ones that look like column headers, resolving column-spans by which body columns a header cell's x-range covers. | Some tables draw a ruled body but leave the header borderless. Without this, the header text is emitted as loose paragraph/heading lines above an unlabeled table. |
| `crates/liteparse/src/markdown_layout/tables.rs` | Hooks the two modules above into the existing ruled-table build path; adds `detect_table_grids` (a row-edge-exposing sibling of the existing `detect_table_rects`, reusing its exact page-background-fill safety check); a small fix to `merge_continuation_rows` so a repeated row-span cell isn't glued onto the row below it a second time; **(follow-up PR)** `split_component_at_grid_gaps`'s gap-vs-pitch threshold is now judged against a *local* pitch window instead of the whole component's median, and both `detect_table_grids`/`detect_table_rects` now route through it via a shared `table_grid_bands` helper instead of reporting one unsplit bbox per connected grid. | Wiring + one existing-function bugfix, plus (follow-up) fixing a table-fusion bug that was silently dropping data across the whole codebase, not just in rotated-text documents — see "Follow-up fix" below. |
| `crates/liteparse/src/markdown_layout/repetition.rs` | The running header/footer stripper no longer removes a line that sits inside a drawn table grid, even if that line repeats across pages (e.g. a table header repeated on a multi-page table). | Previously a table's own header could be mistaken for page chrome and deleted. |
| `crates/liteparse/src/projection.rs` | Rotated (90°/270°) text sitting inside a drawn table is kept as upright cell content (box width/height swapped around its own centre) instead of being silently dropped by the existing "skip rotated sidebar/watermark text" rule, gated by `detect_table_grids` so it only fires inside a real, validated table; **(follow-up PR)** the swapped box is now also snapped to the height of the drawn row it physically sits in, using the row-boundary list `detect_table_grids` already computed but that this function wasn't yet using. | Vertical table headers/labels were previously invisible in the output. The follow-up row-snap fixes the one case that wasn't: a single row of rotated column headers — see below. |
| `Cargo.lock` | Not changed in this tree (reverted before packaging) — building will touch it locally; don't commit it as part of this diff. | |

## Known-good vs. known-open

**Verified, with a 13-PDF regression sweep (word-for-word diff, zero words
lost anywhere) against unmodified `main`:**
- Merged/spanning cells (row-span, column-span) on ruled tables — fixed.
- Borderless multi-level table headers — fixed.
- Table headers wrongly stripped as running page chrome — fixed.
- Rotated *row-group* labels spanning multiple rows (e.g. a sideways label
  next to 3 grouped rows) — fixed.
- A background-fill-covered whole page being misread as a giant table (a real
  regression hit and fixed during development — see `experiments/`) — fixed.
- **(follow-up PR)** Rotated *column headers* (a single row of sideways header
  text) — fixed. See "Follow-up fix" below; the original diagnosis above
  ("a different code path... claims that line") turned out to be a red
  herring for a deeper bug.
- **(follow-up PR)** Two ruled tables stacked in the same column with
  matching x-positions (e.g. a template that reuses column widths) getting
  fused into one bogus table and silently dropping data — fixed. Not
  rotation-specific; confirmed against the Apple 10-K fixture below.

**`experiments/`** (if present in this zip) holds reverted attempts and notes
on why each was reverted — useful context for anyone picking this up.

## Follow-up fix: table fusion + the last rotated-header case (this PR)

Picking this diff back up with two fresh test fixtures (`demo/docs/test_liteparse_hard.pdf`
for the merge/header work above, `demo/docs/test_liteparse_vertical.pdf` — two
small tables, "V1" with rotated column headers and "V2" with a rotated
row-group label spanning 3 rows each) surfaced that "V1" — the case this
diff's own README called out as still open — was in fact a symptom of a
second, unrelated, and more consequential bug.

### Bug 1 (the real one): stacked tables with matching columns fuse into one

`find_grid_components` groups a page's H/V rule segments into "one component
per table" using `cluster_v_segments`, which merges any two vertical rule
segments that share an x-coordinate — with **no cap on how far apart they are
in y**. If two separate ruled tables happen to use the same column widths
(exactly what `test_liteparse_vertical.pdf`'s V1/V2 do, and — it turns out —
extremely common in real filings that reuse one column template across many
tables, e.g. Apple's 10-K), their column rules get welded into one component
spanning from the top of the first table to the bottom of the second, with
whatever sits in the gap (a heading, in this case "V2. Vertical merged
row-group labels") folded inside it.

There's already a safety net for exactly this — `split_component_at_grid_gaps`,
whose own doc comment describes this scenario — but its "is this gap big
enough to be a real break, not just an unruled full-span row" heuristic
compares the gap to the **global median row pitch of the whole (already
fused) component**. Blending two tables with different row heights skews that
median, so a real 58pt inter-table gap needed to clear a 75pt threshold
(2.5× a 30pt blended-median pitch) and narrowly didn't. Fix: compute that
threshold from a small **local** window of pitches around each candidate gap
instead of the whole component's median, so a different row cadence
elsewhere in a fused component can't inflate the bar for judging *this* gap.

The intuition for why this mattered so much more broadly than "vertical
text": once two tables fuse, every column from one table gets unioned with
every column from the other, so the merged grid's empty-cell fraction
explodes (~30–74% in the traces) and gets rejected by the existing quality
filter — meaning ruled-table detection gives up on the region *entirely* and
falls back to a much weaker text-position heuristic. Verifying against the
Apple 10-K (already a fixture for the merge/header work) showed this
recovered several real tables that were silently rendering with entire
columns of numbers missing — a correctness bug independent of rotation.

`detect_table_grids` and `detect_table_rects` (both feed the XY-cut layout
pass, one for "is this rotated text inside a table" and one for "don't slice
a V-cut through a table's column gutters") had the *same* fusion bug and were
routed through the same fix via a new shared `table_grid_bands` helper — this
mattered because leaving them unfixed while fixing `split_component_at_grid_gaps`
elsewhere would have let one path assume the fused-but-still-wrong bbox while
another assumed the corrected, split ones, which is worse than either being
consistently wrong.

### Bug 2: a rotated header row's words can look like a separate "banner"

With bug 1 fixed, V1 progressed from a shredded mess to: table renders with
correct rows, but the header row is blank and its (correctly individually
upright-swapped) words sit as loose text above the table. `keep_rotated_table_text_in_place`
swaps each rotated item's box (width/height around its own centre) so it
reads upright — correct — but two header words sharing the *same physical
ruled row* can end up with different post-swap heights (e.g. a two-line
label like "Revenue" over "(USD" over "m)" next to a plain one-line
"Headcount"), leaving a visual gap between the shorter one and the row below
it that doesn't reflect the real ruled cell they're both actually inside.
The XY-cut layout pass's "banner cut" heuristic — looking for an isolated,
full-width band of text with generous clearance above and below, meant for
document titles sitting over a two-column body — mistook that artificial gap
for a real one and split the header out of the table's own region before
table-binding ever got to look at it.

Fix: `keep_rotated_table_text_in_place` now also snaps the swapped box's
vertical extent to the row band it falls in, using the row-boundary list
`detect_table_grids` already returns but this function had never consumed.
Two words in the same row now report the same, correct row height, so
there's no artificial gap for the banner heuristic to misread.

### A fix that looked right and wasn't

The first attempt at bug 2 was to make the banner-cut heuristic itself
obstacle-aware (skip a candidate cut that passes through a detected table's
bbox) — the same defense the density-based cut already has. It fixed V1. It
also broke the Apple 10-K badly: several genuinely correct banner cuts that
happen to pass through a table's outer bbox (e.g. separating a borderless
multi-level header band from the ruled body below it, which is *exactly* how
several of that document's tables are laid out) got suppressed, undoing most
of bug 1's improvement. Reverted in favour of the row-snap above, which fixes
the actual geometric cause instead of papering over its effect on one
heuristic. Kept here as a note so nobody re-tries the same shortcut.

### Verification

- `test_liteparse_hard.pdf` — byte-identical output before/after.
- `test_liteparse_vertical.pdf` — both V1 and V2 now render as correct
  tables with correct headers/row-group labels, in the correct word order.
- Apple 10-K — diffed against unmodified `main`: no data lost, several
  previously-broken tables (missing columns of numbers, headers detached
  from their tables) now render correctly. One minor, unrelated cosmetic
  regression was checked for and not found in the final version (it *was*
  present in the reverted obstacle-aware banner-cut attempt above).
- `cargo test -p liteparse --no-default-features --lib`: 384/385 passing
  (the one failure needs LibreOffice installed, unrelated to this change).
- `cargo fmt --all --check`: clean.

## Test fixtures used during development

Not part of the diff, but referenced in commit messages / discussion if you
add them as fixtures:
- A synthetic "hard mode" PDF stress-testing block merges, staircase merges,
  borderless multi-level headers, and a row-span table split across a page
  break.
- A synthetic vertical-text PDF (rotated column headers + rotated
  multi-row-spanning labels).
- Apple's FY2024 10-K (a real, large, table-heavy filing) — used as the
  primary real-world regression check.

Ask me if you want these regenerated; the generator scripts are Python +
ReportLab and are quick to rebuild.

---

## How to build and run (this is a Rust project)

### 1. Toolchain

The project's `Cargo.toml` uses **edition 2024**, which needs **Rust 1.91+**.
Check what you have:

```bash
rustc --version
```

If it's older, install/upgrade via [rustup](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.dev | sh
rustup update stable
```

(On a stock Ubuntu box, `apt install cargo rustc` typically gives you
something far too old — 1.75 — which will fail to compile. Use rustup, or an
Ubuntu backport/PPA that ships 1.91+, or your OS's newest available package.)

You'll also need:
- `pkg-config` and `libssl-dev` (OpenSSL headers) — `openssl-sys` needs these.
- `git`.

### 2. PDFium

The build downloads a prebuilt PDFium binary automatically via a build
script. If your network can reach GitHub releases directly, you don't need to
do anything — just build normally (step 3).

If your environment blocks that download (as ours did, behind a restrictive
proxy), fetch it yourself and point the build at it:

```bash
mkdir -p /tmp/pdfium && cd /tmp/pdfium
curl -sSL -o pdfium.tgz \
  "https://github.com/run-llama/pdfium-binaries/releases/download/chromium%2F8058/pdfium-linux-x64.tgz"
  # (use pdfium-mac-x64.tgz / pdfium-mac-arm64.tgz / pdfium-win-x64.tgz as appropriate)
tar xzf pdfium.tgz

export PDFIUM_LIB_PATH=/tmp/pdfium/lib
export PDFIUM_INCLUDE_PATH=/tmp/pdfium/include
export LD_LIBRARY_PATH=/tmp/pdfium/lib:$LD_LIBRARY_PATH   # needed at RUN time too, not just build time
```

Keep the `PDFIUM_*` build-time vars and the `LD_LIBRARY_PATH` run-time var
set in the same shell for both building and running.

### 3. Build

The default build includes an embedded Tesseract OCR engine, which is a slow,
heavy compile and needs its own toolchain (cmake, a C++ compiler, etc.) that
may not be present everywhere. If you just want to build and test the changes
in this diff (none of which touch OCR), skip it:

```bash
cd liteparse-main
cargo build --release -p liteparse --no-default-features
```

If you do want OCR and have the toolchain for it, drop
`--no-default-features`.

First build will take several minutes (many dependencies); it's much faster
on subsequent builds.

### 4. Run the CLI

```bash
./target/release/lit --version

./target/release/lit parse path/to/file.pdf \
  --format markdown --no-ocr -q -o output.md
```

Useful flags:
- `--no-ocr` — skip OCR entirely (required if you built with
  `--no-default-features`).
- `--keep-headers-footers` — don't strip repeated running headers/footers
  (useful for seeing a table's own header before/without the repetition-filter
  fix).
- `--format json` — structured output (blocks, cells, bboxes) instead of
  markdown.

### 5. Debug logging

Several `LITEPARSE_DEBUG_*` environment variables print detailed traces to
stderr — handy for seeing *why* a particular line/table/merge was or wasn't
picked up:

```bash
LITEPARSE_DEBUG_MD=1 LITEPARSE_DEBUG_TABLE=1 LITEPARSE_DEBUG_RULED=1 \
  ./target/release/lit parse file.pdf --format markdown --no-ocr -q -o out.md 2> trace.log
```

(`LITEPARSE_DEBUG_LINES=1` also exists, for tracing `form_lines`'s
line-grouping decisions specifically — not something I added, it was already
in the codebase.)

### 6. Run the test suite

```bash
cargo test -p liteparse --no-default-features --lib markdown_layout
```

This runs the unit tests for the module this diff touches, including the new
ones added alongside the changes above. Drop the `--lib markdown_layout`
filter to run the full crate's test suite (slower).

### 7. Formatting / lint

```bash
cargo fmt --all --check     # should report no diff
cargo clippy -p liteparse --no-default-features --all-targets -- -D warnings
```

I was not able to get `cargo clippy` to complete cleanly in the sandboxed
environment this work was done in (it failed inside the `pdfium-sys` binding
crate, which this diff doesn't touch) — worth re-running clippy on your own
machine before treating it as passing.
