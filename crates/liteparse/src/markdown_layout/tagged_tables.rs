//! Tables read from a tagged PDF's structure tree.
//!
//! When a page carries `Table` / `TR` / `TD` tags, the producer already told
//! us the grid: each cell's marked-content ids identify exactly which text
//! runs belong to it. That beats inferring rows and columns from geometry for
//! multi-line cells, rowspanning labels and borderless layouts. Each tagged
//! table is only used when every text run in the lines it covers belongs to
//! it; anything less falls back to the geometric detectors.

use std::collections::HashMap;

use super::blocks::{Block, Cell};
use super::tables::TableRun;
use crate::types::{ProjectedLine, Rect, StructNode, TextItem};

/// A table must have at least this many rows with two or more filled cells,
/// so single-row and single-column layout tables stay with the heuristics.
const MIN_FILLED_ROWS: usize = 2;

struct TaggedTable {
    /// Row-major cells, each holding the mcids of its whole subtree.
    rows: Vec<Vec<Vec<i32>>>,
    /// Whether the first row is a header (all `TH`, or inside `THead`).
    header: bool,
}

fn subtree_end(nodes: &[StructNode], i: usize) -> usize {
    let depth = nodes[i].depth;
    let mut j = i + 1;
    while j < nodes.len() && nodes[j].depth > depth {
        j += 1;
    }
    j
}

fn read_tables(nodes: &[StructNode]) -> Vec<TaggedTable> {
    let mut out = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        if node.role != "Table" {
            continue;
        }
        let end = subtree_end(nodes, i);
        // A nested table's cells would be claimed by both; leave the outer
        // one to the heuristics and read the inner one on its own.
        if nodes[i + 1..end].iter().any(|n| n.role == "Table") {
            continue;
        }
        let mut rows = Vec::new();
        let mut first_row_header = false;
        let mut thead_end = 0;
        let mut j = i + 1;
        while j < end {
            let n = &nodes[j];
            match n.role.as_str() {
                "THead" => {
                    thead_end = subtree_end(nodes, j);
                    j += 1;
                }
                "TR" => {
                    let tr_end = subtree_end(nodes, j);
                    let mut cells = Vec::new();
                    let mut all_th = true;
                    let mut k = j + 1;
                    while k < tr_end {
                        let c = &nodes[k];
                        if c.depth == n.depth + 1 && (c.role == "TD" || c.role == "TH") {
                            let cell_end = subtree_end(nodes, k);
                            all_th &= c.role == "TH";
                            cells.push(
                                nodes[k..cell_end]
                                    .iter()
                                    .flat_map(|x| x.mcids.iter().copied())
                                    .collect(),
                            );
                            k = cell_end;
                        } else {
                            k += 1;
                        }
                    }
                    if rows.is_empty() {
                        first_row_header = !cells.is_empty() && (all_th || j < thead_end);
                    }
                    rows.push(cells);
                    j = tr_end;
                }
                _ => j += 1,
            }
        }
        out.push(TaggedTable {
            rows,
            header: first_row_header,
        });
    }
    out
}

fn has_text(span: &TextItem) -> bool {
    !span.text.trim().is_empty()
}

fn item_rect(span: &TextItem) -> Rect {
    Rect {
        x: span.x,
        y: span.y,
        width: span.width,
        height: span.height,
    }
}

/// Append one span to a cell's current line, adding a space only where the
/// glyphs are visibly apart (spans can split mid-word).
fn push_span(text: &mut String, last_right: &mut Option<f32>, span: &TextItem) {
    if let Some(right) = *last_right {
        let joined =
            text.ends_with(char::is_whitespace) || span.text.starts_with(char::is_whitespace);
        if !joined && span.x - right > span.height.max(1.0) * 0.2 {
            text.push(' ');
        }
    }
    text.push_str(&span.text);
    *last_right = Some(span.x + span.width);
}

fn cell_map(table: &TaggedTable) -> HashMap<i32, (usize, usize)> {
    let mut cell_of = HashMap::new();
    for (r, row) in table.rows.iter().enumerate() {
        for (c, mcids) in row.iter().enumerate() {
            for &m in mcids {
                cell_of.insert(m, (r, c));
            }
        }
    }
    cell_of
}

fn owns(cell_of: &HashMap<i32, (usize, usize)>, span: &TextItem) -> bool {
    span.mcid.is_some_and(|m| cell_of.contains_key(&m))
}

/// Horizontal extent of the table's own text across `lines`.
fn x_extent<'a>(
    lines: impl Iterator<Item = &'a ProjectedLine>,
    cell_of: &HashMap<i32, (usize, usize)>,
) -> (f32, f32) {
    let mut ext = (f32::INFINITY, f32::NEG_INFINITY);
    for s in lines
        .flat_map(|l| &l.spans)
        .filter(|s| has_text(s) && owns(cell_of, s))
    {
        ext.0 = ext.0.min(s.x);
        ext.1 = ext.1.max(s.x + s.width);
    }
    ext
}

/// Untagged text (artifacts such as margin page numbers) that shares a
/// baseline with a table row but sits wholly beside the table.
fn beside(span: &TextItem, ext: (f32, f32)) -> bool {
    span.mcid.is_none() && (span.x + span.width <= ext.0 || span.x >= ext.1)
}

/// When the XY-cut sliced a tagged table into several leaves, move all of its
/// lines into the first leaf (in y order) so `classify_region` sees it whole.
/// Tables whose lines also carry other text are left alone.
pub(super) fn regroup_split_tables(
    lines: &[ProjectedLine],
    nodes: &[StructNode],
) -> Option<Vec<ProjectedLine>> {
    if !nodes.iter().any(|n| n.role == "Table") {
        return None;
    }
    let mut cur: Option<Vec<ProjectedLine>> = None;
    for table in read_tables(nodes) {
        let cell_of = cell_map(&table);
        let src = cur.as_deref().unwrap_or(lines);
        let idx: Vec<usize> = (0..src.len())
            .filter(|&i| {
                src[i]
                    .spans
                    .iter()
                    .any(|s| has_text(s) && owns(&cell_of, s))
            })
            .collect();
        let Some(&first) = idx.first() else {
            continue;
        };
        let path = src[first].region_path.clone();
        let ext = x_extent(idx.iter().map(|&i| &src[i]), &cell_of);
        if idx.iter().all(|&i| src[i].region_path == path)
            || idx.iter().any(|&i| {
                src[i].rtl
                    || src[i]
                        .spans
                        .iter()
                        .any(|s| has_text(s) && !owns(&cell_of, s) && !beside(s, ext))
            })
        {
            continue;
        }
        let mut moved: Vec<ProjectedLine> = idx
            .iter()
            .map(|&i| {
                let mut l = src[i].clone();
                l.region_path = path.clone();
                l
            })
            .collect();
        moved.sort_by(|a, b| {
            a.bbox
                .y
                .total_cmp(&b.bbox.y)
                .then(a.bbox.x.total_cmp(&b.bbox.x))
        });
        let taken: std::collections::HashSet<usize> = idx.into_iter().collect();
        let mut next = Vec::with_capacity(src.len());
        for (i, l) in src.iter().enumerate() {
            if i == first {
                next.append(&mut moved);
            }
            if !taken.contains(&i) {
                next.push(l.clone());
            }
        }
        cur = Some(next);
    }
    cur
}

fn build_run(
    table: &TaggedTable,
    lines: &[ProjectedLine],
    page_lines: &[ProjectedLine],
) -> Option<TableRun> {
    let cell_of = cell_map(table);
    let owned = |span: &TextItem| owns(&cell_of, span);

    let on_page = page_lines
        .iter()
        .flat_map(|l| &l.spans)
        .filter(|s| has_text(s) && owned(s))
        .count();
    let mut first = None;
    let mut last = 0;
    let mut in_region = 0;
    for (i, line) in lines.iter().enumerate() {
        let n = line
            .spans
            .iter()
            .filter(|s| has_text(s) && owned(s))
            .count();
        if n > 0 {
            first.get_or_insert(i);
            last = i;
            in_region += n;
        }
    }
    let start = first?;
    let end = last + 1;
    if in_region != on_page {
        return None;
    }
    let covered = &lines[start..end];
    let ext = x_extent(covered.iter(), &cell_of);
    if covered.iter().any(|l| {
        l.rtl
            || l.spans
                .iter()
                .any(|s| has_text(s) && !owned(s) && !beside(s, ext))
    }) {
        return None;
    }
    let interstitials: Vec<Cell> = covered
        .iter()
        .flat_map(|l| &l.spans)
        .filter(|s| has_text(s) && !owned(s))
        .map(|s| Cell::located(s.text.trim(), item_rect(s)))
        .collect();

    let mut texts: Vec<Vec<String>> = table
        .rows
        .iter()
        .map(|r| vec![String::new(); r.len()])
        .collect();
    let mut boxes: Vec<Vec<Option<Rect>>> =
        table.rows.iter().map(|r| vec![None; r.len()]).collect();
    let mut bbox = None;
    for line in covered {
        let mut line_text: HashMap<(usize, usize), (String, Option<f32>)> = HashMap::new();
        let mut order = Vec::new();
        for span in line.spans.iter().filter(|s| has_text(s) && owned(s)) {
            let key = cell_of[&span.mcid.unwrap()];
            let entry = line_text.entry(key).or_insert_with(|| {
                order.push(key);
                (String::new(), None)
            });
            push_span(&mut entry.0, &mut entry.1, span);
            Rect::extend(&mut boxes[key.0][key.1], &item_rect(span));
        }
        for key in order {
            let piece = line_text[&key].0.trim().to_string();
            let cell = &mut texts[key.0][key.1];
            if !cell.is_empty() {
                cell.push(' ');
            }
            cell.push_str(&piece);
        }
        Rect::extend(&mut bbox, &line.bbox);
    }

    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut header_kept = false;
    for (r, (row_texts, row_boxes)) in texts.into_iter().zip(boxes).enumerate() {
        if row_texts.iter().all(|t| t.is_empty()) {
            continue;
        }
        if r == 0 {
            header_kept = table.header;
        }
        rows.push(
            row_texts
                .into_iter()
                .zip(row_boxes)
                .map(|(text, b)| Cell {
                    text: text.split_whitespace().collect::<Vec<_>>().join(" "),
                    bbox: b,
                })
                .collect(),
        );
    }
    let filled_rows = rows
        .iter()
        .filter(|row| row.iter().filter(|c| !c.text.is_empty()).count() >= 2)
        .count();
    if filled_rows < MIN_FILLED_ROWS {
        return None;
    }
    let ncols = rows.iter().map(Vec::len).max().unwrap_or(0);
    for row in &mut rows {
        row.resize_with(ncols, || Cell {
            text: String::new(),
            bbox: None,
        });
    }
    let header = if header_kept {
        Some(rows.remove(0))
    } else {
        None
    };
    Some(TableRun {
        start,
        end,
        body_start: start,
        block: Block::Table { header, rows },
        interstitials,
        bbox,
    })
}

/// Table runs for the tagged tables whose text lies wholly inside `lines` (one
/// layout region) and nowhere else on the page.
pub(super) fn detect_tagged_tables(
    lines: &[ProjectedLine],
    page_lines: &[ProjectedLine],
    nodes: &[StructNode],
) -> Vec<TableRun> {
    if !nodes.iter().any(|n| n.role == "Table") {
        return Vec::new();
    }
    let mut runs: Vec<TableRun> = read_tables(nodes)
        .iter()
        .filter_map(|t| build_run(t, lines, page_lines))
        .collect();
    runs.sort_by_key(|r| r.start);
    runs
}

/// Tagged runs replace any heuristic run they overlap.
pub(super) fn prefer_tagged(tagged: Vec<TableRun>, heuristic: Vec<TableRun>) -> Vec<TableRun> {
    if tagged.is_empty() {
        return heuristic;
    }
    let mut out: Vec<TableRun> = heuristic
        .into_iter()
        .filter(|h| !tagged.iter().any(|t| h.start < t.end && t.start < h.end))
        .collect();
    out.extend(tagged);
    out.sort_by_key(|r| r.start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown_layout::test_helpers::line;

    fn node(role: &str, depth: u16, mcids: &[i32]) -> StructNode {
        StructNode {
            role: role.into(),
            mcids: mcids.to_vec(),
            bbox: None,
            alt_text: None,
            depth,
        }
    }

    fn span(text: &str, x: f32, y: f32, mcid: Option<i32>) -> TextItem {
        TextItem {
            text: text.into(),
            x,
            y,
            width: text.len() as f32 * 5.0,
            height: 10.0,
            mcid,
            ..TextItem::default()
        }
    }

    fn row_line(y: f32, spans: Vec<TextItem>) -> ProjectedLine {
        let text = spans
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let mut l = line(&text, 50.0, y, 10.0, 10.0);
        l.mcid = spans.first().and_then(|s| s.mcid);
        l.spans = spans;
        l
    }

    /// Table > TR > (TD > P) x2, two rows; the first cell of row 2 wraps.
    fn two_by_two(head: &str) -> Vec<StructNode> {
        vec![
            node("Document", 0, &[]),
            node("P", 1, &[0]),
            node("Table", 1, &[]),
            node("TR", 2, &[]),
            node(head, 3, &[]),
            node("P", 4, &[1]),
            node(head, 3, &[2]),
            node("TR", 2, &[]),
            node("TD", 3, &[]),
            node("P", 4, &[3, 5]),
            node("TD", 3, &[4]),
        ]
    }

    fn table_lines() -> Vec<ProjectedLine> {
        vec![
            row_line(10.0, vec![span("Title", 50.0, 10.0, Some(0))]),
            row_line(
                30.0,
                vec![
                    span("Term", 50.0, 30.0, Some(1)),
                    span("Value", 200.0, 30.0, Some(2)),
                ],
            ),
            row_line(
                50.0,
                vec![
                    span("Contract", 50.0, 50.0, Some(3)),
                    span("8 years", 200.0, 50.0, Some(4)),
                ],
            ),
            row_line(62.0, vec![span("Expiry", 50.0, 62.0, Some(5))]),
        ]
    }

    fn cells(row: &[Cell]) -> Vec<&str> {
        row.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn builds_grid_from_tags_and_joins_wrapped_cell_lines() {
        let lines = table_lines();
        let runs = detect_tagged_tables(&lines, &lines, &two_by_two("TD"));
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!((run.start, run.end), (1, 4));
        let Block::Table { header, rows } = &run.block else {
            panic!("expected table");
        };
        assert!(header.is_none());
        assert_eq!(cells(&rows[0]), ["Term", "Value"]);
        assert_eq!(cells(&rows[1]), ["Contract Expiry", "8 years"]);
    }

    #[test]
    fn th_first_row_becomes_header() {
        let lines = table_lines();
        let runs = detect_tagged_tables(&lines, &lines, &two_by_two("TH"));
        let Block::Table { header, rows } = &runs[0].block else {
            panic!("expected table");
        };
        assert_eq!(cells(header.as_ref().unwrap()), ["Term", "Value"]);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn foreign_text_inside_the_table_lines_falls_back() {
        let mut lines = table_lines();
        let mut between = lines.clone();
        between[2].spans.insert(1, span("note", 120.0, 50.0, None));
        assert!(detect_tagged_tables(&between, &between, &two_by_two("TD")).is_empty());
        // Tagged text from another structure element, even beside the table.
        lines[2].spans.push(span("note", 300.0, 50.0, Some(99)));
        assert!(detect_tagged_tables(&lines, &lines, &two_by_two("TD")).is_empty());
    }

    #[test]
    fn margin_artifact_on_a_row_is_kept_beside_the_table() {
        let mut lines = table_lines();
        lines[2].spans.insert(0, span("3", 10.0, 50.0, None));
        let runs = detect_tagged_tables(&lines, &lines, &two_by_two("TD"));
        assert_eq!(runs.len(), 1);
        assert_eq!(cells(&runs[0].interstitials), ["3"]);
        let Block::Table { rows, .. } = &runs[0].block else {
            panic!("expected table");
        };
        assert_eq!(cells(&rows[1]), ["Contract Expiry", "8 years"]);
    }

    #[test]
    fn table_split_across_regions_falls_back() {
        let lines = table_lines();
        assert!(detect_tagged_tables(&lines[..3], &lines, &two_by_two("TD")).is_empty());
    }

    #[test]
    fn single_column_layout_table_is_left_to_heuristics() {
        let nodes = vec![
            node("Table", 0, &[]),
            node("TR", 1, &[]),
            node("TD", 2, &[1]),
            node("TR", 1, &[]),
            node("TD", 2, &[3]),
        ];
        let lines = table_lines();
        assert!(detect_tagged_tables(&lines[1..3], &lines[1..3], &nodes).is_empty());
    }

    #[test]
    fn split_table_is_regrouped_into_one_region() {
        let mut lines = table_lines();
        // XY-cut put the value column in its own leaf, after the first one.
        let mut right = Vec::new();
        for l in &mut lines[1..3] {
            let v = l.spans.pop().unwrap();
            let mut r = row_line(l.bbox.y, vec![v]);
            r.region_path = vec![1];
            right.push(r);
        }
        for l in &mut lines {
            l.region_path = vec![0];
        }
        lines.extend(right);
        let regrouped = regroup_split_tables(&lines, &two_by_two("TD")).unwrap();
        assert!(regrouped.iter().all(|l| l.region_path == vec![0]));
        let runs = detect_tagged_tables(&regrouped, &regrouped, &two_by_two("TD"));
        let Block::Table { rows, .. } = &runs[0].block else {
            panic!("expected table");
        };
        assert_eq!(cells(&rows[0]), ["Term", "Value"]);
        assert_eq!(cells(&rows[1]), ["Contract Expiry", "8 years"]);
    }

    #[test]
    fn untagged_page_has_no_runs() {
        let lines = table_lines();
        assert!(detect_tagged_tables(&lines, &lines, &[]).is_empty());
    }

    #[test]
    fn tagged_run_replaces_overlapping_heuristic_run() {
        let lines = table_lines();
        let tagged = detect_tagged_tables(&lines, &lines, &two_by_two("TD"));
        let heuristic = TableRun {
            start: 2,
            end: 4,
            body_start: 2,
            block: Block::Table {
                header: None,
                rows: vec![],
            },
            interstitials: vec![],
            bbox: None,
        };
        let merged = prefer_tagged(tagged, vec![heuristic]);
        assert_eq!(merged.len(), 1);
        assert_eq!((merged[0].start, merged[0].end), (1, 4));
    }
}
