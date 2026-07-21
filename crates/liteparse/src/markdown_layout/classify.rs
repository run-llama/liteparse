use crate::types::{OutlineTarget, ParsedPage, ProjectedLine};

use super::blocks::{Block, paragraph_from_accum};
use super::headings::{
    HEADING_MAX_TEXT_CHARS, MAX_HEADING_LEVELS, heading_level_for, heading_size_of,
    is_caption_line, is_toc_title, looks_like_bold_heading, looks_like_numbered_bold_heading,
    outline_heading_level, page_is_toc, struct_heading_level,
};
use super::hr::detect_horizontal_rules;
use super::inline::{
    append_inline_continuation, line_uniform_style, render_line_inline, render_list_item_text,
};
use super::lists::{
    LIST_INDENT_STEP_PT, detect_ordered_list_lines, parse_list_marker,
    split_ordered_marker_for_emit,
};
use super::paragraphs::{
    ParaAccum, append_to_paragraph, collapse_whitespace, continues_heading, continues_list_item,
    continues_paragraph, ends_hyphenated, ends_sentence_final, is_soft_hyphen_break,
};
use super::repetition::is_header_or_footer;
use super::tables::{detect_ruled_tables, detect_tables, merge_table_runs};

/// A document-order page interruption that breaks the normal text flow: either
/// a horizontal rule (from vector graphics) or a figure injection (a raster
/// image ref). Collected into one y-sorted stream so the two kinds interleave
/// correctly by vertical position rather than emitting all of one before the
/// other.
#[derive(Clone)]
enum Interruption {
    Hr,
    Figure(crate::types::ImageRef),
    /// A ruled table detected by the global pass, already fully built. Emitted
    /// at its top-y position like other interruptions; its lines were pulled
    /// out of the region pipeline so it won't be re-detected per-region.
    Table(super::blocks::Block),
}

/// Returns true if any span on the line is rotated more than ~5° off
/// horizontal — used to exclude sidebar / margin-stamp text (arXiv banners,
/// watermarks, vertical legends) from the body-size and heading-size
/// histograms so it doesn't compete with normal-flow text for heading slots.
pub(super) fn is_rotated_line(line: &ProjectedLine) -> bool {
    line.spans.iter().any(|s| {
        let r = s.rotation.abs() % 360.0;
        // Anything more than ~5° off the horizontal axes is "rotated" for
        // our purposes. 0° and 180° are both horizontal text.
        !(r < 5.0 || (175.0..=185.0).contains(&r) || (355.0..=360.0).contains(&r))
    })
}

/// Classify a single page's `ProjectedLine`s into blocks.
#[cfg(test)]
pub(super) fn classify_page(page: &ParsedPage, heading_map: &[(f32, u8)]) -> Vec<Block> {
    classify_page_with_filters(
        page,
        heading_map,
        &std::collections::HashSet::new(),
        &[],
        crate::config::ImageMode::Placeholder,
        &std::collections::HashSet::new(),
    )
}

/// Same as `classify_page` but also strips lines matching a precomputed
/// running header/footer set. Use this when emitting a whole document so
/// repeating chrome (titles, page numbers) doesn't show up in every page.
///
/// `outline` is the document outline filtered to entries whose `page_index`
/// matches this page (or the full outline — out-of-page entries are
/// ignored). Heading promotion from struct tree + outline outranks the
/// font-size heading map.
pub fn classify_page_with_filters(
    page: &ParsedPage,
    heading_map: &[(f32, u8)],
    header_footer: &std::collections::HashSet<String>,
    outline: &[OutlineTarget],
    image_mode: crate::config::ImageMode,
    chrome_indices: &std::collections::HashSet<usize>,
) -> Vec<Block> {
    let debug = *super::flags::DEBUG_MD;

    // TOC suppression: when ≥3 lines on this page look like TOC entries
    // (alpha body + trailing page-number), demote heading promotion so each
    // entry stays a paragraph instead of becoming a fake H1/H2. The TOC's
    // own title ("Contents", "Table of Contents") is shorter without a
    // trailing number — it falls through the size/bold heuristics with no
    // special handling needed.
    // Fallback TOC detection: when projection truncates the trailing page
    // numbers on TOC entries (common when entries use dot-leaders that the
    // projection eats), `page_is_toc` misses. If a `Contents` / `Table of
    // Contents` title is the first text on the page, treat the rest as TOC
    // entries even without page-number tails. Guard: don't fire if the page
    // has a substantial body paragraph (≥3 lines of ≥80 chars each) — that's
    // a real content page that just happens to mention "contents" near the
    // top.
    let toc_page = page_is_toc(page) || {
        let mut saw_title = false;
        let mut long_lines = 0usize;
        for line in &page.projected_lines {
            if is_rotated_line(line) {
                continue;
            }
            let t = line.text.trim();
            if t.is_empty() {
                continue;
            }
            if !saw_title {
                if is_toc_title(t) {
                    saw_title = true;
                    continue;
                }
                if t.chars().count() > 40 {
                    break;
                }
            } else if t.chars().count() >= 80 {
                // Dot-leader fragments ("`. . . . . . . .`") are part of
                // TOC layout, not body paragraphs — skip them.
                let alpha = t.chars().filter(|c| c.is_alphabetic()).count();
                let alpha_ratio = alpha as f32 / t.chars().count() as f32;
                if alpha_ratio < 0.3 {
                    continue;
                }
                long_lines += 1;
                if long_lines >= 3 {
                    saw_title = false;
                    break;
                }
            }
        }
        saw_title
    };

    // Strip running header/footer lines up-front so they don't leak into
    // table detection (a repeating two-column footer would otherwise look
    // like a 2-row table) or paragraph grouping.
    let need_filter = !header_footer.is_empty() || !chrome_indices.is_empty();
    let filtered_owned: Vec<ProjectedLine> = if !need_filter {
        Vec::new()
    } else {
        page.projected_lines
            .iter()
            .enumerate()
            .filter(|(idx, l)| {
                !chrome_indices.contains(idx) && !is_header_or_footer(l, page, header_footer)
            })
            .map(|(_, l)| l.clone())
            .collect()
    };
    let lines: &[ProjectedLine] = if !need_filter {
        &page.projected_lines
    } else {
        &filtered_owned
    };

    // Global ruled-table pass. `detect_ruled_tables` is otherwise called per
    // xy_cut leaf, but a ruled table whose rows scatter across several leaves
    // never has its full text in any single leaf and gets rejected as
    // mostly-empty. Run it once over all lines, pull the consumed lines out of
    // the region pipeline, and emit each table as a y-positioned interruption.
    // Runs before cross-region merge so that pass (and the region indices its
    // runs carry) operate on the already-filtered line list.
    let mut global_ruled_tables: Vec<(f32, Block)> = Vec::new();
    let mut global_ruled_consumed: std::collections::HashSet<usize> =
        std::collections::HashSet::new();
    for (run, consumed) in super::tables::detect_ruled_tables_global(
        lines,
        &page.graphics,
        page.page_width,
        page.page_height,
    ) {
        // Only rescue tables the per-region path can't see whole: those
        // whose rows scatter across ≥2 xy_cut leaves. A table living in a
        // single region is already handled (often better) by per-region
        // detection — replacing it here only risks regressions.
        let mut groups: std::collections::HashMap<&Vec<u16>, Vec<usize>> =
            std::collections::HashMap::new();
        for &i in &consumed {
            groups.entry(&lines[i].region_path).or_default().push(i);
        }
        if groups.len() < 2 {
            continue;
        }
        // If any single leaf's share of these lines already forms a table
        // on its own, the per-region path handles this content — don't
        // override (a decorative frame can span a data table plus surrounding
        // prose; the data region tables cleanly by itself, while the frame
        // would fuse prose into garbage cells).
        let already_handled = groups.values().any(|idxs| {
            if idxs.len() < 2 {
                return false;
            }
            let sub: Vec<ProjectedLine> = idxs.iter().map(|&i| lines[i].clone()).collect();
            !super::tables::detect_ruled_tables(
                &sub,
                &page.graphics,
                page.page_width,
                page.page_height,
            )
            .is_empty()
                || !super::tables::detect_tables(&sub).is_empty()
        });
        if already_handled {
            continue;
        }
        // Single-column gate. A decorative banner/frame around a prose
        // column (just a left+right border, no interior verticals) unions
        // into a 1-column "grid" whose lone cell is the whole paragraph
        // block. A real data table always has ≥2 columns; a 1-col ruled
        // region is a framed text box, never tabular. Word-count gating can't
        // be used here: genuine scattered description grids have equally long
        // cells.
        if let Block::Table { header, rows } = &run.block {
            let cols = header
                .as_ref()
                .map(|h| h.len())
                .or_else(|| rows.first().map(Vec::len))
                .unwrap_or(0);
            if cols < 2 {
                continue;
            }
        }
        let top_y = consumed
            .iter()
            .map(|&i| lines[i].bbox.y)
            .fold(f32::INFINITY, f32::min);
        global_ruled_tables.push((top_y, run.block));
        global_ruled_consumed.extend(consumed);
    }
    let global_ruled_owned: Option<Vec<ProjectedLine>> = if global_ruled_consumed.is_empty() {
        None
    } else {
        Some(
            lines
                .iter()
                .enumerate()
                .filter(|(i, _)| !global_ruled_consumed.contains(i))
                .map(|(_, l)| l.clone())
                .collect(),
        )
    };
    let lines: &[ProjectedLine] = global_ruled_owned.as_deref().unwrap_or(lines);

    // Cross-region table re-merge: when a V-cut sliced a table into sibling
    // leaves, fuse the slices back into single rows (one synthetic region)
    // before grouping. Each successful merge carries its own validated table
    // runs, which are handed to `classify_region` so emission is guaranteed
    // even for shapes the standard detectors would reject in normal flow.
    // TOC pages are skipped: their split number/title/page-number leaves
    // baseline-align perfectly and fuse into a convincing-but-wrong table.
    let mut cross_merged_owned: Option<Vec<ProjectedLine>> = None;
    let mut cross_region_runs: Vec<(Vec<u16>, Vec<super::tables::TableRun>)> = Vec::new();
    for _ in 0..3 {
        if toc_page {
            break;
        }
        let cur: &[ProjectedLine] = cross_merged_owned.as_deref().unwrap_or(lines);
        let Some(m) = super::cross_region::find_cross_region_table_merge(cur) else {
            break;
        };
        let mut next: Vec<ProjectedLine> = Vec::with_capacity(cur.len());
        next.extend_from_slice(&cur[..m.start]);
        let merged_path = m.merged[0].region_path.clone();
        next.extend(m.merged);
        next.extend_from_slice(&cur[m.end..]);
        cross_region_runs.push((merged_path, m.runs));
        cross_merged_owned = Some(next);
    }
    let lines: &[ProjectedLine] = cross_merged_owned.as_deref().unwrap_or(lines);

    // Region-grouped pipeline: split the filtered line list into contiguous
    // runs sharing a `region_path` (one xy_cut leaf each) and classify each
    // leaf as its own mini-page. Paragraph / list / code / heading state is
    // scoped per leaf so a column-wrap can't silently fuse two leaves into one
    // paragraph, and table detection runs per-leaf so a misfired borderless
    // table can't consume lines from a different leaf (otherwise a spurious
    // table starting in one column's footnote area eats the first lines of the
    // next column, dropping those words from any block). Cross-leaf merges
    // happen only in `stitch_regions` at the
    // end, where the rule is explicit and inspectable.
    let mut region_ranges: Vec<(usize, usize)> = Vec::new();
    {
        let mut s = 0;
        while s < lines.len() {
            let path = &lines[s].region_path;
            let mut e = s + 1;
            while e < lines.len() && lines[e].region_path == *path {
                e += 1;
            }
            region_ranges.push((s, e));
            s = e;
        }
    }

    // Page-level interruptions (HRs from vector graphics + figure refs) need
    // to interleave with text by y-position. Collect once, then dispatch into
    // each region's classify call by y-band so they emit in the right slot.
    // Two interruptions belonging to *no* region (e.g. an HR in a margin band
    // that no leaf covers) are emitted between regions in y order.
    let mut all_interruptions: Vec<(f32, Interruption)> = detect_horizontal_rules(page)
        .into_iter()
        .map(|y| (y, Interruption::Hr))
        .collect();
    if !matches!(image_mode, crate::config::ImageMode::Off) {
        for r in &page.image_refs {
            all_interruptions.push((r.bbox.y, Interruption::Figure(r.clone())));
        }
    }
    for (y, block) in global_ruled_tables {
        all_interruptions.push((y, Interruption::Table(block)));
    }
    all_interruptions.sort_by(|a, b| a.0.total_cmp(&b.0));

    // region_boundary_idx[k] = index into `blocks` where region k's first
    // block lives. Used by `stitch_regions` to know where to test cross-leaf
    // paragraph continuation. Stored alongside the region's last line so the
    // stitcher can use the same `continues_paragraph` cross-region rule that
    // governs intra-region merging — no second source of truth.
    let mut region_boundaries: Vec<usize> = Vec::new();
    let mut interrupt_cursor = 0usize;
    let mut blocks: Vec<Block> = Vec::new();

    let push_interruption = |blocks: &mut Vec<Block>, kind: Interruption| {
        blocks.push(match kind {
            Interruption::Hr => Block::HorizontalRule,
            Interruption::Figure(r) => Block::Figure {
                id: r.id,
                format: r.format,
            },
            Interruption::Table(b) => b,
        });
    };

    for (rstart, rend) in region_ranges {
        let region_lines = &lines[rstart..rend];

        // Compute this region's y-extent so we can slot in interruptions that
        // fall above or inside it. xy_cut leaves are rectangular partitions,
        // so a leaf's y-band is well-defined.
        let mut y_min = f32::INFINITY;
        let mut y_max = f32::NEG_INFINITY;
        for l in region_lines {
            y_min = y_min.min(l.bbox.y);
            y_max = y_max.max(l.bbox.y + l.bbox.height);
        }

        // Emit any interruptions that sit above this region's top edge before
        // we start emitting the region's own blocks.
        while interrupt_cursor < all_interruptions.len()
            && all_interruptions[interrupt_cursor].0 < y_min
        {
            let (_, kind) = all_interruptions[interrupt_cursor].clone();
            push_interruption(&mut blocks, kind);
            interrupt_cursor += 1;
        }

        // Hand the region its in-band interruptions (y in [y_min, y_max]) so
        // the per-line loop can interleave them by y, matching the old
        // page-level behavior within a leaf.
        let mut region_interruptions: Vec<(f32, Interruption)> = Vec::new();
        while interrupt_cursor < all_interruptions.len()
            && all_interruptions[interrupt_cursor].0 <= y_max
        {
            region_interruptions.push(all_interruptions[interrupt_cursor].clone());
            interrupt_cursor += 1;
        }

        let region_start = blocks.len();
        let precomputed_tables = cross_region_runs
            .iter()
            .find(|(path, _)| *path == lines[rstart].region_path)
            .map(|(_, runs)| runs.clone());
        let region_blocks = classify_region(
            region_lines,
            region_interruptions,
            page,
            heading_map,
            outline,
            toc_page,
            debug,
            precomputed_tables,
        );
        if !region_blocks.is_empty() {
            region_boundaries.push(region_start);
            blocks.extend(region_blocks);
        }
    }

    // Trailing interruptions below the last region.
    while interrupt_cursor < all_interruptions.len() {
        let (_, kind) = all_interruptions[interrupt_cursor].clone();
        push_interruption(&mut blocks, kind);
        interrupt_cursor += 1;
    }

    stitch_regions(blocks, &region_boundaries)
}

/// Mutable per-line flow state threaded through `classify_region`: the active
/// paragraph accumulator, the active code run, and the current list run. The
/// flush/reset/emit methods live here so list-state resets collapse to method
/// calls; methods on the struct also sidestep the borrow-checker friction of
/// threading five `&mut Option<…>` cells through a free closure.
#[derive(Default)]
struct FlowState {
    paragraph: Option<ParaAccum>,
    code: Option<Vec<String>>,
    list_base_indent: Option<f32>,
    last_list_item_idx: Option<usize>,
    last_list_line: Option<usize>,
}

impl FlowState {
    /// Drop the active list run. A heading, table, code block, or paragraph
    /// break all terminate the current list.
    fn reset_list(&mut self) {
        self.list_base_indent = None;
        self.last_list_item_idx = None;
        self.last_list_line = None;
    }

    /// Emit the active paragraph (if it has non-blank content) and clear it.
    fn flush_paragraph(&mut self, blocks: &mut Vec<Block>) {
        if let Some(acc) = self.paragraph.take()
            && !acc.raw.trim().is_empty()
        {
            blocks.push(paragraph_from_accum(acc));
        }
    }

    /// Emit the active code run (if non-empty) and clear it.
    fn flush_code(&mut self, blocks: &mut Vec<Block>) {
        if let Some(lines) = self.code.take()
            && !lines.is_empty()
        {
            let lang = detect_code_language(&lines);
            blocks.push(Block::CodeBlock { lines, lang });
        }
    }

    /// Emit any interruptions whose y is at or above `before_y`, flushing the
    /// active paragraph/code/list state first so each lands as its own block.
    fn emit_before(
        &mut self,
        blocks: &mut Vec<Block>,
        iter: &mut std::iter::Peekable<std::vec::IntoIter<(f32, Interruption)>>,
        before_y: f32,
    ) {
        while let Some((y, _)) = iter.peek() {
            if *y > before_y {
                break;
            }
            let (_, kind) = iter.next().unwrap();
            self.flush_paragraph(blocks);
            self.flush_code(blocks);
            self.reset_list();
            blocks.push(match kind {
                Interruption::Hr => Block::HorizontalRule,
                Interruption::Figure(r) => Block::Figure {
                    id: r.id,
                    format: r.format,
                },
                Interruption::Table(b) => b,
            });
        }
    }
}

/// Classify the lines of a single xy_cut leaf into a sequence of blocks. All
/// per-line state (active paragraph, code run, list run, heading_run) is local
/// to this call — nothing crosses a leaf boundary except through the explicit
/// `stitch_regions` post-pass. Page-level signals (`heading_map`,
/// `struct_nodes`, `outline`, header/footer y-bands) are still consulted
/// because they're computed once per document.
#[allow(clippy::too_many_arguments)]
fn classify_region(
    lines: &[ProjectedLine],
    interruptions: Vec<(f32, Interruption)>,
    page: &ParsedPage,
    heading_map: &[(f32, u8)],
    outline: &[OutlineTarget],
    toc_page: bool,
    debug: bool,
    precomputed_tables: Option<Vec<super::tables::TableRun>>,
) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut state = FlowState::default();
    let mut heading_run: Option<(u8, usize)> = None;
    // Track whether we've already emitted a TOC title on this page. Once seen,
    // any subsequent line matching `is_toc_title` (e.g. "Index", "List of
    // Figures") is a TOC entry pointing at that chapter, not another title,
    // and must stay suppressed.
    let mut toc_title_emitted = false;

    // Per-region table detection. Indices are local to `lines`. Page-level
    // graphics are still consulted for ruled-table detection because path
    // objects are page-coordinate; the detector intersects them against the
    // sub-list's line bboxes anyway.
    let ruled_runs = detect_ruled_tables(lines, &page.graphics, page.page_width, page.page_height);
    let borderless_runs = precomputed_tables.unwrap_or_else(|| detect_tables(lines));
    let table_runs = merge_table_runs(ruled_runs, borderless_runs);

    // Region-wide pre-pass: which line indices carry a lettered/roman marker
    // (`a.`, `i.`) that belongs to a confirmed sequential list run. Only these
    // are treated as list items below — a lone `A.` / initial is left alone.
    // Lines already inside a detected table are excluded so enumerated table
    // footnotes (`(a)`, `(b)` below a table) aren't pulled out as a list,
    // which would disturb the table's row/track inference.
    let table_covered: std::collections::HashSet<usize> = table_runs
        .iter()
        .flat_map(|run| run.start..run.end)
        .collect();
    let ordered_list_lines = detect_ordered_list_lines(lines, &table_covered);

    const TABLE_HR_SUPPRESS_HEADROOM_ROWS: f32 = 4.0;
    let table_y_extents: Vec<(f32, f32)> = table_runs
        .iter()
        .map(|run| {
            let top_line = &lines[run.start];
            let row_h = top_line.bbox.height.max(super::MIN_ROW_HEIGHT_PT);
            let top = top_line.bbox.y - row_h * TABLE_HR_SUPPRESS_HEADROOM_ROWS;
            let last = &lines[run.end.saturating_sub(1).max(run.start)];
            let bot = last.bbox.y + last.bbox.height;
            (top, bot)
        })
        .collect();

    let mut table_iter = table_runs.into_iter().peekable();

    let in_table_band = |y: f32| {
        table_y_extents
            .iter()
            .any(|(top, bot)| y >= *top - 2.0 && y <= *bot + 2.0)
    };
    let mut region_interruptions: Vec<(f32, Interruption)> = interruptions
        .into_iter()
        .filter(|(y, _)| !in_table_band(*y))
        .collect();
    region_interruptions.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut interruptions = region_interruptions.into_iter().peekable();

    let mut idx = 0;
    while idx < lines.len() {
        if let Some(run) = table_iter.peek()
            && run.start == idx
        {
            // Flush any interruptions above this table's top edge first.
            let table_top = lines[run.start].bbox.y;
            state.emit_before(&mut blocks, &mut interruptions, table_top);
            state.flush_paragraph(&mut blocks);
            state.flush_code(&mut blocks);
            state.reset_list();
            let run = table_iter.next().unwrap();
            blocks.push(run.block);
            idx = run.end;
            continue;
        }
        let line_idx = idx;
        let line = &lines[line_idx];
        // Emit any interruptions that fall above this line.
        state.emit_before(&mut blocks, &mut interruptions, line.bbox.y);
        idx += 1;
        let text = line.text.trim();
        if text.is_empty() {
            continue;
        }
        // Skip rotated text (vertical sidebars, arXiv banners, watermarks).
        // Including it would either inject a paragraph of disconnected
        // characters or be misclassified as a heading.
        if is_rotated_line(line) {
            continue;
        }
        if debug {
            eprintln!(
                "[md] y={:.1} h={:.1} size={:.2} anchor={:?} indent={:.1} text={:?}",
                line.bbox.y,
                line.bbox.height,
                line.dominant_font_size,
                line.anchor,
                line.indent_x,
                text
            );
        }

        // Code block detection runs first so a mono heading-shaped line
        // (rare but plausible — e.g., a code identifier in a large mono font)
        // still becomes code. Mono content also wouldn't make a useful
        // heading.
        if line.all_mono {
            state.flush_paragraph(&mut blocks);
            state.reset_list();
            state
                .code
                .get_or_insert_with(Vec::new)
                .push(line.text.trim_end().to_string());
            continue;
        }
        // Any non-mono line ends the current code block (if any).
        state.flush_code(&mut blocks);

        // Decorative divider / flourish lines (`* * * *`, a lone em-dash).
        // Handled before heading/paragraph classification so the ornament
        // neither glues onto the next paragraph nor gets promoted to a heading.
        if let Some(is_rule) = decorative_divider_kind(text) {
            state.flush_paragraph(&mut blocks);
            state.reset_list();
            heading_run = None;
            if is_rule {
                blocks.push(Block::HorizontalRule);
            }
            if debug {
                eprintln!(
                    "[md decorative] {} '{}'",
                    if is_rule { "rule" } else { "drop" },
                    text.chars().take(40).collect::<String>(),
                );
            }
            continue;
        }

        // Priority chain: tagged-PDF struct tree → outline → font-size map.
        let tagged_level = struct_heading_level(line, &page.struct_nodes);
        // Caption lines ("Figure 7", "Table 3.") are routinely set in a
        // distinct (and slightly larger) font that lands them in the
        // font-size heading map. Suppress font-size promotion for them;
        // outline / struct-tree signals still win since those are explicit.
        let is_first_toc_title = is_toc_title(text) && !toc_title_emitted;
        let toc_suppress = toc_page && !is_first_toc_title;
        // Outline entries on a TOC page are the TOC itself — every entry
        // prefix-matches an outline target ("Introduction", "Part I", ...).
        // Promoting them to `##` shreds the document's heading structure (a
        // TOC page is just `# Contents`). Tagged-PDF struct levels are explicit
        // and still win.
        let outline_level = tagged_level.or_else(|| {
            if toc_suppress {
                None
            } else {
                outline_heading_level(line, page.page_height, outline, text)
            }
        });
        // Rotated lines (margin stamps like a sideways "arXiv:…" watermark) are
        // excluded from `build_heading_map`, but their size can still coincide
        // with a map entry built from horizontal text. Suppress font-size
        // promotion for them so they don't surface as spurious headings.
        let size_level =
            if is_caption_line(text) || toc_suppress || is_rotated_line(line) || line.in_figure {
                None
            } else {
                heading_level_for(heading_size_of(line), heading_map)
            };
        // Guard against height-jitter false headings: a line that flows from
        // the previous line (same paragraph) AND starts lowercase is a
        // mid-paragraph continuation, not a heading — even if its
        // height-estimated size jittered into a heading slot. A real heading
        // has a break above it and is capitalized, so it won't satisfy both.
        // Outline / struct-tree levels are explicit and bypass this guard.
        let size_level = size_level.filter(|_| {
            let starts_lower = text.chars().next().is_some_and(|c| c.is_lowercase());
            let prev = state
                .paragraph
                .as_ref()
                .map(|p| &p.last)
                .or(state.last_list_line.map(|i| &lines[i]));
            // A previous line ending in a mid-word hyphen wrap means this
            // line is its continuation regardless of capitalization
            // ("…SOLAR 10.7 Billion-" / "Parameter Model: We have…").
            let prev_hyphen_wrap = prev.is_some_and(|p| ends_hyphenated(&p.text));
            // `prev` is the *live body paragraph* (or list line) immediately
            // above, so it only exists mid-paragraph — never right after a
            // heading. A lowercase line that *completes a sentence* (ends with
            // terminal punctuation) while its predecessor is left open (no
            // terminal punctuation) is a wrapped sentence tail, not a heading —
            // even when a centered short tail's large left-edge indent defeats
            // `continues_paragraph` ("…journalistic or" → "scholarly works.").
            // Requiring the tail to end a sentence keeps genuine lowercase
            // headings (which rarely end in `.`/`!`/`?`) promoted.
            let sentence_tail = starts_lower
                && ends_sentence_final(text)
                && prev.is_some_and(|p| !ends_sentence_final(&p.text));
            let is_continuation =
                prev.is_some_and(|p| continues_paragraph(p, line)) || sentence_tail;
            !((starts_lower || prev_hyphen_wrap) && is_continuation)
        });
        // Long-text guard: a real heading is a label, not a sentence with
        // multiple clauses. Footnotes / citations / captions that happen to
        // be set slightly larger than body text (e.g. a 10.7pt citation
        // over a 9pt body) score as the smallest heading level in the size
        // map; without a length cap they emit as `##`-shaped sentences.
        // Threshold is generous — book titles and long subsection labels
        // can run 100+ chars — but a 200-char citation is unambiguously
        // not a heading.
        let size_level = size_level.filter(|_| text.chars().count() <= HEADING_MAX_TEXT_CHARS);
        // Attribution lines under figures/charts are never section headings,
        // even when the chart caption font is slightly above body size.
        let size_level =
            size_level.filter(|_| !crate::markdown_layout::headings::is_attribution_line(text));
        // Run-in label guard: a size-promoted line that contains a mid-line
        // ". " followed by ≥3 more words AND ends with a mid-word "-" wrap
        // is almost always a paragraph leading with a bold run-in label
        // ("Base model. Any n-layer transformer architec-"), not a section
        // heading. Both signals must fire — a heading occasionally has either
        // on its own (e.g. abbreviation periods, intentional hyphen).
        let size_level = size_level.filter(|_| {
            let has_mid_sentence = text.contains(". ") || text.contains(": ");
            let mid_word_wrap = text.trim_end().ends_with('-');
            !(has_mid_sentence && mid_word_wrap)
        });
        // A standalone "Contents" / "Table of Contents" / "Index" line is
        // almost always a real H1, even when `page_is_toc` is false — many
        // TOCs list entries without inline trailing page numbers, so the
        // page-level detector misses them. `is_toc_title` matches the *entire*
        // trimmed line against an exact whitelist, so it can't fire mid-prose.
        let toc_title_level = if is_first_toc_title { Some(1u8) } else { None };
        if is_first_toc_title {
            toc_title_emitted = true;
        }
        let level = outline_level
            .or(size_level)
            .or(toc_title_level)
            .map(|l| l.clamp(1, MAX_HEADING_LEVELS as u8));
        let mut demoted_heading = false;
        // Unconditional wrap-continuation merge: when the previous block is a
        // live heading_run and this line continues it (same font, same region,
        // tight gap), merge regardless of whether this line itself qualifies
        // as a heading. Catches body-size wrap continuations of bold headings
        // ("Cellular Cycle\nand Replication") where line 2 has the same
        // sub-body size and would otherwise emit as a stray bold paragraph.
        // Gate the unconditional merge: only fold body-sized continuations that
        // *look* like the rest of a heading, not body prose. `continues_heading`
        // only enforces structural agreement (font/bold/region/gap), so a body
        // paragraph that happens to share those traits (e.g. a heading followed
        // by an all-bold body paragraph) would otherwise be absorbed.
        // Reject the candidate if it starts a list item (`l.`, `1.`, `· `…) or
        // contains a mid-sentence period — both are unambiguous prose signals
        // and never appear inside a real wrapped heading.
        let cont_looks_like_heading = parse_list_marker(text).is_none() && !text.contains(". ");
        if level.is_none()
            && cont_looks_like_heading
            && let Some((run_level, run_idx)) = heading_run.as_ref()
            && continues_heading(&lines[*run_idx], line)
            && let Some(Block::Heading {
                level: last_level,
                text: htext,
            }) = blocks.last_mut()
            && *last_level == *run_level
        {
            let combined_chars = htext.chars().count() + 1 + text.chars().count();
            if combined_chars <= HEADING_MAX_TEXT_CHARS {
                let run_level = *run_level;
                if debug {
                    eprintln!(
                        "[MD heading-wrap-uncond] merge h{} '{}' <- '{}' (prev_idx={} cur_idx={} combined={})",
                        run_level,
                        htext.chars().take(40).collect::<String>(),
                        text.chars().take(60).collect::<String>(),
                        run_idx,
                        line_idx,
                        combined_chars
                    );
                }
                append_inline_continuation(htext, text, &collapse_whitespace(text));
                heading_run = Some((run_level, line_idx));
                continue;
            }
        }
        if let Some(level) = level {
            // Merge a wrapped continuation into the heading directly above when
            // it flows as one block: same level, the heading is still the last
            // emitted block, and the line continues the paragraph. A real
            // section heading is followed by body text (which breaks the run),
            // so only a genuinely multi-line heading/caption merges here.
            if let Some((run_level, run_idx)) = heading_run.as_ref()
                && *run_level == level
                && continues_heading(&lines[*run_idx], line)
                && let Some(Block::Heading {
                    level: last_level,
                    text: htext,
                }) = blocks.last_mut()
                && *last_level == level
            {
                // Length guard: a wrapped multi-line heading is fine, but a
                // run that grows past `HEADING_MAX_TEXT_CHARS` is almost
                // certainly a footnote/citation block whose font is only
                // slightly above body (e.g. 10.7pt over 9pt body). Demote
                // the existing heading block back to a paragraph and let
                // the current line continue that paragraph below. The
                // first line scored as a solo heading because it was under
                // the limit on its own; only the second-or-later line tips
                // the cumulative content into footnote territory.
                let combined_chars = htext.chars().count() + 1 + text.chars().count();
                if combined_chars > HEADING_MAX_TEXT_CHARS {
                    let demoted = std::mem::take(htext);
                    blocks.pop();
                    state.paragraph = Some(ParaAccum {
                        raw: demoted.clone(),
                        inline: demoted,
                        last: lines[*run_idx].clone(),
                        uniform: None,
                    });
                    heading_run = None;
                    demoted_heading = true;
                } else {
                    append_inline_continuation(htext, text, &collapse_whitespace(text));
                    heading_run = Some((level, line_idx));
                    continue;
                }
            }
            if !demoted_heading {
                state.flush_paragraph(&mut blocks);
                state.reset_list();
                if debug {
                    eprintln!(
                        "[MD heading-emit size/outline] h{} idx={} '{}' size={:.2}",
                        level,
                        line_idx,
                        text.chars().take(80).collect::<String>(),
                        line.dominant_font_size,
                    );
                }
                blocks.push(Block::Heading {
                    level,
                    text: collapse_whitespace(text),
                });
                heading_run = Some((level, line_idx));
                continue;
            }
        }

        // List item? Bullets/decimals come from `parse_list_marker`; lettered
        // and roman markers are only accepted when the region pre-pass
        // confirmed them as part of a sequential run.
        let list_marker: Option<(bool, String, String)> = parse_list_marker(text)
            .map(|(o, m, r)| (o, m, r.to_string()))
            .or_else(|| {
                if ordered_list_lines.contains(&line_idx) {
                    split_ordered_marker_for_emit(text).map(|(m, r)| (true, m, r))
                } else {
                    None
                }
            });
        if let Some((ordered, marker, rest)) = list_marker {
            let rest = rest.as_str();
            // Numbered bold-section heading: "1. **Foo**" / "5. **The dynamics**".
            // When the post-marker body is uniformly bold + body-sized,
            // standalone (paragraph-break gap above and below), short, and
            // mostly alpha, treat it as a heading rather than the first item
            // of an ordered list. Without this, decimal-numbered section
            // headings in technical/legal/scientific PDFs silently emit as
            // ordered list items and lose all heading structure.
            if ordered
                && !toc_suppress
                && looks_like_numbered_bold_heading(
                    line,
                    rest,
                    state
                        .paragraph
                        .as_ref()
                        .map(|p| &p.last)
                        .or(state.last_list_line.map(|i| &lines[i])),
                )
            {
                state.flush_paragraph(&mut blocks);
                state.reset_list();
                let level = (heading_map.len() as u8 + 1).clamp(1, MAX_HEADING_LEVELS as u8);
                blocks.push(Block::Heading {
                    level,
                    text: collapse_whitespace(text),
                });
                continue;
            }
            state.flush_paragraph(&mut blocks);
            let base = *state.list_base_indent.get_or_insert(line.indent_x);
            let level = (((line.indent_x - base) / LIST_INDENT_STEP_PT)
                .round()
                .max(0.0)) as u8;
            state.last_list_item_idx = Some(blocks.len());
            state.last_list_line = Some(line_idx);
            // Render the list-item text via the inline pipeline so per-span
            // emphasis surfaces. We strip the marker from `rest` (already
            // done by `parse_list_marker`), but emphasis lives on `line.spans`,
            // which still contain the marker span — render the line and then
            // peel the marker off the front of the rendered string.
            let item_text = render_list_item_text(line, &marker, rest);
            blocks.push(Block::ListItem {
                ordered,
                marker,
                level,
                text: item_text,
                bold: false,
                italic: false,
            });
            continue;
        }

        // Continuation of a list item: same gap/font rules as paragraphs, but
        // hanging-indent tolerant. Wrapped bodies either left-flush below the
        // marker (footnote style) or align under the marker's text (indented
        // right) — `continues_list_item` accepts both.
        if let Some(item_idx) = state.last_list_item_idx
            && let Some(prev_idx) = state.last_list_line
            && continues_list_item(&lines[prev_idx], line)
            && let Some(Block::ListItem {
                text: prev_text, ..
            }) = blocks.get_mut(item_idx)
        {
            // De-hyphenate against the prior rendered text, then append the
            // inline-styled continuation.
            let cont_inline = render_line_inline(line);
            append_inline_continuation(prev_text, text, &cont_inline);
            state.last_list_line = Some(line_idx);
            continue;
        }

        // Bold body-size heading. Section headings in academic / technical
        // PDFs are routinely body-sized + bold (e.g. "Abstract",
        // "1 Introduction"); without this rule they look just like a bold
        // first sentence of a paragraph. Runs after list-marker detection so
        // bold bullet items stay as list items.
        let prev_for_gap = state
            .paragraph
            .as_ref()
            .map(|p| &p.last)
            .or(state.last_list_line.map(|i| &lines[i]));
        let next_for_gap = lines.get(idx);
        if !toc_suppress && looks_like_bold_heading(line, prev_for_gap, next_for_gap) {
            state.flush_paragraph(&mut blocks);
            state.reset_list();
            // Level: one deeper than the deepest size-based level we already
            // have. With an empty heading_map this lands on H1; with a full
            // 6-level map it caps at H6.
            let level = (heading_map.len() as u8 + 1).clamp(1, MAX_HEADING_LEVELS as u8);
            if debug {
                eprintln!(
                    "[MD heading-emit bold] h{} idx={} '{}' size={:.2}",
                    level,
                    line_idx,
                    text.chars().take(80).collect::<String>(),
                    line.dominant_font_size,
                );
            }
            blocks.push(Block::Heading {
                level,
                text: collapse_whitespace(text),
            });
            // Arm the heading_run so a wrapped continuation on the next line
            // merges into this heading instead of emitting as a second one.
            heading_run = Some((level, line_idx));
            continue;
        }

        match state.paragraph.as_mut() {
            Some(acc) if continues_paragraph(&acc.last, line) => {
                append_to_paragraph(acc, line);
            }
            _ => {
                state.flush_paragraph(&mut blocks);
                state.reset_list();
                let inline = render_line_inline(line);
                let raw = collapse_whitespace(text);
                // Strike has no block-level Paragraph flag, so a struck line
                // must keep the per-line `inline` rendering (which emits `~~…~~`)
                // rather than take the uniform raw-text fast path.
                let uniform = line_uniform_style(line)
                    .filter(|s| !s.strike)
                    .map(|s| (s.bold, s.italic));
                state.paragraph = Some(ParaAccum {
                    raw,
                    inline,
                    last: line.clone(),
                    uniform,
                });
            }
        }
    }

    state.flush_paragraph(&mut blocks);
    state.flush_code(&mut blocks);
    // Flush any trailing interruptions that sat below the last text line.
    state.emit_before(&mut blocks, &mut interruptions, f32::INFINITY);
    blocks
}

/// Classify a line that is *purely decorative* — no alphanumeric content, made
/// up only of divider symbols (`* * * *`, `———`, `____`). Returns:
///   - `Some(true)`  → a section divider (≥3 symbols): emit a thematic break.
///   - `Some(false)` → a lone 1–2 char, drop it, tool small
///   - `None`        → not decorative; classify normally.
///
/// Without this, a `* * * *` divider line flows into the paragraph accumulator
/// and glues the ornament onto the start of the following paragraph, and a lone
/// decorative dash gets size-promoted to a `# -` heading.
fn decorative_divider_kind(text: &str) -> Option<bool> {
    let mut symbols = 0usize;
    for c in text.chars() {
        if c.is_whitespace() {
            continue;
        }
        if c.is_alphanumeric() || !is_divider_symbol(c) {
            return None;
        }
        symbols += 1;
    }
    if symbols == 0 {
        return None;
    }
    Some(symbols >= 3)
}

/// Characters treated as decorative divider/ornament glyphs. Deliberately
/// conservative — excludes `.`, `=`, `~`, `+`, `#` which carry meaning elsewhere
/// (dot leaders, ellipses, setext-ish rules, headers).
fn is_divider_symbol(c: char) -> bool {
    matches!(
        c,
        '*' | '-' | '_' | '–' | '—' | '•' | '·' | '●' | '▪' | '■' | '◦' | '★' | '☆'
    )
}

/// Best-effort language hint for a fenced code block, used as the fence
/// info-string. Conservative: returns `Some` only when the body carries
/// strong, language-specific signals; otherwise `None` (bare fence). This
/// feeds markdown consumers that key off the language tag (e.g. syntax
/// highlighting).
fn detect_code_language(lines: &[String]) -> Option<String> {
    let body = lines.join("\n");
    let trimmed = body.trim_start();

    // JSON: starts with a brace/bracket and reads as an object/array with
    // quoted keys. Checked first because `{`/`[` are unambiguous openers.
    if let Some(first) = trimmed.chars().find(|c| !c.is_whitespace())
        && (first == '{' || first == '[')
        && body.contains("\":")
        && body.matches('{').count() + body.matches('[').count() >= 1
    {
        return Some("json".to_string());
    }

    // C / C++: preprocessor includes, namespace qualifiers, stream ops.
    let cpp_hits = [
        "#include",
        "std::",
        "int main",
        "nullptr",
        "->",
        "::",
        "template<",
    ]
    .iter()
    .filter(|s| body.contains(**s))
    .count();
    if cpp_hits >= 2 {
        return Some("cpp".to_string());
    }

    // Python: keyword-led lines, f-strings, comprehensions, dunder/`self.`.
    let py_signals = [
        "self.", "import ", "from ", "def ", "class ", "print(", "lambda ", "elif ", "f'", "f\"",
        "__", " for ", "len(", "sorted(", "range(",
    ];
    let py_hits = py_signals.iter().filter(|s| body.contains(**s)).count();
    // A colon-terminated control line (`if ...:`, `for ...:`, `def ...:`) is a
    // strong Python tell that other curly-brace languages lack.
    let py_colon_block = lines.iter().any(|l| {
        let t = l.trim_end();
        t.ends_with(':')
            && [
                "if ", "for ", "while ", "def ", "class ", "elif ", "else", "try", "except",
                "with ",
            ]
            .iter()
            .any(|kw| t.trim_start().starts_with(kw))
    });
    if py_hits >= 2 || (py_hits >= 1 && py_colon_block) {
        return Some("python".to_string());
    }

    None
}

/// Cross-region merging. Walks the concatenated block stream and, at each
/// region boundary, tries to fuse the last block of region A with the first
/// block of region B when they represent one logical unit split across leaves.
/// This is the *only* place per-region scoping is relaxed — every other
/// detector now treats a region as a hard wall — so the merge rules here are
/// deliberately narrow and inspectable.
///
/// Currently supports:
/// - **Paragraph → Paragraph**: same cross-region rule as
///   `continues_paragraph`'s `region_path != region_path` arm: the previous
///   paragraph ends mid-sentence (no terminal punctuation) and the next
///   starts lowercase. Heals a column wrap losing its tail to a separate
///   block.
/// - **Hyphen splice**: previous paragraph ends `<letter>-` and next starts
///   ASCII-lowercase. Already handled in `render_blocks` for the adjacent
///   case, but doing it here too lets the merged block flow through the
///   uniform paragraph-rendering path and avoids a `\n\n` slipping in when
///   the renderer sees an intervening non-paragraph block.
///
/// Lists, code blocks, headings, and tables are *not* fused across regions.
/// A bullet point split across columns is rare and a false merge is worse
/// than a true split; a table split across leaves indicates a projection-side
/// issue better fixed there than papered over here.
fn stitch_regions(blocks: Vec<Block>, region_starts: &[usize]) -> Vec<Block> {
    if region_starts.len() <= 1 {
        return blocks;
    }
    let boundary_set: std::collections::HashSet<usize> =
        region_starts.iter().skip(1).copied().collect();
    let mut out: Vec<Block> = Vec::with_capacity(blocks.len());
    for (i, block) in blocks.into_iter().enumerate() {
        if boundary_set.contains(&i)
            && let Some(prev) = out.last_mut()
            && let (
                Block::Paragraph {
                    text: prev_text, ..
                },
                Block::Paragraph {
                    text: cur_text,
                    bold: false,
                    italic: false,
                },
            ) = (prev, &block)
        {
            let prev_trim = prev_text.trim_end();
            let starts_lower = cur_text
                .trim_start()
                .chars()
                .next()
                .is_some_and(|c| c.is_lowercase());
            // Hyphen-splice path: a mid-word soft hyphen break across the
            // leaf boundary. Drop the hyphen, join with no separator.
            if is_soft_hyphen_break(prev_text, cur_text) {
                // Pop the trailing `-` (it may have whitespace after it from
                // a previous trim, so re-trim).
                while prev_text.ends_with(|c: char| c.is_whitespace()) {
                    prev_text.pop();
                }
                prev_text.pop(); // the '-'
                prev_text.push_str(cur_text.trim_start());
                continue;
            }
            let ends_open = !prev_trim.ends_with(|c: char| {
                matches!(
                    c,
                    '.' | '!' | '?' | ':' | ';' | '”' | '"' | ')' | ']' | '。' | '』' | '」'
                )
            });
            if ends_open && starts_lower {
                prev_text.push(' ');
                prev_text.push_str(cur_text.trim_start());
                continue;
            }
        }
        out.push(block);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::blocks::{Block, render_blocks};
    use super::super::headings::{build_heading_map, compute_body_size};
    use super::super::repetition::compute_header_footer_set;
    use super::super::test_helpers::{
        header_footer_page, line, mono_line, page, page_with_graphics, stroke, styled_line,
    };
    use super::*;
    use crate::types::TextItem;

    #[test]
    fn classify_emits_heading_and_paragraph() {
        let p = page(vec![
            line("Title of the document goes here", 50.0, 50.0, 18.0, 18.0),
            line("First sentence of the para-", 50.0, 80.0, 10.0, 10.0),
            line("graph continues here.", 50.0, 92.0, 10.0, 10.0),
            line("Another paragraph.", 50.0, 130.0, 10.0, 10.0),
        ]);
        let pages = vec![p];
        let body = compute_body_size(&pages);
        let map = build_heading_map(&pages, body);
        let blocks = classify_page(&pages[0], &map);
        assert_eq!(blocks.len(), 3);
        match &blocks[0] {
            Block::Heading { level, text } => {
                assert_eq!(*level, 1);
                assert_eq!(text, "Title of the document goes here");
            }
            other => panic!("expected heading, got {other:?}"),
        }
        match &blocks[1] {
            Block::Paragraph { text: t, .. } => {
                assert!(t.contains("paragraph continues"), "got: {t}");
                assert!(!t.contains("para-"), "de-hyphenation failed: {t}");
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
        match &blocks[2] {
            Block::Paragraph { text: t, .. } => assert_eq!(t, "Another paragraph."),
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn paragraph_break_on_big_gap() {
        let p = page(vec![
            line("Line A.", 50.0, 80.0, 10.0, 10.0),
            line("Line B.", 50.0, 200.0, 10.0, 10.0),
        ]);
        let pages = vec![p];
        let body = compute_body_size(&pages);
        let map = build_heading_map(&pages, body);
        let blocks = classify_page(&pages[0], &map);
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn classify_emits_list_items() {
        let p = page(vec![
            line("Intro line.", 50.0, 50.0, 10.0, 10.0),
            line("• first bullet", 60.0, 80.0, 10.0, 10.0),
            line("• second bullet", 60.0, 92.0, 10.0, 10.0),
            line("◦ nested item", 72.0, 104.0, 10.0, 10.0),
            line("• back to top", 60.0, 116.0, 10.0, 10.0),
        ]);
        let pages = vec![p];
        let body = compute_body_size(&pages);
        let map = build_heading_map(&pages, body);
        let blocks = classify_page(&pages[0], &map);
        let list_items: Vec<&Block> = blocks
            .iter()
            .filter(|b| matches!(b, Block::ListItem { .. }))
            .collect();
        assert_eq!(list_items.len(), 4);
        if let Block::ListItem { level, text, .. } = list_items[0] {
            assert_eq!(*level, 0);
            assert_eq!(text, "first bullet");
        } else {
            panic!();
        }
        // The "- nested item" line is indented +12pt from the base bullet.
        if let Block::ListItem { level, .. } = list_items[2] {
            assert_eq!(*level, 1);
        } else {
            panic!();
        }
    }

    #[test]
    fn classify_emits_code_block() {
        let p = page(vec![
            line("Intro line.", 50.0, 50.0, 10.0, 10.0),
            mono_line("    let x = 1;", 80.0),
            mono_line("    let y = x + 2;", 92.0),
            line("After the code.", 50.0, 120.0, 10.0, 10.0),
        ]);
        let pages = vec![p];
        let body = compute_body_size(&pages);
        let map = build_heading_map(&pages, body);
        let blocks = classify_page(&pages[0], &map);
        // Expect: Paragraph("Intro line."), CodeBlock(2 lines), Paragraph("After...")
        assert_eq!(blocks.len(), 3);
        match &blocks[1] {
            Block::CodeBlock { lines, .. } => {
                assert_eq!(lines.len(), 2);
                assert!(lines[0].contains("let x = 1;"));
                assert!(lines[1].contains("let y = x + 2;"));
            }
            other => panic!("expected code block, got {other:?}"),
        }
        let s = render_blocks(&blocks);
        assert!(s.contains("```\n    let x = 1;"));
        assert!(s.ends_with("After the code."));
    }

    #[test]
    fn detect_code_language_classifies_common_langs() {
        let py = vec![
            "self.mm_list = sorted([x for x in self.files_list])".to_string(),
            "self.mm_total = len(self.mm_list)".to_string(),
        ];
        assert_eq!(detect_code_language(&py).as_deref(), Some("python"));

        let py_block = vec![
            "if item.total > 0:".to_string(),
            "    print('many')".to_string(),
        ];
        assert_eq!(detect_code_language(&py_block).as_deref(), Some("python"));

        let json = vec![
            "{".to_string(),
            "    \"formatVersion\": \"1.0\",".to_string(),
            "}".to_string(),
        ];
        assert_eq!(detect_code_language(&json).as_deref(), Some("json"));

        let cpp = vec![
            "#include <vector>".to_string(),
            "std::vector<int> v;".to_string(),
        ];
        assert_eq!(detect_code_language(&cpp).as_deref(), Some("cpp"));

        // Ambiguous / unknown content stays untagged (bare fence).
        let unknown = vec!["let x = 1;".to_string(), "let y = x + 2;".to_string()];
        assert_eq!(detect_code_language(&unknown), None);
    }

    #[test]
    fn classify_marks_paragraph_bold_when_all_lines_bold() {
        let mut a = line("Bold line one.", 50.0, 50.0, 10.0, 10.0);
        let mut b = line("bold continuation.", 50.0, 62.0, 10.0, 10.0);
        // Mark the underlying spans as bold so per-span style detection sees
        // it — the new inline pipeline reads from `spans`, not the per-line
        // `all_bold` shortcut flag.
        let bold_span = TextItem {
            text: "x".into(),
            font_name: Some("Arial-Bold".into()),
            ..Default::default()
        };
        a.spans = vec![bold_span.clone()];
        b.spans = vec![bold_span];
        a.all_bold = true;
        b.all_bold = true;
        let p = page(vec![a, b]);
        let pages = vec![p];
        let body = compute_body_size(&pages);
        let map = build_heading_map(&pages, body);
        let blocks = classify_page(&pages[0], &map);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Paragraph { bold, italic, .. } => {
                assert!(*bold);
                assert!(!*italic);
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
        let s = render_blocks(&blocks);
        assert!(s.starts_with("**") && s.ends_with("**"), "got: {s}");
    }

    #[test]
    fn detects_simple_borderless_table() {
        use super::super::test_helpers::line_with_spans;
        let lines = vec![
            line_with_spans(
                &[("Name", 50.0), ("Age", 150.0), ("City", 250.0)],
                100.0,
                10.0,
            ),
            line_with_spans(
                &[("Alice", 50.0), ("30", 150.0), ("NYC", 250.0)],
                115.0,
                10.0,
            ),
            line_with_spans(&[("Bob", 50.0), ("25", 150.0), ("LA", 250.0)], 130.0, 10.0),
        ];
        let p = page(lines);
        let pages = vec![p];
        let body = compute_body_size(&pages);
        let map = build_heading_map(&pages, body);
        let blocks = classify_page(&pages[0], &map);
        assert_eq!(blocks.len(), 1, "got: {blocks:?}");
        match &blocks[0] {
            Block::Table { header, rows } => {
                // Header isn't bold so no header row promoted.
                assert!(header.is_none());
                assert_eq!(rows.len(), 3);
                assert_eq!(rows[0][0], "Name");
                assert_eq!(rows[1][2], "NYC");
            }
            other => panic!("expected table, got {other:?}"),
        }
    }

    #[test]
    fn full_format_strips_header_footer() {
        let pages = vec![
            header_footer_page(1, "Acme Confidential", "Page 1 of 2", "First page body."),
            header_footer_page(2, "Acme Confidential", "Page 2 of 2", "Second page body."),
        ];
        let body = compute_body_size(&pages);
        let map = build_heading_map(&pages, body);
        let set = compute_header_footer_set(&pages);
        let blocks = classify_page_with_filters(
            &pages[0],
            &map,
            &set,
            &[],
            crate::config::ImageMode::Placeholder,
            &std::collections::HashSet::new(),
        );
        let s = render_blocks(&blocks);
        assert!(!s.contains("Acme Confidential"), "got: {s}");
        assert!(!s.contains("Page 1 of 2"), "got: {s}");
        assert!(s.contains("First page body."));
    }

    #[test]
    fn classify_paragraph_with_mid_line_bold() {
        // First line has a bold word mid-line → not uniformly styled; paragraph
        // should emit baked-in `**bold**` inside the text and `bold=false` at
        // the block level.
        let a = styled_line(
            &[
                ("a sentence with a", 50.0, Some("Arial")),
                ("bold", 180.0, Some("Arial-Bold")),
                ("word in it.", 230.0, Some("Arial")),
            ],
            50.0,
            10.0,
        );
        let p = page(vec![a]);
        let pages = vec![p];
        let body = compute_body_size(&pages);
        let map = build_heading_map(&pages, body);
        let blocks = classify_page(&pages[0], &map);
        assert_eq!(blocks.len(), 1, "got: {blocks:?}");
        match &blocks[0] {
            Block::Paragraph { text, bold, italic } => {
                assert!(!*bold, "mixed-style paragraph shouldn't set block bold");
                assert!(!*italic);
                assert!(text.contains("**bold**"), "got: {text}");
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn classify_list_item_strips_marker_under_emphasis() {
        // Whole bullet line is bold (marker + text). Rendered text should be
        // wrapped, with the marker dropped (the renderer prints it).
        let l = styled_line(
            &[
                ("•", 60.0, Some("Arial-Bold")),
                ("important item", 80.0, Some("Arial-Bold")),
            ],
            50.0,
            10.0,
        );
        let p = page(vec![l]);
        let pages = vec![p];
        let body = compute_body_size(&pages);
        let map = build_heading_map(&pages, body);
        let blocks = classify_page(&pages[0], &map);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::ListItem { text, .. } => {
                assert_eq!(text, "**important item**");
            }
            other => panic!("expected list item, got {other:?}"),
        }
    }

    #[test]
    fn hr_emitted_between_lines_by_y_order() {
        let lines = vec![
            line("before the rule", 50.0, 100.0, 10.0, 10.0),
            line("after the rule", 50.0, 300.0, 10.0, 10.0),
        ];
        // Stroke between the two lines, far from either's baseline.
        let p = page_with_graphics(lines, vec![stroke(50.0, 200.0, 450.0, 200.0, 0.5)]);
        let blocks = classify_page(&p, &[]);
        let has_hr = blocks
            .iter()
            .position(|b| matches!(b, Block::HorizontalRule));
        assert!(has_hr.is_some(), "expected an HR block, got {blocks:?}");
        // HR must land between the two paragraphs, not before/after both.
        let pos = has_hr.unwrap();
        assert!(pos > 0 && pos < blocks.len() - 1);
    }
}
