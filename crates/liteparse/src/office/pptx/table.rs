//! DrawingML tables (`a:tbl` inside a graphic frame): grid layout, merged cells,
//! table styles (`ppt/tableStyles.xml`) with banding and first/last row/column
//! parts, cell fills, borders and text.
//!
//! Rows grow to fit their text the way PowerPoint does; declared heights are a
//! minimum.

use std::rc::Rc;

use tiny_skia::Transform;

use crate::office::pptx::color::Rgba;
use crate::office::pptx::geom::TextRect;
use crate::office::pptx::scene::{Item, Paint, PathItem, Rect, Stroke, TableCell, TableInfo};
use crate::office::pptx::slide::{EMU_PER_PT, SlideBuilder};
use crate::office::pptx::style::{self, Fill, read_fill, read_line};
use crate::office::pptx::textprops;
use crate::office::pptx::xml::{self, XNode};

struct CellSpec {
    node: Rc<XNode>,
    row: usize,
    col: usize,
    row_span: usize,
    col_span: usize,
}

/// Style parts that apply to a cell, lowest priority first.
fn style_parts(
    flags: &Flags,
    row: usize,
    col: usize,
    nrows: usize,
    ncols: usize,
) -> Vec<&'static str> {
    let mut parts = vec!["a:wholeTbl"];
    let first_row = flags.first_row && row == 0;
    let last_row = flags.last_row && row + 1 == nrows;
    let first_col = flags.first_col && col == 0;
    let last_col = flags.last_col && col + 1 == ncols;
    if flags.band_col && !first_col && !last_col {
        let c = col - flags.first_col as usize;
        parts.push(if c.is_multiple_of(2) {
            "a:band1V"
        } else {
            "a:band2V"
        });
    }
    if flags.band_row && !first_row && !last_row {
        let r = row - flags.first_row as usize;
        parts.push(if r.is_multiple_of(2) {
            "a:band1H"
        } else {
            "a:band2H"
        });
    }
    if last_col {
        parts.push("a:lastCol");
    }
    if first_col {
        parts.push("a:firstCol");
    }
    if last_row {
        parts.push("a:lastRow");
    }
    if first_row {
        parts.push("a:firstRow");
    }
    if last_row && last_col {
        parts.push("a:seCell");
    }
    if last_row && first_col {
        parts.push("a:swCell");
    }
    if first_row && last_col {
        parts.push("a:neCell");
    }
    if first_row && first_col {
        parts.push("a:nwCell");
    }
    parts
}

#[derive(Default)]
struct Flags {
    first_row: bool,
    last_row: bool,
    first_col: bool,
    last_col: bool,
    band_row: bool,
    band_col: bool,
}

/// Default cell margins (EMU → pt): 0.1" left/right, 0.05" top/bottom.
const MAR_LR: f64 = 7.2;
const MAR_TB: f64 = 3.6;

pub fn build_table(
    b: &mut SlideBuilder,
    tbl: &XNode,
    part: &str,
    tf: Transform,
    _w: f64,
    _h: f64,
    idx: usize,
) {
    let tbl_pr = tbl.child("a:tblPr").cloned();
    let flag = |k: &str| {
        tbl_pr
            .as_ref()
            .and_then(|p| p.attr_bool(k))
            .unwrap_or(false)
    };
    let flags = Flags {
        first_row: flag("firstRow"),
        last_row: flag("lastRow"),
        first_col: flag("firstCol"),
        last_col: flag("lastCol"),
        band_row: flag("bandRow"),
        band_col: flag("bandCol"),
    };
    let style = table_style(b, tbl_pr.as_deref());
    let cols: Vec<f64> = tbl
        .child("a:tblGrid")
        .map(|g| {
            g.children_named("a:gridCol")
                .map(|c| c.attr_f64("w").unwrap_or(0.0) / EMU_PER_PT)
                .collect()
        })
        .unwrap_or_default();
    let rows: Vec<&Rc<XNode>> = tbl.children_named("a:tr").collect();
    if cols.is_empty() || rows.is_empty() {
        return;
    }
    let ncols = cols.len();
    let nrows = rows.len();
    let mut heights: Vec<f64> = rows
        .iter()
        .map(|r| r.attr_f64("h").unwrap_or(0.0) / EMU_PER_PT)
        .collect();

    // Cells that are not covered by a merge.
    let mut cells: Vec<CellSpec> = Vec::new();
    for (ri, tr) in rows.iter().enumerate() {
        for (ci, tc) in tr.children_named("a:tc").enumerate() {
            if ci >= ncols {
                break;
            }
            if tc.attr_bool("hMerge") == Some(true) || tc.attr_bool("vMerge") == Some(true) {
                continue;
            }
            let row_span = tc.attr_i64("rowSpan").unwrap_or(1).max(1) as usize;
            let col_span = tc.attr_i64("gridSpan").unwrap_or(1).max(1) as usize;
            cells.push(CellSpec {
                node: tc.clone(),
                row: ri,
                col: ci,
                row_span: row_span.min(nrows - ri),
                col_span: col_span.min(ncols - ci),
            });
        }
    }

    let col_x: Vec<f64> = std::iter::once(0.0)
        .chain(cols.iter().scan(0.0, |acc, w| {
            *acc += w;
            Some(*acc)
        }))
        .collect();

    // Grow rows to fit their text: single-row cells first, then cells spanning
    // several rows grow the last row they cover by whatever is still missing.
    let mut spanning: Vec<(usize, usize, f64)> = Vec::new();
    for c in &cells {
        let width = col_x[c.col + c.col_span] - col_x[c.col];
        let (lists, bodies, color, mar) = cell_text_setup(
            b,
            &c.node,
            style.as_ref(),
            &flags,
            c.row,
            c.col,
            nrows,
            ncols,
        );
        let Some(tx) = c.node.child("a:txBody").cloned() else {
            continue;
        };
        // Text wraps inside the cell margins, as it is drawn.
        let rect = TextRect {
            l: 0.0,
            t: 0.0,
            r: (width - mar.0 - mar.2).max(1.0),
            b: 10_000.0,
        };
        let needed = measure(b, &tx, &lists, &bodies, part, rect, color) + mar.1 + mar.3;
        if c.row_span == 1 {
            if needed > heights[c.row] {
                heights[c.row] = needed;
            }
        } else {
            spanning.push((c.row, c.row_span, needed));
        }
    }
    for (row, span, needed) in spanning {
        let have: f64 = heights[row..row + span].iter().sum();
        if needed > have {
            heights[row + span - 1] += needed - have;
        }
    }
    let row_y: Vec<f64> = std::iter::once(0.0)
        .chain(heights.iter().scan(0.0, |acc, h| {
            *acc += h;
            Some(*acc)
        }))
        .collect();

    let mut info_rows: Vec<Vec<TableCell>> = vec![Vec::new(); nrows];
    let mut borders: Vec<(f64, f64, f64, f64, Stroke)> = Vec::new();
    for c in &cells {
        let x0 = col_x[c.col];
        let x1 = col_x[c.col + c.col_span];
        let y0 = row_y[c.row];
        let y1 = row_y[c.row + c.row_span];
        let cell_rect = Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        };
        let tc_pr = c.node.child("a:tcPr").cloned();
        let parts = style_parts(&flags, c.row, c.col, nrows, ncols);
        let ctx = b.style_ctx(part);

        // Fill: cell tcPr, else the highest-priority style part that defines one.
        let mut fill: Option<Fill> = tc_pr
            .as_ref()
            .and_then(|p| style::fill_child(p))
            .map(|f| read_fill(f, &ctx, None, cell_rect));
        if fill.is_none()
            && let Some(st) = &style
        {
            for p in parts.iter().rev() {
                let Some(tc_style) = st.child(p).and_then(|s| s.child("a:tcStyle")) else {
                    continue;
                };
                if let Some(f) = tc_style.child("a:fill").and_then(|f| style::fill_child(f)) {
                    fill = Some(read_fill(f, &ctx, None, cell_rect));
                    break;
                }
                if let Some(fr) = tc_style.child("a:fillRef") {
                    fill = style::shape_fill(None, Some(fr), &ctx, cell_rect);
                    break;
                }
            }
        }
        if let Some(Fill::Paint(p)) = fill
            && let Some(path) = crate::office::pptx::textlayout::rect_path(x0, y0, x1 - x0, y1 - y0)
                .and_then(|p| p.transform(tf))
        {
            let paint = map_paint(p, &tf);
            b.scene.items.push(Item::Path(PathItem {
                path,
                fill: Some(paint),
                stroke: None,
                has_curve: false,
                shape: Some(idx),
                from_text: false,
            }));
        }

        // Borders: cell lnX, else style tcBdr edge (outer edge or inside line).
        let edges = [
            (
                "a:lnL",
                if c.col == 0 { "a:left" } else { "a:insideV" },
                (x0, y0, x0, y1),
            ),
            (
                "a:lnR",
                if c.col + c.col_span == ncols {
                    "a:right"
                } else {
                    "a:insideV"
                },
                (x1, y0, x1, y1),
            ),
            (
                "a:lnT",
                if c.row == 0 { "a:top" } else { "a:insideH" },
                (x0, y0, x1, y0),
            ),
            (
                "a:lnB",
                if c.row + c.row_span == nrows {
                    "a:bottom"
                } else {
                    "a:insideH"
                },
                (x0, y1, x1, y1),
            ),
        ];
        for (own, style_edge, seg) in edges {
            // wholeTbl names the table's outer edges and inside lines; every other part
            // (bands, first/last row and column) names the cell's own sides.
            let side = match own {
                "a:lnL" => "a:left",
                "a:lnR" => "a:right",
                "a:lnT" => "a:top",
                _ => "a:bottom",
            };
            let own_ln = tc_pr.as_ref().and_then(|p| p.child(own).cloned());
            let mut stroke = None;
            let mut explicit_none = false;
            if let Some(ln) = &own_ln {
                if ln.child("a:noFill").is_some() {
                    explicit_none = true;
                } else {
                    stroke = read_line(Some(ln), None, &ctx, cell_rect);
                }
            }
            if stroke.is_none()
                && !explicit_none
                && let Some(st) = &style
            {
                for p in parts.iter().rev() {
                    let Some(edge) = st
                        .child(p)
                        .and_then(|s| s.path(&["a:tcStyle", "a:tcBdr"]))
                        .and_then(|bdr| {
                            bdr.child(if *p == "a:wholeTbl" { style_edge } else { side })
                        })
                    else {
                        continue;
                    };
                    stroke = read_line(
                        edge.child("a:ln").map(|n| n.as_ref()),
                        edge.child("a:lnRef").map(|n| n.as_ref()),
                        &ctx,
                        cell_rect,
                    );
                    break;
                }
            }
            if let Some(s) = stroke {
                borders.push((seg.0, seg.1, seg.2, seg.3, s));
            }
        }

        // Text.
        let (lists, bodies, color, mar) = cell_text_setup(
            b,
            &c.node,
            style.as_ref(),
            &flags,
            c.row,
            c.col,
            nrows,
            ncols,
        );
        let text = c
            .node
            .child("a:txBody")
            .map(|t| crate::office::pptx::slide::para_texts(t))
            .unwrap_or_default();
        if let Some(tx) = c.node.child("a:txBody").cloned() {
            let rect = TextRect {
                l: x0 + mar.0,
                t: y0 + mar.1,
                r: x1 - mar.2,
                b: y1 - mar.3,
            };
            b.layout_text(&tx, &lists, &bodies, part, rect, tf, color, None, idx);
        }
        let bounds =
            crate::office::pptx::textlayout::transformed_bbox(&tf, x0, y0, x1 - x0, y1 - y0)
                .unwrap_or(cell_rect);
        info_rows[c.row].push(TableCell {
            text,
            bounds,
            row_span: c.row_span as u32,
            col_span: c.col_span as u32,
        });
    }
    for (x0, y0, x1, y1, s) in borders {
        let mut pb = tiny_skia::PathBuilder::new();
        pb.move_to(x0 as f32, y0 as f32);
        pb.line_to(x1 as f32, y1 as f32);
        let Some(path) = pb.finish().and_then(|p| p.transform(tf)) else {
            continue;
        };
        let mut s = s;
        s.paint = map_paint(s.paint, &tf);
        b.scene.items.push(Item::Path(PathItem {
            path,
            fill: None,
            stroke: Some(s),
            has_curve: false,
            shape: Some(idx),
            from_text: false,
        }));
    }
    let bounds = crate::office::pptx::textlayout::transformed_bbox(
        &tf,
        0.0,
        0.0,
        col_x[ncols],
        row_y[nrows],
    )
    .unwrap_or(b.scene.shapes[idx].bounds);
    b.scene.shapes[idx].bounds = bounds;
    b.scene.tables.push(TableInfo {
        shape: idx,
        bounds,
        rows: info_rows,
    });
}

fn map_paint(p: Paint, tf: &Transform) -> Paint {
    match p {
        Paint::Linear { start, end, stops } => {
            let mut pts = [
                tiny_skia::Point::from_xy(start.0 as f32, start.1 as f32),
                tiny_skia::Point::from_xy(end.0 as f32, end.1 as f32),
            ];
            tf.map_points(&mut pts);
            Paint::Linear {
                start: (pts[0].x as f64, pts[0].y as f64),
                end: (pts[1].x as f64, pts[1].y as f64),
                stops,
            }
        }
        Paint::Path(g) => Paint::Path(Box::new((*g).transformed(tf))),
        other => other,
    }
}

type CellText = (
    Vec<Rc<XNode>>,
    Vec<Rc<XNode>>,
    Option<Rgba>,
    (f64, f64, f64, f64),
);

#[expect(
    clippy::too_many_arguments,
    reason = "table styles pick cell text by row, column and the table's size and flags"
)]
fn cell_text_setup(
    b: &SlideBuilder,
    tc: &XNode,
    style: Option<&Rc<XNode>>,
    flags: &Flags,
    row: usize,
    col: usize,
    nrows: usize,
    ncols: usize,
) -> CellText {
    let mut lists: Vec<Rc<XNode>> = Vec::new();
    // Table text starts from the master's "other" text style (18pt in Office
    // templates), not the presentation default meant for text boxes.
    if let Some(other) = b
        .chain
        .master
        .as_ref()
        .and_then(|m| m.path(&["p:txStyles", "p:otherStyle"]))
    {
        lists.push(other.clone());
    }
    let ctx = b.style_ctx("");
    let mut color = None;
    if let Some(st) = style {
        // Fold tcTxStyle parts (bold/italic/font/color) into a synthetic list style.
        let mut bold = None;
        let mut italic = None;
        let mut font: Option<String> = None;
        for p in style_parts(flags, row, col, nrows, ncols) {
            let Some(tx) = st.child(p).and_then(|s| s.child("a:tcTxStyle")) else {
                continue;
            };
            if let Some(v) = tx.attr("b") {
                bold = Some(v == "on");
            }
            if let Some(v) = tx.attr("i") {
                italic = Some(v == "on");
            }
            if let Some(fr) = tx.child("a:fontRef") {
                font = match fr.attr("idx") {
                    Some("major") => Some("+mj-lt".into()),
                    _ => Some("+mn-lt".into()),
                };
                if let Some(c) = ctx.color(fr, None) {
                    color = Some(c);
                }
            }
            if let Some(c) = tx
                .children
                .iter()
                .find(|c| c.name.ends_with("Clr"))
                .and_then(|c| ctx.color(c, None))
            {
                color = Some(c);
            }
        }
        let mut attrs = String::new();
        if let Some(v) = bold {
            attrs.push_str(&format!(r#" b="{}""#, v as u8));
        }
        if let Some(v) = italic {
            attrs.push_str(&format!(r#" i="{}""#, v as u8));
        }
        // The style color sits above the master's otherStyle color (often tx1,
        // which a dark layout maps to white), so it goes into the list too.
        let mut inner = color.map(textprops::solid_fill_xml).unwrap_or_default();
        if let Some(f) = font {
            inner.push_str(&format!(r#"<a:latin typeface="{f}"/>"#));
        }
        lists.extend(textprops::uniform_list_style(&attrs, &inner));
    }
    if let Some(ls) = tc.path(&["a:txBody", "a:lstStyle"]) {
        lists.push(ls.clone());
    }
    let tc_pr = tc.child("a:tcPr");
    let m = |k: &str, d: f64| {
        tc_pr
            .and_then(|p| p.attr_f64(k))
            .map(|v| v / EMU_PER_PT)
            .unwrap_or(d)
    };
    let mar = (
        m("marL", MAR_LR),
        m("marT", MAR_TB),
        m("marR", MAR_LR),
        m("marB", MAR_TB),
    );
    // Cell margins are applied by the caller, so the body has zero insets; the
    // vertical anchor comes from tcPr.
    let anchor = tc_pr.and_then(|p| p.attr("anchor")).unwrap_or("t");
    let vert = tc_pr
        .and_then(|p| p.attr("vert"))
        .map(|v| format!(r#" vert="{v}""#))
        .unwrap_or_default();
    let body_xml = format!(
        r#"<a:bodyPr xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" lIns="0" tIns="0" rIns="0" bIns="0" anchor="{anchor}"{vert}/>"#
    );
    let mut bodies = Vec::new();
    if let Ok(n) = xml::parse(&body_xml) {
        bodies.push(n);
    }
    (lists, bodies, color, mar)
}

fn measure(
    b: &mut SlideBuilder,
    tx: &Rc<XNode>,
    lists: &[Rc<XNode>],
    bodies: &[Rc<XNode>],
    part: &str,
    rect: TextRect,
    color: Option<Rgba>,
) -> f64 {
    // Lay out into a scratch scene and keep only the height.
    let saved = std::mem::take(&mut b.scene);
    b.scene.shapes.push(crate::office::pptx::scene::ShapeInfo {
        id: String::new(),
        name: String::new(),
        parent: None,
        kind: "measure",
        placeholder: None,
        bounds: Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        },
        rotation: 0.0,
        alt_text: None,
        hyperlink: None,
        paragraphs: Vec::new(),
        inherited: false,
        hidden: false,
    });
    // A row holding only empty cells still fits their empty lines, as in PowerPoint.
    b.measure_empty = true;
    let h = b.layout_text(
        tx,
        lists,
        bodies,
        part,
        rect,
        Transform::identity(),
        color,
        None,
        0,
    );
    b.measure_empty = false;
    b.scene = saved;
    h
}

/// Finds the table style: `a:tableStyleId` in `ppt/tableStyles.xml`, or the
/// list's default.
fn table_style(b: &SlideBuilder, tbl_pr: Option<&XNode>) -> Option<Rc<XNode>> {
    if let Some(inline) = tbl_pr.and_then(|p| p.child("a:tableStyle")) {
        return Some(inline.clone());
    }
    let id = tbl_pr
        .and_then(|p| p.child("a:tableStyleId"))
        .map(|n| n.text.trim().to_string());
    let styles_part = b
        .doc
        .pkg
        .rel_by_kind(&b.doc.pres_part, "tableStyles")
        .map(|r| r.target)
        .unwrap_or_else(|| "ppt/tableStyles.xml".to_string());
    let list = b.doc.xml(&styles_part);
    let wanted = id.or_else(|| {
        list.as_ref()
            .and_then(|l| l.attr("def").map(str::to_string))
    })?;
    let found = list.and_then(|l| {
        l.children_named("a:tblStyle")
            .find(|s| s.attr("styleId") == Some(wanted.as_str()))
            .cloned()
    });
    // Built-in styles are not always written to tableStyles.xml.
    found.or_else(|| builtin_table_style(&wanted))
}

/// PowerPoint's 74 built-in table styles, keyed by their fixed GUIDs. PowerPoint does
/// not always write a used built-in style to tableStyles.xml; the definitions are the
/// majority form PowerPoint itself wrote into the decks of the evaluation corpus.
const PRESET_TABLE_STYLES: &str = include_str!("assets/presetTableStyles.xml");

thread_local! {
    static PRESETS: std::collections::HashMap<String, Rc<XNode>> = xml::parse(PRESET_TABLE_STYLES)
        .map(|l| {
            l.children_named("a:tblStyle")
                .filter_map(|s| Some((s.attr("styleId")?.to_string(), s.clone())))
                .collect()
        })
        .unwrap_or_default();
}

fn builtin_table_style(id: &str) -> Option<Rc<XNode>> {
    PRESETS.with(|p| p.get(id).cloned())
}

#[cfg(test)]
mod tests;
