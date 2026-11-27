//! chartEx charts (`cx:chartSpace`, the Office 2016 chart types).
//!
//! `translate` rewrites a chartEx part as a synthetic `c:chartSpace`, so the
//! classic renderer supplies the title, legend, axes and data labels. Layouts
//! with no classic equivalent become `c:cx*Chart` groups drawn here (waterfall,
//! funnel, box & whisker, treemap, sunburst); histograms and Pareto charts
//! become ordinary bar and line groups over binned or aggregated data.
//!
//! chartEx parts usually omit formatting that the chart style part
//! (`chartStyle`, `cs:` entries) supplies: axis lines, gridlines and text
//! colors and sizes are taken from it when the element has none of its own.

use std::collections::HashMap;
use std::rc::Rc;

use super::*;

// ---------------------------------------------------------------------------
// Translation
// ---------------------------------------------------------------------------

/// One `cx:data` block: category levels (leaf level first, as stored) and values.
#[derive(Default, Clone)]
struct CxData {
    cats: Vec<Vec<String>>,
    cat_fmt: Option<String>,
    vals: Vec<Option<f64>>,
    fmt: Option<String>,
}

fn lvl_points(lvl: &XNode) -> Vec<String> {
    let n = lvl.attr_i64("ptCount").unwrap_or(0).clamp(0, 100_000) as usize;
    let mut out = vec![String::new(); n];
    for p in lvl.children_named("cx:pt") {
        let i = p.attr_i64("idx").unwrap_or(0).max(0) as usize;
        if i >= 100_000 {
            continue;
        }
        if i >= out.len() {
            out.resize(i + 1, String::new());
        }
        out[i] = p.text.clone();
    }
    out
}

fn read_data(root: &XNode) -> HashMap<String, CxData> {
    let mut out = HashMap::new();
    let Some(cd) = root.child("cx:chartData") else {
        return out;
    };
    for d in cd.children_named("cx:data") {
        let mut data = CxData::default();
        for dim in &d.children {
            let ty = dim.attr("type").unwrap_or("");
            let lvls: Vec<&Rc<XNode>> = dim.children_named("cx:lvl").collect();
            match (dim.name.as_str(), ty) {
                ("cx:strDim" | "cx:numDim", "cat") => {
                    data.cats = lvls.iter().map(|l| lvl_points(l)).collect();
                    if dim.name == "cx:numDim" {
                        data.cat_fmt = lvls
                            .first()
                            .and_then(|l| l.attr("formatCode"))
                            .map(str::to_string);
                    }
                }
                ("cx:numDim", "val" | "size" | "y") => {
                    if let Some(l) = lvls.first() {
                        data.vals = lvl_points(l)
                            .iter()
                            .map(|v| v.trim().parse::<f64>().ok().filter(|x| x.is_finite()))
                            .collect();
                        data.fmt = l.attr("formatCode").map(str::to_string);
                    }
                }
                _ => {}
            }
        }
        out.insert(d.attr("id").unwrap_or("0").to_string(), data);
    }
    out
}

fn is_color(n: &XNode) -> bool {
    matches!(
        n.name.as_str(),
        "a:srgbClr" | "a:schemeClr" | "a:sysClr" | "a:prstClr" | "a:hslClr" | "a:scrgbClr"
    )
}

fn is_fill(n: &XNode) -> bool {
    matches!(
        n.name.as_str(),
        "a:solidFill" | "a:noFill" | "a:gradFill" | "a:pattFill" | "a:blipFill"
    )
}

fn renamed(n: &XNode, name: &str) -> Rc<XNode> {
    Rc::new(XNode {
        name: name.to_string(),
        attrs: n.attrs.clone(),
        children: n.children.clone(),
        text: n.text.clone(),
    })
}

fn v_node(name: &str, v: impl ToString) -> Rc<XNode> {
    node(name, vec![("val", v.to_string())], vec![])
}

/// The chart style's entry for an element (`categoryAxis`, `title`, ...).
fn style_entry<'s>(style: Option<&'s XNode>, name: &str) -> Option<&'s XNode> {
    style?
        .children
        .iter()
        .find(|c| c.local() == name)
        .map(|c| c.as_ref())
}

/// An `a:ln` for an axis or gridline: the element's own line, else the style's
/// `spPr` line or its `lnRef` color, else no line.
fn style_line(own: Option<&XNode>, entry: Option<&XNode>) -> Rc<XNode> {
    if let Some(ln) = own.and_then(|s| s.child("a:ln"))
        && ln.children.iter().any(|c| is_fill(c))
    {
        return ln.clone();
    }
    let sp_ln = entry
        .and_then(|e| e.children.iter().find(|c| c.local() == "spPr"))
        .and_then(|s| s.child("a:ln"));
    if let Some(ln) = sp_ln.filter(|l| l.children.iter().any(|c| is_fill(c))) {
        return ln.clone();
    }
    let lnref = entry.and_then(|e| e.children.iter().find(|c| c.local() == "lnRef"));
    let color = lnref
        .filter(|r| r.attr_i64("idx").unwrap_or(0) > 0)
        .and_then(|r| r.children.iter().find(|c| is_color(c)).cloned());
    match color {
        Some(c) => node(
            "a:ln",
            vec![("w", "9525".into())],
            vec![node("a:solidFill", vec![], vec![c])],
        ),
        None => node("a:ln", vec![], vec![node("a:noFill", vec![], vec![])]),
    }
}

/// A `c:txPr` layering the element's own text properties over the chart
/// style's font color and size for that element.
fn merged_txpr(own: Option<&XNode>, entry: Option<&XNode>, title: bool) -> Option<Rc<XNode>> {
    let own_ppr = own
        .and_then(|t| t.child("a:p"))
        .and_then(|p| p.child("a:pPr"));
    let own_def = own_ppr.and_then(|p| p.child("a:defRPr"));
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut kids: Vec<Rc<XNode>> = Vec::new();
    if let Some(e) = entry {
        if let Some(d) = e.children.iter().find(|c| c.local() == "defRPr") {
            attrs = d.attrs.clone();
            kids.extend(
                d.children
                    .iter()
                    .filter(|c| c.name.starts_with("a:"))
                    .cloned(),
            );
        }
        let font = e.children.iter().find(|c| c.local() == "fontRef");
        if let Some(c) = font.and_then(|f| f.children.iter().find(|c| is_color(c))) {
            kids.retain(|k| !is_fill(k));
            kids.insert(0, node("a:solidFill", vec![], vec![c.clone()]));
        }
    }
    if title && !attrs.iter().any(|(k, _)| k == "b") {
        attrs.push(("b".into(), "0".into()));
    }
    if let Some(d) = own_def {
        for (k, v) in &d.attrs {
            match attrs.iter_mut().find(|(a, _)| a == k) {
                Some(slot) => slot.1 = v.clone(),
                None => attrs.push((k.clone(), v.clone())),
            }
        }
        for c in &d.children {
            if is_fill(c) {
                kids.retain(|k| !is_fill(k));
            } else {
                kids.retain(|k| k.name != c.name);
            }
            kids.push(c.clone());
        }
    }
    if own.is_none() && attrs.is_empty() && kids.is_empty() {
        return None;
    }
    let def = Rc::new(XNode {
        name: "a:defRPr".into(),
        attrs,
        children: kids,
        text: String::new(),
    });
    let ppr = Rc::new(XNode {
        name: "a:pPr".into(),
        attrs: own_ppr.map(|p| p.attrs.clone()).unwrap_or_default(),
        children: vec![def],
        text: String::new(),
    });
    let body = own
        .and_then(|t| t.child("a:bodyPr").cloned())
        .unwrap_or_else(|| node("a:bodyPr", vec![], vec![]));
    Some(node(
        "c:txPr",
        vec![],
        vec![
            body,
            node("a:lstStyle", vec![], vec![]),
            node("a:p", vec![], vec![ppr]),
        ],
    ))
}

fn str_ref(values: &[String]) -> Rc<XNode> {
    let mut kids = vec![v_node("c:ptCount", values.len())];
    for (i, v) in values.iter().enumerate() {
        kids.push(node(
            "c:pt",
            vec![("idx", i.to_string())],
            vec![text_node("c:v", v)],
        ));
    }
    node("c:strRef", vec![], vec![node("c:strCache", vec![], kids)])
}

fn num_ref(values: &[Option<f64>], fmt: &str) -> Rc<XNode> {
    let mut kids = vec![
        text_node("c:formatCode", fmt),
        v_node("c:ptCount", values.len()),
    ];
    for (i, v) in values.iter().enumerate() {
        if let Some(v) = v {
            kids.push(node(
                "c:pt",
                vec![("idx", i.to_string())],
                vec![text_node("c:v", &format!("{v}"))],
            ));
        }
    }
    node("c:numRef", vec![], vec![node("c:numCache", vec![], kids)])
}

/// Title (chart or axis) from a `cx:title`.
fn title_node(t: &XNode, style: Option<&XNode>, entry: &str) -> Rc<XNode> {
    let st = style_entry(style, entry);
    let own_tx = t.child("cx:txPr");
    let txpr = merged_txpr(own_tx.map(|n| n.as_ref()), st, true);
    let mut kids = Vec::new();
    let rich = t.path(&["cx:tx", "cx:rich"]);
    let text = t
        .path(&["cx:tx", "cx:txData", "cx:v"])
        .map(|v| v.text.clone());
    if let Some(r) = rich {
        kids.push(node("c:tx", vec![], vec![renamed(r, "c:rich")]));
    } else if own_tx.is_some_and(|tp| !tp.all_text().trim().is_empty()) {
        // The title text is often stored as runs inside its txPr.
        let r = own_tx.map(|tp| renamed(tp, "c:rich"));
        kids.extend(r.map(|r| node("c:tx", vec![], vec![r])));
    } else if let Some(text) = text {
        kids.push(node("c:tx", vec![], vec![str_ref(&[text])]));
    }
    kids.push(v_node("c:overlay", t.attr("overlay").unwrap_or("0")));
    kids.extend(txpr);
    if let Some(sp) = t.child("cx:spPr") {
        kids.push(renamed(sp, "c:spPr"));
    }
    node("c:title", vec![], kids)
}

struct AxisSpec<'x> {
    cx: Option<&'x XNode>,
    id: &'static str,
    cross: &'static str,
    cat: bool,
    pos: &'static str,
    hidden: bool,
    reversed: bool,
    crosses_max: bool,
    pct: bool,
}

fn axis_node(a: &AxisSpec, style: Option<&XNode>) -> Rc<XNode> {
    let cx = a.cx;
    let entry = style_entry(style, if a.cat { "categoryAxis" } else { "valueAxis" });
    let mut scaling = vec![v_node(
        "c:orientation",
        if a.reversed { "maxMin" } else { "minMax" },
    )];
    let vs = cx.and_then(|n| n.child("cx:valScaling"));
    let num = |k: &str| vs.and_then(|s| s.attr_f64(k)).filter(|v| v.is_finite());
    if let Some(v) = num("max").or(if a.pct { Some(1.0) } else { None }) {
        scaling.push(v_node("c:max", v));
    }
    if let Some(v) = num("min").or(if a.pct { Some(0.0) } else { None }) {
        scaling.push(v_node("c:min", v));
    }
    let hidden = a.hidden || cx.and_then(|n| n.attr_bool("hidden")).unwrap_or(false);
    let mut kids = vec![
        v_node("c:axId", a.id),
        node("c:scaling", vec![], scaling),
        v_node("c:delete", if hidden { "1" } else { "0" }),
        v_node("c:axPos", a.pos),
    ];
    for (cx_name, c_name) in [
        ("cx:majorGridlines", "c:majorGridlines"),
        ("cx:minorGridlines", "c:minorGridlines"),
    ] {
        if let Some(g) = cx.and_then(|n| n.child(cx_name)) {
            let ge = style_entry(
                style,
                if c_name == "c:majorGridlines" {
                    "gridlineMajor"
                } else {
                    "gridlineMinor"
                },
            );
            let ln = style_line(g.child("cx:spPr").map(|s| s.as_ref()), ge);
            kids.push(node(c_name, vec![], vec![node("c:spPr", vec![], vec![ln])]));
        }
    }
    if let Some(t) = cx.and_then(|n| n.child("cx:title")) {
        kids.push(title_node(t, style, "axisTitle"));
    }
    match cx.and_then(|n| n.child("cx:numFmt")) {
        Some(f) => kids.push(node(
            "c:numFmt",
            vec![
                (
                    "formatCode",
                    f.attr("formatCode").unwrap_or("General").into(),
                ),
                ("sourceLinked", f.attr("sourceLinked").unwrap_or("0").into()),
            ],
            vec![],
        )),
        None if a.pct => kids.push(node(
            "c:numFmt",
            vec![("formatCode", "0%".into()), ("sourceLinked", "0".into())],
            vec![],
        )),
        None => {}
    }
    let tick = cx
        .and_then(|n| n.child("cx:majorTickMarks"))
        .and_then(|t| t.attr("type"))
        .unwrap_or("none");
    kids.push(v_node("c:majorTickMark", tick));
    kids.push(v_node("c:minorTickMark", "none"));
    let labels = cx.is_some_and(|n| n.child("cx:tickLabels").is_some()) || a.pct;
    // chartEx category labels always sit at the low end, wherever zero is.
    kids.push(v_node(
        "c:tickLblPos",
        match (labels, a.cat) {
            (false, _) => "none",
            (true, true) => "low",
            (true, false) => "nextTo",
        },
    ));
    let own_sp = cx.and_then(|n| n.child("cx:spPr")).map(|s| s.as_ref());
    let mut sp_kids: Vec<Rc<XNode>> = own_sp
        .map(|s| {
            s.children
                .iter()
                .filter(|c| c.name != "a:ln")
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    sp_kids.push(style_line(own_sp, entry));
    kids.push(node("c:spPr", vec![], sp_kids));
    kids.extend(merged_txpr(
        cx.and_then(|n| n.child("cx:txPr")).map(|t| t.as_ref()),
        entry,
        false,
    ));
    kids.push(v_node("c:crossAx", a.cross));
    if a.crosses_max {
        kids.push(v_node("c:crosses", "max"));
    }
    if a.cat {
        return node("c:catAx", vec![], kids);
    }
    kids.push(v_node("c:crossBetween", "between"));
    if let Some(u) = num("majorUnit").filter(|u| *u > 0.0) {
        kids.push(v_node("c:majorUnit", u));
    }
    node("c:valAx", vec![], kids)
}

/// `c:dLbls` from a `cx:dataLabels`.
fn data_labels(d: &XNode, style: Option<&XNode>) -> Rc<XNode> {
    let entry = style_entry(style, "dataLabel");
    let flags = |n: &XNode| -> Vec<Rc<XNode>> {
        let vis = n.child("cx:visibility");
        let f = |k: &str, def: bool| {
            let on = vis.and_then(|v| v.attr_bool(k)).unwrap_or(def);
            if on { "1" } else { "0" }
        };
        vec![
            v_node("c:showLegendKey", "0"),
            v_node("c:showVal", f("value", true)),
            v_node("c:showCatName", f("categoryName", false)),
            v_node("c:showSerName", f("seriesName", false)),
            v_node("c:showPercent", "0"),
        ]
    };
    let mut kids = Vec::new();
    for l in d.children_named("cx:dataLabel") {
        let mut lk = vec![v_node("c:idx", l.attr("idx").unwrap_or("0"))];
        let tp = l.child("cx:txPr");
        if let Some(tp) = tp.filter(|t| !t.all_text().trim().is_empty()) {
            lk.push(node("c:tx", vec![], vec![renamed(tp, "c:rich")]));
        }
        lk.extend(merged_txpr(tp.map(|t| t.as_ref()), entry, false));
        lk.extend(flags(l));
        if let Some(s) = l.child("cx:separator") {
            lk.push(text_node("c:separator", &s.text));
        }
        kids.push(node("c:dLbl", vec![], lk));
    }
    for h in d.children_named("cx:dataLabelHidden") {
        kids.push(node(
            "c:dLbl",
            vec![],
            vec![
                v_node("c:idx", h.attr("idx").unwrap_or("0")),
                v_node("c:delete", "1"),
            ],
        ));
    }
    if let Some(f) = d.child("cx:numFmt") {
        kids.push(node(
            "c:numFmt",
            vec![
                (
                    "formatCode",
                    f.attr("formatCode").unwrap_or("General").into(),
                ),
                ("sourceLinked", f.attr("sourceLinked").unwrap_or("0").into()),
            ],
            vec![],
        ));
    }
    if let Some(sp) = d.child("cx:spPr") {
        kids.push(renamed(sp, "c:spPr"));
    }
    kids.extend(merged_txpr(
        d.child("cx:txPr").map(|t| t.as_ref()),
        entry,
        false,
    ));
    if let Some(p) = d.attr("pos") {
        kids.push(v_node("c:dLblPos", p));
    }
    kids.extend(flags(d));
    if let Some(s) = d.child("cx:separator") {
        kids.push(text_node("c:separator", &s.text));
    }
    node("c:dLbls", vec![], kids)
}

/// Series formatting shared by every layout: name, fill, point overrides, labels.
fn ser_common(s: &XNode, k: usize, style: Option<&XNode>) -> Vec<Rc<XNode>> {
    let mut kids = vec![v_node("c:idx", k), v_node("c:order", k)];
    if let Some(name) = s.path(&["cx:tx", "cx:txData", "cx:v"]) {
        kids.push(node(
            "c:tx",
            vec![],
            vec![str_ref(std::slice::from_ref(&name.text))],
        ));
    }
    if let Some(sp) = s.child("cx:spPr") {
        kids.push(renamed(sp, "c:spPr"));
    }
    for p in s.children_named("cx:dataPt") {
        let mut pk = vec![v_node("c:idx", p.attr("idx").unwrap_or("0"))];
        if let Some(sp) = p.child("cx:spPr") {
            pk.push(renamed(sp, "c:spPr"));
        }
        kids.push(node("c:dPt", vec![], pk));
    }
    if let Some(d) = s.child("cx:dataLabels") {
        kids.push(data_labels(d, style));
    }
    kids
}

fn cat_val(cats: &[String], vals: &[Option<f64>], fmt: &str) -> Vec<Rc<XNode>> {
    let mut out = Vec::new();
    if !cats.is_empty() {
        out.push(node("c:cat", vec![], vec![str_ref(cats)]));
    }
    out.push(node("c:val", vec![], vec![num_ref(vals, fmt)]));
    out
}

/// Rewrites a chartEx part as a classic `c:chartSpace`. `Err` names a layout
/// that is not drawn natively (region maps, unknown layouts).
pub(crate) fn translate(root: &XNode, style: Option<&XNode>) -> Result<Rc<XNode>, String> {
    let data = read_data(root);
    let chart = root.child("cx:chart").ok_or_else(|| "empty".to_string())?;
    let region = chart
        .path(&["cx:plotArea", "cx:plotAreaRegion"])
        .ok_or_else(|| "empty".to_string())?;
    let all: Vec<&Rc<XNode>> = region.children_named("cx:series").collect();
    let visible: Vec<&Rc<XNode>> = all
        .iter()
        .copied()
        .filter(|s| s.attr_bool("hidden") != Some(true))
        .collect();
    let layout = visible
        .first()
        .and_then(|s| s.attr("layoutId"))
        .unwrap_or("")
        .to_string();
    let axes: Vec<&Rc<XNode>> = chart
        .child("cx:plotArea")
        .map(|p| p.children_named("cx:axis").collect())
        .unwrap_or_default();
    let ax = |id: &str| {
        axes.iter()
            .find(|a| a.attr("id") == Some(id))
            .map(|a| a.as_ref())
    };
    let series_data = |s: &XNode| {
        let id = s
            .child("cx:dataId")
            .and_then(|d| d.attr("val"))
            .unwrap_or("0");
        data.get(id).cloned().unwrap_or_default()
    };
    let gap = ax("0")
        .and_then(|a| a.child("cx:catScaling"))
        .and_then(|c| c.attr_f64("gapWidth"))
        .filter(|g| g.is_finite() && *g >= 0.0)
        .map(|g| g * 100.0);
    let mut plot_kids: Vec<Rc<XNode>> = Vec::new();
    let std_axes =
        |plot: &mut Vec<Rc<XNode>>, horizontal_cat: bool, cat_hidden: bool, val_hidden: bool| {
            plot.push(axis_node(
                &AxisSpec {
                    cx: ax("0"),
                    id: "0",
                    cross: "1",
                    cat: true,
                    pos: if horizontal_cat { "b" } else { "l" },
                    hidden: cat_hidden,
                    reversed: !horizontal_cat,
                    crosses_max: false,
                    pct: false,
                },
                style,
            ));
            plot.push(axis_node(
                &AxisSpec {
                    cx: ax("1"),
                    id: "1",
                    cross: "0",
                    cat: false,
                    pos: if horizontal_cat { "l" } else { "b" },
                    hidden: val_hidden,
                    reversed: false,
                    crosses_max: false,
                    pct: false,
                },
                style,
            ));
        };
    let axis_ids = || vec![v_node("c:axId", "0"), v_node("c:axId", "1")];
    match layout.as_str() {
        "waterfall" | "funnel" | "boxWhisker" | "treemap" | "sunburst" => {
            let mut g = vec![v_node("c:varyColors", "0")];
            if let Some(gw) = gap {
                g.push(v_node("c:gapWidth", gw));
            }
            for (k, s) in visible.iter().enumerate() {
                let d = series_data(s);
                let fmt = d.fmt.clone().unwrap_or_else(|| "General".into());
                let mut kids = ser_common(s, k, style);
                let leaf = d.cats.first().cloned().unwrap_or_default();
                kids.extend(cat_val(&leaf, &d.vals, &fmt));
                let lp = s.child("cx:layoutPr");
                match layout.as_str() {
                    "waterfall" => {
                        let subs = lp
                            .and_then(|l| l.child("cx:subtotals"))
                            .map(|t| {
                                t.children_named("cx:idx")
                                    .filter_map(|i| i.attr("val"))
                                    .map(|v| v_node("c:idx", v))
                                    .collect()
                            })
                            .unwrap_or_default();
                        kids.push(node("x:subtotals", vec![], subs));
                        let conn = lp
                            .and_then(|l| l.child("cx:visibility"))
                            .and_then(|v| v.attr_bool("connectorLines"))
                            .unwrap_or(true);
                        kids.push(v_node("x:connectors", if conn { "1" } else { "0" }));
                    }
                    "boxWhisker" => {
                        let vis = lp.and_then(|l| l.child("cx:visibility"));
                        let b = |k: &str, def: bool| {
                            if vis.and_then(|v| v.attr_bool(k)).unwrap_or(def) {
                                "1".to_string()
                            } else {
                                "0".to_string()
                            }
                        };
                        let method = lp
                            .and_then(|l| l.child("cx:statistics"))
                            .and_then(|s| s.attr("quartileMethod"))
                            .unwrap_or("exclusive")
                            .to_string();
                        kids.push(node(
                            "x:box",
                            vec![
                                ("meanLine", b("meanLine", false)),
                                ("meanMarker", b("meanMarker", true)),
                                ("nonoutliers", b("nonoutliers", false)),
                                ("outliers", b("outliers", true)),
                                ("quartileMethod", method),
                            ],
                            vec![],
                        ));
                    }
                    "treemap" | "sunburst" => {
                        let lvls = d
                            .cats
                            .iter()
                            .map(|l| node("x:lvl", vec![], vec![str_ref(l)]))
                            .collect();
                        kids.push(node("x:lvls", vec![], lvls));
                        let parent = lp
                            .and_then(|l| l.child("cx:parentLabelLayout"))
                            .and_then(|p| p.attr("val"))
                            .unwrap_or("overlapping");
                        kids.push(v_node("x:parentLabelLayout", parent));
                    }
                    _ => {}
                }
                g.push(node("c:ser", vec![], kids));
            }
            g.extend(axis_ids());
            let name = match layout.as_str() {
                "waterfall" => "c:cxWaterfallChart",
                "funnel" => "c:cxFunnelChart",
                "boxWhisker" => "c:cxBoxWhiskerChart",
                "treemap" => "c:cxTreemapChart",
                _ => "c:cxSunburstChart",
            };
            plot_kids.push(node(name, vec![], g));
            match layout.as_str() {
                "waterfall" | "boxWhisker" => std_axes(&mut plot_kids, true, false, false),
                "funnel" => std_axes(&mut plot_kids, false, false, true),
                _ => {}
            }
        }
        "clusteredColumn" | "paretoLine" => {
            let Some(bars) = visible
                .iter()
                .find(|s| s.attr("layoutId") == Some("clusteredColumn"))
            else {
                return Err(layout);
            };
            let d = series_data(bars);
            let fmt = d.fmt.clone().unwrap_or_else(|| "General".into());
            let binning = bars.path(&["cx:layoutPr", "cx:binning"]);
            let leaf = d.cats.first().cloned().unwrap_or_default();
            let (cats, counts, count_fmt) = if binning.is_some() || leaf.is_empty() {
                let (c, n) = histogram(&d.vals, binning.map(|b| b.as_ref()), &fmt);
                (c, n, "General".to_string())
            } else {
                let (c, n) = aggregate(
                    &leaf,
                    &d.vals,
                    all.iter().any(|s| s.attr("layoutId") == Some("paretoLine")),
                );
                (c, n, fmt)
            };
            let vals: Vec<Option<f64>> = counts.iter().map(|v| Some(*v)).collect();
            let mut kids = ser_common(bars, 0, style);
            kids.extend(cat_val(&cats, &vals, &count_fmt));
            let mut g = vec![
                v_node("c:barDir", "col"),
                v_node("c:grouping", "clustered"),
                v_node("c:varyColors", "0"),
                node("c:ser", vec![], kids),
                v_node("c:gapWidth", gap.unwrap_or(0.0)),
                v_node("c:overlap", 0),
            ];
            g.extend(axis_ids());
            plot_kids.push(node("c:barChart", vec![], g));
            std_axes(&mut plot_kids, true, false, false);
            if let Some(line) = visible
                .iter()
                .find(|s| s.attr("layoutId") == Some("paretoLine"))
            {
                let total: f64 = counts.iter().sum();
                let mut run = 0.0;
                let cum: Vec<Option<f64>> = counts
                    .iter()
                    .map(|c| {
                        run += c;
                        (total > 0.0).then(|| run / total)
                    })
                    .collect();
                let mut kids = ser_common(line, 1, style);
                kids.push(node("c:marker", vec![], vec![v_node("c:symbol", "none")]));
                kids.extend(cat_val(&cats, &cum, "0%"));
                plot_kids.push(node(
                    "c:lineChart",
                    vec![],
                    vec![
                        v_node("c:grouping", "standard"),
                        v_node("c:varyColors", "0"),
                        node("c:ser", vec![], kids),
                        v_node("c:marker", "0"),
                        v_node("c:axId", "3"),
                        v_node("c:axId", "2"),
                    ],
                ));
                plot_kids.push(axis_node(
                    &AxisSpec {
                        cx: None,
                        id: "3",
                        cross: "2",
                        cat: true,
                        pos: "b",
                        hidden: true,
                        reversed: false,
                        crosses_max: false,
                        pct: false,
                    },
                    style,
                ));
                plot_kids.push(axis_node(
                    &AxisSpec {
                        cx: ax("2"),
                        id: "2",
                        cross: "3",
                        cat: false,
                        pos: "r",
                        hidden: false,
                        reversed: false,
                        crosses_max: true,
                        pct: true,
                    },
                    style,
                ));
            }
        }
        _ => {
            return Err(if layout.is_empty() {
                "empty".into()
            } else {
                layout
            });
        }
    }
    if let Some(sp) = region.path(&["cx:plotSurface", "cx:spPr"]) {
        plot_kids.push(renamed(sp, "c:spPr"));
    }
    let mut chart_kids = Vec::new();
    match chart.child("cx:title") {
        Some(t) => chart_kids.push(title_node(t, style, "title")),
        None => chart_kids.push(v_node("c:autoTitleDeleted", "1")),
    }
    chart_kids.push(node("c:plotArea", vec![], plot_kids));
    if let Some(l) = chart.child("cx:legend") {
        let mut lk = vec![
            v_node("c:legendPos", l.attr("pos").unwrap_or("r")),
            v_node("c:overlay", l.attr("overlay").unwrap_or("0")),
        ];
        if let Some(sp) = l.child("cx:spPr") {
            lk.push(renamed(sp, "c:spPr"));
        }
        lk.extend(merged_txpr(
            l.child("cx:txPr").map(|t| t.as_ref()),
            style_entry(style, "legend"),
            false,
        ));
        chart_kids.push(node("c:legend", vec![], lk));
    }
    chart_kids.push(v_node("c:plotVisOnly", "1"));
    chart_kids.push(v_node("c:dispBlanksAs", "gap"));
    let mut root_kids = Vec::new();
    if let Some(sp) = root.child("cx:spPr") {
        root_kids.push(renamed(sp, "c:spPr"));
    }
    root_kids.extend(merged_txpr(
        root.child("cx:txPr").map(|t| t.as_ref()),
        style_entry(style, "chartArea"),
        false,
    ));
    root_kids.push(node("c:chart", vec![], chart_kids));
    Ok(node("c:chartSpace", vec![], root_kids))
}

/// Histogram bins: labels (`[a, b]`, `(b, c]`, `≤u`, `>o`) and counts. Excel's
/// automatic bin width is Scott's rule, 3.5·σ / n^(1/3), starting at the minimum.
fn histogram(vals: &[Option<f64>], binning: Option<&XNode>, fmt: &str) -> (Vec<String>, Vec<f64>) {
    let mut xs: Vec<f64> = vals.iter().flatten().copied().collect();
    if xs.is_empty() {
        return (Vec::new(), Vec::new());
    }
    xs.sort_by(|a, b| a.total_cmp(b));
    let num_attr = |k: &str| {
        binning
            .and_then(|b| b.attr(k))
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
    };
    let under = num_attr("underflow");
    let over = num_attr("overflow");
    let right_closed = binning.and_then(|b| b.attr("intervalClosed")) != Some("l");
    let lo = under.unwrap_or(xs[0]);
    let hi = over.unwrap_or(xs[xs.len() - 1]).max(lo);
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let sd = if xs.len() > 1 {
        (xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1.0)).sqrt()
    } else {
        0.0
    };
    let child_val = |k: &str| {
        binning
            .and_then(|b| b.child(k))
            .and_then(|c| c.attr_f64("val"))
            .filter(|v| v.is_finite() && *v > 0.0)
    };
    let width = match (child_val("cx:binSize"), child_val("cx:binCount")) {
        (Some(w), _) => w,
        (None, Some(c)) => (hi - lo) / c.round().max(1.0),
        _ => 3.5 * sd / n.cbrt(),
    };
    let count = if width > 0.0 && hi > lo {
        ((hi - lo) / width).ceil().clamp(1.0, 1000.0) as usize
    } else {
        1
    };
    let width = if width > 0.0 { width } else { 1.0 };
    let f = |v: f64| format_value(v, fmt, false);
    let mut labels = Vec::new();
    let mut counts = Vec::new();
    if let Some(u) = under {
        labels.push(format!("≤{}", f(u)));
        counts.push(xs.iter().filter(|x| **x <= u).count() as f64);
    }
    for k in 0..count {
        let a = lo + k as f64 * width;
        let b = a + width;
        let first = k == 0 && under.is_none();
        let last = k + 1 == count && over.is_none();
        let inside = |x: f64| {
            if right_closed {
                (x > a || (first && x >= a)) && x <= b
            } else {
                x >= a && (x < b || (last && x <= b))
            }
        };
        let c = xs.iter().filter(|x| inside(**x)).count();
        let text = if right_closed {
            format!("{}{}, {}]", if first { "[" } else { "(" }, f(a), f(b))
        } else {
            format!("[{}, {}{}", f(a), f(b), if last { "]" } else { ")" })
        };
        labels.push(text);
        counts.push(c as f64);
    }
    if let Some(o) = over {
        labels.push(format!(">{}", f(o)));
        counts.push(xs.iter().filter(|x| **x > o).count() as f64);
    }
    (labels, counts)
}

/// Sums values per distinct category (first-appearance order), largest first
/// for Pareto charts.
fn aggregate(cats: &[String], vals: &[Option<f64>], sort: bool) -> (Vec<String>, Vec<f64>) {
    let mut keys: Vec<String> = Vec::new();
    let mut sums: Vec<f64> = Vec::new();
    for (i, c) in cats.iter().enumerate() {
        let v = vals.get(i).copied().flatten().unwrap_or(0.0);
        match keys.iter().position(|k| k == c) {
            Some(p) => sums[p] += v,
            None => {
                keys.push(c.clone());
                sums.push(v);
            }
        }
    }
    if sort {
        let mut idx: Vec<usize> = (0..keys.len()).collect();
        idx.sort_by(|a, b| sums[*b].total_cmp(&sums[*a]));
        return (
            idx.iter().map(|i| keys[*i].clone()).collect(),
            idx.iter().map(|i| sums[*i]).collect(),
        );
    }
    (keys, sums)
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

/// Excel's QUARTILE.EXC (`exclusive`) / QUARTILE.INC: rank (n+1)p or (n-1)p+1,
/// linearly interpolated and clamped to the data.
pub(super) fn quantile(sorted: &[f64], p: f64, exclusive: bool) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    let h = if exclusive {
        (n as f64 + 1.0) * p
    } else {
        (n as f64 - 1.0) * p + 1.0
    }
    .clamp(1.0, n as f64);
    let lo = h.floor() as usize;
    let frac = h - lo as f64;
    if lo >= n {
        return sorted[n - 1];
    }
    sorted[lo - 1] + frac * (sorted[lo] - sorted[lo - 1])
}

pub(super) struct BoxStats {
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub lo: f64,
    pub hi: f64,
    pub mean: f64,
    pub outliers: Vec<f64>,
    pub inner: Vec<f64>,
}

/// Box plot statistics: whiskers reach the furthest points within 1.5 IQR of
/// the box; points beyond are outliers.
pub(super) fn box_stats(vals: &[f64], exclusive: bool) -> Option<BoxStats> {
    let mut v: Vec<f64> = vals.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let q1 = quantile(&v, 0.25, exclusive);
    let q3 = quantile(&v, 0.75, exclusive);
    let median = quantile(&v, 0.5, false);
    let iqr = q3 - q1;
    let (f0, f1) = (q1 - 1.5 * iqr, q3 + 1.5 * iqr);
    let inside: Vec<f64> = v.iter().copied().filter(|x| *x >= f0 && *x <= f1).collect();
    let lo = inside.first().copied().unwrap_or(q1).min(q1);
    let hi = inside.last().copied().unwrap_or(q3).max(q3);
    Some(BoxStats {
        q1,
        median,
        q3,
        lo,
        hi,
        mean: v.iter().sum::<f64>() / v.len() as f64,
        outliers: v.iter().copied().filter(|x| *x < f0 || *x > f1).collect(),
        inner: inside,
    })
}

// ---------------------------------------------------------------------------
// Group data
// ---------------------------------------------------------------------------

fn subtotals(s: &SerInfo) -> Vec<usize> {
    s.node
        .child("x:subtotals")
        .map(|t| {
            t.children_named("c:idx")
                .filter_map(|i| i.attr_i64("val"))
                .filter(|v| *v >= 0)
                .map(|v| v as usize)
                .collect()
        })
        .unwrap_or_default()
}

impl Grp {
    /// Running totals: each point spans (total before, total after); a
    /// subtotal point is a full bar from zero to its own value.
    pub(super) fn waterfall_stack(&self, n: usize) -> Vec<Vec<Option<(f64, f64)>>> {
        self.sers
            .iter()
            .map(|s| {
                let subs = subtotals(s);
                let mut run = 0.0;
                (0..n)
                    .map(|i| {
                        let v = s.vals.get(i).copied().flatten()?;
                        if subs.contains(&i) {
                            run = v;
                            Some((0.0, v))
                        } else {
                            let b = run;
                            run += v;
                            Some((b, run))
                        }
                    })
                    .collect()
            })
            .collect()
    }

    /// Distinct categories of a box & whisker group, in first-appearance order.
    pub(super) fn box_cats(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for s in &self.sers {
            for c in s.s.categories.iter().take(MAX_POINTS) {
                if !out.contains(c) {
                    out.push(c.clone());
                }
            }
        }
        if out.is_empty() {
            out.push(String::new());
        }
        out
    }

    /// The values of series `s` that fall in each category of `cats`.
    pub(super) fn box_values(s: &SerInfo, cats: &[String]) -> Vec<Vec<f64>> {
        let mut out = vec![Vec::new(); cats.len()];
        for (i, v) in s.vals.iter().enumerate() {
            let Some(v) = v else { continue };
            let k = match s.s.categories.get(i) {
                Some(c) => cats.iter().position(|x| x == c).unwrap_or(0),
                None => 0,
            };
            if let Some(b) = out.get_mut(k) {
                b.push(*v);
            }
        }
        out
    }

    pub(super) fn box_stack(&self, n: usize) -> Vec<Vec<Option<(f64, f64)>>> {
        let cats = self.box_cats();
        self.sers
            .iter()
            .map(|s| {
                let vals = Self::box_values(s, &cats);
                (0..n)
                    .map(|k| {
                        let v = vals.get(k)?;
                        let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
                        let hi = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                        (lo <= hi).then_some((lo, hi))
                    })
                    .collect()
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Hierarchies (treemap, sunburst)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(super) struct TreeNode {
    pub label: String,
    pub value: f64,
    pub children: Vec<usize>,
    /// Pre-order position, the index `dataPt`/`dataLabel` overrides use.
    pub pre: usize,
    pub depth: usize,
    /// Index of the top-level ancestor among the top-level nodes.
    pub top: usize,
    /// Data point of a leaf.
    pub point: Option<usize>,
}

/// Builds the category tree. `levels` are stored leaf level first; a blank
/// parent cell skips that level (ragged hierarchies). Node 0 is a virtual root.
pub(super) fn build_tree(levels: &[Vec<String>], vals: &[Option<f64>]) -> Vec<TreeNode> {
    let mut nodes = vec![TreeNode {
        label: String::new(),
        value: 0.0,
        children: Vec::new(),
        pre: 0,
        depth: 0,
        top: 0,
        point: None,
    }];
    for (i, v) in vals.iter().enumerate().take(MAX_POINTS) {
        let Some(v) = v.filter(|v| *v > 0.0) else {
            continue;
        };
        let mut cur = 0;
        let path: Vec<String> = levels
            .iter()
            .rev()
            .map(|l| l.get(i).cloned().unwrap_or_default())
            .collect();
        let last = path.len().saturating_sub(1);
        for (d, label) in path.iter().enumerate() {
            if label.is_empty() && d != last {
                continue;
            }
            let leaf = d == last;
            let found = if leaf {
                None
            } else {
                nodes[cur]
                    .children
                    .iter()
                    .copied()
                    .find(|c| nodes[*c].label == *label && nodes[*c].point.is_none())
            };
            cur = match found {
                Some(c) => c,
                None => {
                    nodes.push(TreeNode {
                        label: label.clone(),
                        value: 0.0,
                        children: Vec::new(),
                        pre: 0,
                        depth: 0,
                        top: 0,
                        point: leaf.then_some(i),
                    });
                    let id = nodes.len() - 1;
                    nodes[cur].children.push(id);
                    id
                }
            };
            nodes[cur].value += v;
        }
        nodes[0].value += v;
    }
    // Pre-order numbering, depths and top-level ancestors.
    let tops = nodes[0].children.clone();
    let mut counter = 0;
    let mut stack: Vec<(usize, usize, usize)> = tops
        .iter()
        .enumerate()
        .rev()
        .map(|(t, id)| (*id, 0, t))
        .collect();
    while let Some((id, depth, top)) = stack.pop() {
        nodes[id].pre = counter;
        nodes[id].depth = depth;
        nodes[id].top = top;
        counter += 1;
        for c in nodes[id].children.clone().into_iter().rev() {
            stack.push((c, depth + 1, top));
        }
    }
    nodes
}

/// Squarified treemap layout (Bruls, Huizing, van Wijk): items are placed in
/// rows along the shorter side, keeping each row's worst aspect ratio low.
pub(super) fn squarify(values: &[f64], r: Rect) -> Vec<Rect> {
    let mut out = vec![
        Rect {
            x: r.x,
            y: r.y,
            w: 0.0,
            h: 0.0
        };
        values.len()
    ];
    let total: f64 = values.iter().sum();
    if total <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
        return out;
    }
    let scale = r.w * r.h / total;
    let mut rest = r;
    let mut i = 0;
    while i < values.len() {
        let side = rest.w.min(rest.h);
        let worst = |row: &[f64]| {
            let s: f64 = row.iter().sum();
            let (mx, mn) = row
                .iter()
                .fold((0.0f64, f64::MAX), |(a, b), v| (a.max(*v), b.min(*v)));
            let s2 = s * s;
            let w2 = side * side;
            (w2 * mx / s2).max(s2 / (w2 * mn))
        };
        let mut j = i + 1;
        let areas: Vec<f64> = values.iter().map(|v| v * scale).collect();
        while j < values.len() && worst(&areas[i..=j]) <= worst(&areas[i..j]) {
            j += 1;
        }
        let row = &areas[i..j];
        let sum: f64 = row.iter().sum();
        let thick = if side > 0.0 { sum / side } else { 0.0 };
        let mut off = 0.0;
        for (k, a) in row.iter().enumerate() {
            let len = if thick > 0.0 { a / thick } else { 0.0 };
            out[i + k] = if rest.w >= rest.h {
                Rect {
                    x: rest.x,
                    y: rest.y + off,
                    w: thick,
                    h: len,
                }
            } else {
                Rect {
                    x: rest.x + off,
                    y: rest.y,
                    w: len,
                    h: thick,
                }
            };
            off += len;
        }
        if rest.w >= rest.h {
            rest.x += thick;
            rest.w = (rest.w - thick).max(0.0);
        } else {
            rest.y += thick;
            rest.h = (rest.h - thick).max(0.0);
        }
        i = j;
    }
    out
}

/// Relative luminance used to pick black or white label text on a tile.
fn luminance(c: Rgba) -> f32 {
    0.299 * c.r + 0.587 * c.g + 0.114 * c.b
}

fn color_layer(c: Rgba) -> Rc<XNode> {
    let hex = c.to_argb_hex();
    let fill = node(
        "a:solidFill",
        vec![],
        vec![node(
            "a:srgbClr",
            vec![("val", hex[2..].to_string())],
            vec![],
        )],
    );
    node(
        "a:lstStyle",
        vec![],
        vec![node(
            "a:lvl1pPr",
            vec![],
            vec![node("a:defRPr", vec![], vec![fill])],
        )],
    )
}

fn has_explicit_color(txpr: Option<&Rc<XNode>>) -> bool {
    txpr.and_then(|t| t.path(&["a:p", "a:pPr", "a:defRPr"]))
        .is_some_and(|d| d.child("a:solidFill").is_some())
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

impl Renderer<'_, '_> {
    /// Charts drawn without the classic axis frame: radar, surface, treemap
    /// and sunburst groups, laid out in the plot rectangle.
    pub(super) fn draw_free(
        &mut self,
        groups: &[Grp],
        plot: Option<&XNode>,
        rect: Rect,
        manual: bool,
        out: &mut Vec<DLabel>,
    ) {
        let radar: Vec<&Grp> = groups
            .iter()
            .filter(|g| matches!(g.kind, Kind::Radar { .. }))
            .collect();
        if !radar.is_empty() {
            self.draw_radar(&radar, plot, rect, manual, out);
        }
        for g in groups {
            match g.kind {
                Kind::Surface { three_d } => self.draw_surface(g, plot, rect, three_d),
                Kind::Treemap => self.draw_treemap(g, rect),
                Kind::Sunburst => self.draw_sunburst(g, rect),
                _ => {}
            }
        }
    }

    /// Legend entries for layouts whose legend is not one key per series.
    pub(super) fn special_legend(&mut self, groups: &[Grp]) -> Option<Vec<(String, Key)>> {
        let g = groups.first()?;
        let unit = Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        };
        match g.kind {
            Kind::Waterfall => {
                let s = g.sers.first()?;
                let names = ["Increase", "Decrease", "Total"];
                Some(
                    names
                        .iter()
                        .enumerate()
                        .map(|(k, n)| {
                            let fill = self.waterfall_fill(s, k, usize::MAX, unit);
                            ((*n).to_string(), Key::Box(fill, None))
                        })
                        .collect(),
                )
            }
            Kind::Treemap | Kind::Sunburst => {
                let s = g.sers.first()?;
                let tree = build_tree(&tree_levels(s), &s.vals);
                let tops = tree[0].children.clone();
                Some(
                    tops.iter()
                        .map(|id| {
                            let fill = self.tree_fill(s, &tree, *id, tops.len(), unit);
                            (tree[*id].label.clone(), Key::Box(fill, None))
                        })
                        .collect(),
                )
            }
            Kind::Surface { .. } => Some(self.surface_legend(g)),
            _ => None,
        }
    }

    /// Fill of waterfall point kind `k` (0 increase, 1 decrease, 2 total).
    fn waterfall_fill(&self, s: &SerInfo, k: usize, i: usize, r: Rect) -> Option<Paint> {
        let dpt =
            s.s.point_sp_pr
                .iter()
                .find(|(p, _)| *p == i)
                .map(|(_, n)| n.clone());
        let auto = Some(Paint::Solid(self.auto_color(k, 3)));
        if Self::has_fill(dpt.as_deref()) {
            return self.fill_or(dpt.as_deref(), r, auto);
        }
        self.fill_or(s.s.sp_pr.as_deref(), r, auto)
    }

    pub(super) fn draw_waterfall(
        &mut self,
        g: &Grp,
        axes: &[Axis],
        p: &Rect,
        out: &mut Vec<DLabel>,
    ) {
        let (cat, v) = (&axes[g.cat], &axes[g.val]);
        let n = g.n_points().min(cat.n.max(1));
        if n == 0 || g.sers.is_empty() {
            return;
        }
        let gap = val_f(&g.node, "c:gapWidth").unwrap_or(50.0);
        let k = g.sers.len();
        let stack = g.stack();
        let slot_t = 1.0 / cat.n.max(1) as f64;
        let conn_color = mix(self.tx1(), self.bg1(), 0.45);
        for (j, s) in g.sers.iter().enumerate() {
            let subs = subtotals(s);
            let connectors = val(&s.node, "x:connectors") != Some("0");
            let (off, bw) = cluster_geometry(k, gap, 0.0, j);
            let mut prev: Option<(f64, f64)> = None;
            for i in 0..n {
                let Some((b0, b1)) = stack[j].get(i).copied().flatten() else {
                    prev = None;
                    continue;
                };
                let t0 = i as f64 * slot_t + off * slot_t;
                let t1 = t0 + bw * slot_t;
                let (c0, c1) = (cat.px(t0, p), cat.px(t1, p));
                let (p0, p1) = (
                    v.px(v.frac(b0).clamp(0.0, 1.0), p),
                    v.px(v.frac(b1).clamp(0.0, 1.0), p),
                );
                if let (true, Some((x_end, level))) = (connectors, prev) {
                    let y = v.px(v.frac(level).clamp(0.0, 1.0), p);
                    let stroke = Some(Stroke {
                        paint: Paint::Solid(conn_color),
                        width: 0.75,
                        dash: Vec::new(),
                        cap: LineCap::Flat,
                        join: LineJoin::Miter,
                        head: None,
                        tail: None,
                    });
                    self.push_line(&[(x_end, y), (c0.min(c1), y)], stroke);
                }
                let r = rect_xyxy(c0, p0, c1, p1);
                let kind = if subs.contains(&i) {
                    2
                } else {
                    usize::from(b1 < b0)
                };
                let fill = self.waterfall_fill(s, kind, i, r);
                let stroke = self.point_stroke(s, i, None);
                self.push_rect(
                    Rect {
                        h: r.h.max(0.01),
                        ..r
                    },
                    fill,
                    stroke,
                );
                prev = Some((c0.max(c1), b1));
                let up = if b1 >= b0 { -1.0 } else { 1.0 };
                let dir = (0.0, if v.reversed { -up } else { up });
                self.collect_dlabel(
                    g,
                    s,
                    i,
                    n,
                    b1 - b0,
                    |pos| Anchor::Bar {
                        r,
                        dir,
                        pos: pos.unwrap_or("outEnd").to_string(),
                    },
                    out,
                );
            }
        }
    }

    pub(super) fn draw_funnel(&mut self, g: &Grp, axes: &[Axis], p: &Rect, out: &mut Vec<DLabel>) {
        let cat = &axes[g.cat];
        let n = g.n_points().min(cat.n.max(1));
        let Some(s) = g.sers.first() else { return };
        let max = s.vals.iter().flatten().fold(0.0f64, |m, v| m.max(v.abs()));
        if n == 0 || max <= 0.0 {
            return;
        }
        let gap = val_f(&g.node, "c:gapWidth").unwrap_or(6.0) / 100.0;
        let slot = p.h / n as f64;
        let bar_h = slot / (1.0 + gap);
        for i in 0..n {
            let Some(val_i) = s.vals.get(i).copied().flatten() else {
                continue;
            };
            let cy = cat.px(cat.cat_frac(i as f64), p);
            let w = val_i.abs() / max * p.w;
            let r = Rect {
                x: p.x + (p.w - w) / 2.0,
                y: cy - bar_h / 2.0,
                w,
                h: bar_h,
            };
            let fill = self.point_fill(g, s, i, n, r);
            let stroke = self.point_stroke(s, i, None);
            self.push_rect(r, fill, stroke);
            self.collect_dlabel(
                g,
                s,
                i,
                n,
                val_i,
                |pos| Anchor::Bar {
                    r,
                    dir: (1.0, 0.0),
                    pos: pos.unwrap_or("ctr").to_string(),
                },
                out,
            );
        }
    }

    pub(super) fn draw_box_whisker(&mut self, g: &Grp, axes: &[Axis], p: &Rect) {
        let (cat, v) = (&axes[g.cat], &axes[g.val]);
        let cats = g.box_cats();
        let n = cats.len().min(cat.n.max(1));
        let k = g.sers.len();
        if n == 0 || k == 0 {
            return;
        }
        let gap = val_f(&g.node, "c:gapWidth").unwrap_or(100.0);
        let slot_t = 1.0 / cat.n.max(1) as f64;
        let y = |x: f64| v.px(v.frac(x).clamp(0.0, 1.0), p);
        let bg = self.bg1();
        let line = |c: Rgba, w: f64| {
            Some(Stroke {
                paint: Paint::Solid(c),
                width: w,
                dash: Vec::new(),
                cap: LineCap::Flat,
                join: LineJoin::Miter,
                head: None,
                tail: None,
            })
        };
        for (j, s) in g.sers.iter().enumerate() {
            let opts = s.node.child("x:box").cloned();
            let opt =
                |k: &str, def: bool| opts.as_ref().and_then(|o| o.attr_bool(k)).unwrap_or(def);
            let exclusive =
                opts.as_ref().and_then(|o| o.attr("quartileMethod")) != Some("inclusive");
            let color = self.series_color(s, false);
            let fill = self.point_fill(g, s, usize::MAX, 1, *p);
            let outline = self.stroke_or(s.s.sp_pr.as_deref(), None);
            let (off, bw) = cluster_geometry(k, gap, 0.0, j);
            let values = Grp::box_values(s, &cats);
            let mut means = Vec::new();
            for (c, vals) in values.iter().enumerate().take(n) {
                let Some(st) = box_stats(vals, exclusive) else {
                    continue;
                };
                let t0 = c as f64 * slot_t + off * slot_t;
                let t1 = t0 + bw * slot_t;
                let (x0, x1) = (cat.px(t0, p), cat.px(t1, p));
                let xm = (x0 + x1) / 2.0;
                let cap = (x1 - x0).abs() * 0.25;
                // Whiskers and caps, then the box over them.
                let wl = line(color, 0.75);
                self.push_line(&[(xm, y(st.hi)), (xm, y(st.q3))], wl.clone());
                self.push_line(&[(xm, y(st.q1)), (xm, y(st.lo))], wl.clone());
                self.push_line(&[(xm - cap, y(st.hi)), (xm + cap, y(st.hi))], wl.clone());
                self.push_line(&[(xm - cap, y(st.lo)), (xm + cap, y(st.lo))], wl);
                let r = rect_xyxy(x0, y(st.q1), x1, y(st.q3));
                self.push_rect(r, fill.clone(), outline.clone());
                self.push_line(
                    &[(x0.min(x1), y(st.median)), (x0.max(x1), y(st.median))],
                    line(bg, 1.0),
                );
                let mark = |c: Rgba| MarkerSpec {
                    symbol: "circle".into(),
                    size: 5.0,
                    fill: Some(Paint::Solid(bg)),
                    stroke: line(c, 0.75),
                };
                if opt("outliers", true) {
                    for o in &st.outliers {
                        self.draw_marker(&mark(color), xm, y(*o));
                    }
                }
                if opt("nonoutliers", false) {
                    for o in &st.inner {
                        self.draw_marker(&mark(color), xm, y(*o));
                    }
                }
                if opt("meanMarker", true) {
                    let inside = st.mean >= st.q1 && st.mean <= st.q3;
                    let m = MarkerSpec {
                        symbol: "x".into(),
                        size: 6.0,
                        fill: None,
                        stroke: line(if inside { bg } else { color }, 1.0),
                    };
                    self.draw_marker(&m, xm, y(st.mean));
                }
                means.push((xm, y(st.mean)));
            }
            if opt("meanLine", false) && means.len() > 1 {
                self.push_line(&means, line(color, 1.0));
            }
        }
    }

    /// Fill of tree node `id`: its own `dataPt`, else an ancestor's, else the
    /// palette color of its top-level branch.
    fn tree_fill(
        &self,
        s: &SerInfo,
        tree: &[TreeNode],
        id: usize,
        n_top: usize,
        r: Rect,
    ) -> Option<Paint> {
        let auto = Some(Paint::Solid(self.auto_color(tree[id].top, n_top.max(1))));
        let mut cur = Some(id);
        while let Some(c) = cur {
            let pre = tree[c].pre;
            let dpt =
                s.s.point_sp_pr
                    .iter()
                    .find(|(p, _)| *p == pre)
                    .map(|(_, n)| n.clone());
            if Self::has_fill(dpt.as_deref()) {
                return self.fill_or(dpt.as_deref(), r, auto);
            }
            cur = tree
                .iter()
                .position(|t| t.children.contains(&c))
                .filter(|p| *p != 0);
        }
        self.fill_or(s.s.sp_pr.as_deref(), r, auto)
    }

    fn tree_stroke(&self, s: &SerInfo, pre: usize) -> Option<Stroke> {
        let dpt =
            s.s.point_sp_pr
                .iter()
                .find(|(p, _)| *p == pre)
                .map(|(_, n)| n.clone());
        let sp = if dpt.as_ref().and_then(|d| d.child("a:ln")).is_some() {
            dpt
        } else {
            s.s.sp_pr.clone()
        };
        self.stroke_or(sp.as_deref(), Some((self.bg1(), 1.0)))
    }

    /// The label of tree node `id` per the series' data label settings, with
    /// black or white text by tile brightness unless a color is set.
    fn tree_label(
        &mut self,
        s: &SerInfo,
        node: &TreeNode,
        tile: Option<Rgba>,
        wrap: Option<f64>,
    ) -> Option<Label> {
        let dl = s.node.child("c:dLbls")?.clone();
        let own = dl
            .children_named("c:dLbl")
            .find(|l| val_f(l, "c:idx").map(|v| v as usize) == Some(node.pre))
            .cloned();
        if own
            .as_ref()
            .is_some_and(|o| flag(o, "c:delete") == Some(true))
        {
            return None;
        }
        let chain: Vec<Rc<XNode>> = own.iter().cloned().chain(std::iter::once(dl)).collect();
        let txpr = dl_get(&chain, "c:txPr").cloned();
        let mut lists = self.lists(TextKind::Body, &[txpr.as_ref()]);
        if let (Some(c), false) = (tile, has_explicit_color(txpr.as_ref())) {
            lists.push(color_layer(if luminance(c) > 0.55 {
                Rgba::BLACK
            } else {
                Rgba::WHITE
            }));
        }
        let rich = own
            .as_ref()
            .and_then(|o| o.path(&["c:tx", "c:rich"]).cloned());
        let lab = if let Some(r) = rich {
            self.label_tx(r, lists.clone(), 0.0, None)
        } else {
            let mut parts = Vec::new();
            if dl_flag(&chain, "c:showCatName") {
                parts.push(node.label.clone());
            }
            if dl_flag(&chain, "c:showVal") {
                let fmt = match dl_get(&chain, "c:numFmt") {
                    Some(f) if f.attr_bool("sourceLinked") != Some(true) => {
                        f.attr("formatCode").unwrap_or("General").to_string()
                    }
                    _ => s.fmt.clone(),
                };
                parts.push(format_value(node.value, &fmt, self.date1904));
            }
            let sep = dl_get(&chain, "c:separator")
                .map(|n| n.text.clone())
                .unwrap_or_else(|| ", ".into());
            let text = parts.join(&sep);
            self.label(&text, &lists, 0.0, None)
        }?;
        match wrap {
            Some(w) if lab.w > w => Some(self.relabel(&lab, Some(w.max(lab.size)), 0.0)),
            _ => Some(lab),
        }
    }

    fn draw_treemap(&mut self, g: &Grp, rect: Rect) {
        let Some(s) = g.sers.first() else { return };
        let tree = build_tree(&tree_levels(s), &s.vals);
        let n_top = tree[0].children.len();
        let parent_layout = val(&s.node, "x:parentLabelLayout")
            .unwrap_or("overlapping")
            .to_string();
        let pad = 3.0;
        // Breadth-first layout: each node's children share its rectangle.
        let mut rects: Vec<Option<Rect>> = vec![None; tree.len()];
        rects[0] = Some(rect);
        let mut queue = vec![0usize];
        let mut banners: Vec<(usize, Rect)> = Vec::new();
        while let Some(id) = queue.pop() {
            let Some(mut r) = rects[id] else { continue };
            if tree[id].children.is_empty() {
                continue;
            }
            if id != 0
                && tree[id].depth == 0
                && parent_layout == "banner"
                && let Some(lab) = self.tree_label(s, &tree[id], None, Some(r.w - 2.0 * pad))
            {
                let bh = (lab.h + 2.0 * pad).min(r.h * 0.5);
                banners.push((id, Rect { h: bh, ..r }));
                r.y += bh;
                r.h -= bh;
            }
            let mut kids = tree[id].children.clone();
            kids.sort_by(|a, b| tree[*b].value.total_cmp(&tree[*a].value));
            let vals: Vec<f64> = kids.iter().map(|k| tree[*k].value).collect();
            for (k, kr) in kids.iter().zip(squarify(&vals, r)) {
                rects[*k] = Some(kr);
                queue.push(*k);
            }
        }
        let mut labels: Vec<(Label, f64, f64)> = Vec::new();
        for (id, t) in tree.iter().enumerate().skip(1) {
            let Some(r) = rects[id] else { continue };
            if !t.children.is_empty() {
                continue;
            }
            let fill = self.tree_fill(s, &tree, id, n_top, r);
            let stroke = self.tree_stroke(s, t.pre);
            self.push_rect(r, fill.clone(), stroke);
            let avail = r.w - 2.0 * pad;
            if avail <= 4.0 {
                continue;
            }
            let tile = fill.as_ref().and_then(|f| f.representative());
            let Some(lab) = self.tree_label(s, t, tile, Some(avail)) else {
                continue;
            };
            if lab.h + pad > r.h {
                continue;
            }
            let cx = r.x + pad + lab.box_w.min(avail) / 2.0;
            let cy = if parent_layout == "overlapping" {
                r.y + r.h - pad - lab.h / 2.0
            } else {
                r.y + pad + lab.h / 2.0
            };
            labels.push((lab, cx, cy));
        }
        for (id, br) in banners {
            let fill = self.tree_fill(s, &tree, id, n_top, br);
            self.push_rect(br, fill.clone(), self.tree_stroke(s, tree[id].pre));
            let tile = fill.as_ref().and_then(|f| f.representative());
            if let Some(lab) = self.tree_label(s, &tree[id], tile, Some(br.w - 2.0 * pad)) {
                labels.push((lab.clone(), br.x + pad + lab.box_w / 2.0, br.y + br.h / 2.0));
            }
        }
        if parent_layout == "overlapping" {
            for &id in &tree[0].children {
                let (Some(r), false) = (rects[id], tree[id].children.is_empty()) else {
                    continue;
                };
                if let Some(lab) = self.tree_label(s, &tree[id], None, Some(r.w - 2.0 * pad)) {
                    labels.push((
                        lab.clone(),
                        r.x + pad + lab.box_w / 2.0,
                        r.y + pad + lab.h / 2.0,
                    ));
                }
            }
        }
        for (lab, cx, cy) in &labels {
            self.draw_label(lab, *cx, *cy);
        }
    }

    fn draw_sunburst(&mut self, g: &Grp, rect: Rect) {
        let Some(s) = g.sers.first() else { return };
        let tree = build_tree(&tree_levels(s), &s.vals);
        let total = tree[0].value;
        let depth = tree.iter().skip(1).map(|t| t.depth + 1).max().unwrap_or(0);
        if total <= 0.0 || depth == 0 {
            return;
        }
        let n_top = tree[0].children.len();
        let radius = rect.w.min(rect.h) / 2.0;
        let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        // A small hole in the middle, then one ring per level.
        let ring = radius / (depth as f64 + 0.35);
        let hole = radius - ring * depth as f64;
        let mut labels = Vec::new();
        let mut stack: Vec<(usize, f64, f64)> = vec![(0, 0.0, 360.0)];
        while let Some((id, a0, sweep)) = stack.pop() {
            let t = &tree[id];
            if id != 0 {
                let (r0, r1) = (
                    hole + ring * t.depth as f64,
                    hole + ring * (t.depth + 1) as f64,
                );
                let bounds = Rect {
                    x: cx - r1,
                    y: cy - r1,
                    w: 2.0 * r1,
                    h: 2.0 * r1,
                };
                let fill = self.tree_fill(s, &tree, id, n_top, bounds);
                let stroke = self.tree_stroke(s, t.pre);
                if let Some(path) = slice_path(cx, cy, r0, r1, 1.0, a0, sweep) {
                    self.push(path, fill.clone(), stroke, true);
                }
                let mid = (a0 + sweep / 2.0).to_radians();
                let rm = (r0 + r1) / 2.0;
                let arc = rm * sweep.to_radians().min(std::f64::consts::PI);
                let tile = fill.as_ref().and_then(|f| f.representative());
                if let Some(lab) = self.tree_label(s, t, tile, Some((r1 - r0) * 0.95)) {
                    let (ew, eh) = lab.ext();
                    if eh <= arc * 0.9 && ew <= (r1 - r0) * 1.05 {
                        labels.push((lab, cx + rm * mid.sin(), cy - rm * mid.cos()));
                    }
                }
            }
            let mut a = a0;
            for c in &t.children {
                let sw = if t.value > 0.0 {
                    sweep * tree[*c].value / t.value
                } else {
                    0.0
                };
                stack.push((*c, a, sw));
                a += sw;
            }
        }
        for (lab, x, y) in &labels {
            self.draw_label(lab, *x, *y);
        }
    }
}

/// Category levels of a treemap/sunburst series (`x:lvls`), leaf level first.
fn tree_levels(s: &SerInfo) -> Vec<Vec<String>> {
    let Some(l) = s.node.child("x:lvls") else {
        return vec![s.s.categories.clone()];
    };
    l.children_named("x:lvl")
        .map(|lv| {
            lv.path(&["c:strRef", "c:strCache"])
                .map(|c| {
                    let n = val_f(c, "c:ptCount").unwrap_or(0.0).clamp(0.0, 100_000.0) as usize;
                    let mut out = vec![String::new(); n];
                    for p in c.children_named("c:pt") {
                        let i = p.attr_i64("idx").unwrap_or(0).max(0) as usize;
                        if i < n {
                            out[i] = p.child("c:v").map(|v| v.text.clone()).unwrap_or_default();
                        }
                    }
                    out
                })
                .unwrap_or_default()
        })
        .collect()
}
