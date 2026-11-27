//! Charts (`c:chart` graphic frames): data extraction into the worker's
//! `OpenXMLChart` shape (`formats/openxml-chart.ts`). Rendering lives in
//! `chart_render.rs`.

use std::rc::Rc;

use serde_json::{Value, json};
use tiny_skia::Transform;

use crate::office::pptx::scene::Rect;
use crate::office::pptx::slide::{EMU_PER_PT, SlideBuilder, rect_json};
use crate::office::pptx::xml::XNode;

/// One data series read from the chart's cached values.
#[derive(Debug, Clone, Default)]
pub struct Series {
    pub name: Option<String>,
    pub categories: Vec<String>,
    pub values: Vec<Option<f64>>,
    pub x_values: Vec<String>,
    /// Explicit series color (`c:spPr` solid fill), if any.
    pub sp_pr: Option<Rc<XNode>>,
    /// Per-point overrides (`c:dPt`), by point index.
    pub point_sp_pr: Vec<(usize, Rc<XNode>)>,
    pub index: usize,
}

#[derive(Debug, Clone)]
pub struct PlotGroup {
    /// Local name of the chart element (`barChart`, `lineChart`, `pieChart`, ...).
    pub kind: String,
    pub node: Rc<XNode>,
    pub series: Vec<Series>,
}

#[derive(Debug, Clone, Default)]
pub struct ChartData {
    pub title: Option<String>,
    pub groups: Vec<PlotGroup>,
    pub root: Option<Rc<XNode>>,
    pub part: String,
}

fn cache_values(node: Option<&Rc<XNode>>) -> Vec<String> {
    let Some(node) = node else { return Vec::new() };
    // numRef/strRef/multiLvlStrRef carry a cache; numLit/strLit are inline.
    let cache = node.children.iter().find_map(|c| match c.local() {
        "numRef" => c.child("c:numCache").cloned(),
        "strRef" => c.child("c:strCache").cloned(),
        "multiLvlStrRef" => c
            .child("c:multiLvlStrCache")
            .and_then(|m| m.children_named("c:lvl").next().cloned()),
        "numLit" | "strLit" => Some(c.clone()),
        _ => None,
    });
    let Some(cache) = cache else {
        return Vec::new();
    };
    let count = cache
        .child("c:ptCount")
        .and_then(|p| p.attr_i64("val"))
        .unwrap_or(0)
        .max(0) as usize;
    let mut pts: Vec<(usize, String)> = cache
        .children_named("c:pt")
        .map(|p| {
            (
                p.attr_i64("idx").unwrap_or(0).max(0) as usize,
                p.child("c:v").map(|v| v.text.clone()).unwrap_or_default(),
            )
        })
        .collect();
    let n = count
        .max(pts.iter().map(|(i, _)| i + 1).max().unwrap_or(0))
        .min(100_000);
    let mut out = vec![String::new(); n];
    for (i, v) in pts.drain(..) {
        if i < n {
            out[i] = v;
        }
    }
    out
}

pub fn read_chart(root: &Rc<XNode>, part: &str) -> ChartData {
    let chart = root.child("c:chart");
    let title = chart.and_then(|c| {
        if c.child("c:autoTitleDeleted")
            .and_then(|a| a.attr_bool("val"))
            == Some(true)
            && c.child("c:title").is_none()
        {
            return None;
        }
        let t = c.child("c:title")?;
        let mut texts = Vec::new();
        t.find_all("a:t", &mut texts);
        let s: String = texts.iter().map(|n| n.text.as_str()).collect();
        if s.trim().is_empty() {
            // A title element with no text shows the series name for single-series charts.
            None
        } else {
            Some(s)
        }
    });
    let mut groups = Vec::new();
    if let Some(plot) = chart.and_then(|c| c.child("c:plotArea")) {
        for g in &plot.children {
            if !g.name.starts_with("c:") || !g.local().ends_with("Chart") {
                continue;
            }
            let series = g
                .children_named("c:ser")
                .enumerate()
                .map(|(i, s)| {
                    let name = s.child("c:tx").and_then(|tx| {
                        cache_values(Some(tx))
                            .into_iter()
                            .next()
                            .or_else(|| tx.child("c:v").map(|v| v.text.clone()))
                    });
                    let cats = cache_values(s.child("c:cat"));
                    let vals_raw = cache_values(s.child("c:val").or(s.child("c:yVal")));
                    let x_values = cache_values(s.child("c:xVal"));
                    Series {
                        name,
                        categories: cats,
                        values: vals_raw
                            .iter()
                            .map(|v| v.trim().parse::<f64>().ok())
                            .collect(),
                        x_values,
                        sp_pr: s.child("c:spPr").cloned(),
                        point_sp_pr: s
                            .children_named("c:dPt")
                            .filter_map(|d| {
                                Some((
                                    d.child("c:idx")?.attr_i64("val")? as usize,
                                    d.child("c:spPr")?.clone(),
                                ))
                            })
                            .collect(),
                        index: s
                            .child("c:idx")
                            .and_then(|n| n.attr_i64("val"))
                            .map(|v| v as usize)
                            .unwrap_or(i),
                    }
                })
                .collect();
            groups.push(PlotGroup {
                kind: g.local().to_string(),
                node: g.clone(),
                series,
            });
        }
    }
    ChartData {
        title,
        groups,
        root: Some(root.clone()),
        part: part.to_string(),
    }
}

/// The `OpenXMLTable` JSON for a chart: a header row of series names, then one
/// row per category (or x value for XY charts).
fn table_json(data: &ChartData) -> Option<Value> {
    let series: Vec<&Series> = data.groups.iter().flat_map(|g| g.series.iter()).collect();
    if series.is_empty() {
        return None;
    }
    let cell =
        |s: &str| json!({ "content": [s], "colspan": 1, "vMerge": "", "isEmpty": s.is_empty() });
    let mut rows = Vec::new();
    let mut header = vec![cell("")];
    header.extend(series.iter().map(|s| cell(s.name.as_deref().unwrap_or(""))));
    rows.push(json!({ "cells": header, "isHeader": true }));
    let xy = data
        .groups
        .iter()
        .any(|g| g.kind == "scatterChart" || g.kind == "bubbleChart");
    let labels: Vec<String> = if xy {
        series
            .iter()
            .find(|s| !s.x_values.is_empty())
            .map(|s| s.x_values.clone())
            .unwrap_or_default()
    } else {
        series
            .iter()
            .find(|s| !s.categories.is_empty())
            .map(|s| s.categories.clone())
            .unwrap_or_default()
    };
    let n = labels
        .len()
        .max(series.iter().map(|s| s.values.len()).max().unwrap_or(0));
    for i in 0..n {
        let mut r = vec![cell(labels.get(i).map(String::as_str).unwrap_or(""))];
        for s in &series {
            let v = s
                .values
                .get(i)
                .copied()
                .flatten()
                .map(fmt_num)
                .unwrap_or_default();
            r.push(cell(&v));
        }
        rows.push(json!({ "cells": r, "isHeader": false }));
    }
    Some(json!({ "rows": rows, "caption": data.title }))
}

fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

pub fn build_chart(
    b: &mut SlideBuilder,
    data: &XNode,
    part: &str,
    tf: Transform,
    frame: Rect,
    bounds: Rect,
    idx: usize,
) {
    let rid = data.child("c:chart").and_then(|c| c.attr("r:id"));
    let chart_part = rid
        .and_then(|r| b.doc.pkg.rel_by_id(part, r))
        .map(|r| r.target);
    let Some(chart_part) = chart_part else {
        b.scene
            .warnings
            .push(("CHART_MISSING".into(), b.scene.shapes[idx].name.clone()));
        return;
    };
    let Some(root) = b.doc.xml(&chart_part) else {
        return;
    };
    emit_chart(b, &root, &chart_part, tf, frame, bounds, idx);
}

/// A chartEx frame (`cx:chart`: waterfall, treemap, ...). Returns false when
/// the chart is not drawn natively, so the caller draws the fallback picture.
pub fn build_chartex(
    b: &mut SlideBuilder,
    data: &XNode,
    part: &str,
    tf: Transform,
    frame: Rect,
    bounds: Rect,
    idx: usize,
) -> bool {
    let rid = data.child("cx:chart").and_then(|c| c.attr("r:id"));
    let Some(chart_part) = rid
        .and_then(|r| b.doc.pkg.rel_by_id(part, r))
        .map(|r| r.target)
    else {
        return false;
    };
    let Some(root) = b.doc.xml(&chart_part) else {
        return false;
    };
    let style = b
        .doc
        .pkg
        .rel_by_kind(&chart_part, "chartStyle")
        .and_then(|r| b.doc.xml(&r.target));
    match crate::office::pptx::chart_render::translate_chartex(&root, style.as_deref()) {
        Ok(synth) => {
            emit_chart(b, &synth, &chart_part, tf, frame, bounds, idx);
            true
        }
        Err(layout) => {
            let name = b.scene.shapes[idx].name.clone();
            b.scene
                .warnings
                .push(("CHART_PARTIAL".into(), format!("{name}: {layout}")));
            false
        }
    }
}

/// Records the chart's data (`charts[]`) and draws it.
fn emit_chart(
    b: &mut SlideBuilder,
    root: &Rc<XNode>,
    chart_part: &str,
    tf: Transform,
    frame: Rect,
    bounds: Rect,
    idx: usize,
) {
    let chart = read_chart(root, chart_part);
    let kinds: Vec<&str> = chart.groups.iter().map(|g| g.kind.as_str()).collect();
    let mut value = json!({
        "coords": rect_json(&bounds),
        "coordsEMU": { "x": frame.x * EMU_PER_PT, "y": frame.y * EMU_PER_PT, "w": frame.w * EMU_PER_PT, "h": frame.h * EMU_PER_PT },
        "description": kinds.join(", "),
        "tableDataSource": "cached",
    });
    if let Some(t) = &chart.title {
        value["title"] = json!(t);
    }
    if !b.cfg.presentation.skip_embedded_data
        && let Some(t) = table_json(&chart)
    {
        value["table"] = t;
    }
    b.scene.charts.push(value);
    crate::office::pptx::chart_render::render(b, &chart, chart_part, tf, frame.w, frame.h, idx);
}
