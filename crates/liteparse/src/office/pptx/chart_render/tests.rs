use super::*;

#[test]
fn general_format() {
    assert_eq!(general(0.0), "0");
    assert_eq!(general(42.0), "42");
    assert_eq!(general(1448.0), "1448");
    assert_eq!(general(28288.0), "28288");
    assert_eq!(general(-3.5), "-3.5");
    assert_eq!(general(0.1 + 0.2), "0.3");
    assert_eq!(general(1.0 / 3.0), "0.333333333");
    assert_eq!(general(12_345_678_901.0), "12345678901");
    assert_eq!(general(123_456_789_012.0), "1.23457E+11");
    assert_eq!(general(0.000_012_345_678), "1.23457E-05");
}

#[test]
fn numeric_formats() {
    assert_eq!(format_value(1234.567, "#,##0", false), "1,235");
    assert_eq!(format_value(1234.567, "#,##0.00", false), "1,234.57");
    assert_eq!(format_value(0.256, "0%", false), "26%");
    assert_eq!(format_value(0.256, "0.0%", false), "25.6%");
    assert_eq!(format_value(2.5, "0", false), "3");
    assert_eq!(format_value(-1500.0, "#,##0", false), "-1,500");
    assert_eq!(format_value(-1500.0, "#,##0;(#,##0)", false), "(1,500)");
    assert_eq!(format_value(0.0, "#,##0;-#,##0;\"-\"", false), "-");
    assert_eq!(format_value(55.0, "0\"%\"", false), "55%");
    assert_eq!(format_value(1234.5, "\"$\"#,##0.00", false), "$1,234.50");
    assert_eq!(format_value(1234.5, "[$€-407]#,##0", false), "€1,235");
    assert_eq!(format_value(1_500_000.0, "#,##0,,\"M\"", false), "2M");
    assert_eq!(format_value(12345.0, "0.00E+00", false), "1.23E+04");
    assert_eq!(format_value(5.0, "0.##", false), "5.");
    assert_eq!(format_value(5.25, "0.##", false), "5.25");
    assert_eq!(format_value(7.0, "General", false), "7");
    assert_eq!(format_value(0.5, "[Red]0.0", false), "0.5");
    assert_eq!(format_value(3.0, "0_);(0)", false), "3 ");
}

#[test]
fn date_formats() {
    // 2020-01-15 is serial 43845.
    assert_eq!(format_value(43845.0, "mmm-yy", false), "Jan-20");
    assert_eq!(format_value(43845.0, "m/d/yyyy", false), "1/15/2020");
    assert_eq!(format_value(43845.0, "dd/mm/yyyy", false), "15/01/2020");
    assert_eq!(format_value(43845.0, "mmmm yyyy", false), "January 2020");
    assert_eq!(format_value(43845.5, "h:mm AM/PM", false), "12:00 PM");
    assert_eq!(format_value(43845.0, "ddd", false), "Wed");
    assert_eq!(format_value(1.0, "yyyy-mm-dd", false), "1900-01-01");
}

#[test]
fn nice_units() {
    assert_eq!(nice_unit(52.5, 10.0), 10.0);
    assert_eq!(nice_unit(105.0, 7.0), 20.0);
    assert_eq!(nice_unit(1.0, 5.0), 0.2);
    assert_eq!(nice_unit(0.0, 5.0), 0.2);
}

#[test]
fn excel_scaling() {
    // Data up to 50: 5% headroom pushes the max to the next unit.
    let s = auto_scale(0.0, 50.0, None, None, None, 6.0);
    assert_eq!(
        s,
        AxisScale {
            min: 0.0,
            max: 60.0,
            major: 10.0
        }
    );
    // Max just under a unit stays within it.
    let s = auto_scale(3.0, 47.0, None, None, None, 6.0);
    assert_eq!((s.min, s.max, s.major), (0.0, 50.0, 10.0));
    // All values within the top sixth: the axis does not start at zero.
    let s = auto_scale(95.0, 100.0, None, None, None, 6.0);
    assert!(s.min > 0.0 && s.min <= 95.0, "{s:?}");
    assert!(s.max >= 100.0);
    // Negative data.
    let s = auto_scale(-20.0, 30.0, None, None, None, 6.0);
    assert!(s.min <= -20.0 && s.max >= 30.0 && s.min < 0.0);
    // Fixed bounds and unit win.
    let s = auto_scale(0.0, 47.0, Some(0.0), Some(100.0), Some(25.0), 6.0);
    assert_eq!(
        s,
        AxisScale {
            min: 0.0,
            max: 100.0,
            major: 25.0
        }
    );
    // A fixed min anchors the ticks: the max is min + k units.
    let s = auto_scale(4000.0, 33241.0, Some(2000.0), None, Some(5000.0), 10.0);
    assert_eq!((s.min, s.max), (2000.0, 37000.0));
    // Degenerate data.
    let s = auto_scale(0.0, 0.0, None, None, None, 5.0);
    assert!(s.max > s.min);
    let s = auto_scale(f64::NAN, f64::INFINITY, None, None, None, 5.0);
    assert!(s.max > s.min && s.major > 0.0);
}

#[test]
fn clustered_bars() {
    // One series, gap 150%: bar is 1/2.5 of the slot, centered.
    let (off, bw) = cluster_geometry(1, 150.0, 0.0, 0);
    assert!((bw - 0.4).abs() < 1e-9);
    assert!((off - 0.3).abs() < 1e-9);
    // Two series with full overlap share one position.
    let (o0, _) = cluster_geometry(2, 100.0, 100.0, 0);
    let (o1, _) = cluster_geometry(2, 100.0, 100.0, 1);
    assert!((o0 - o1).abs() < 1e-9);
    // Clusters are symmetric in the slot.
    let (o0, bw) = cluster_geometry(3, 50.0, -20.0, 0);
    let (o2, _) = cluster_geometry(3, 50.0, -20.0, 2);
    assert!(((o0) - (1.0 - (o2 + bw))).abs() < 1e-9);
}

#[test]
fn sections_respect_quotes() {
    assert_eq!(split_sections("0;\"a;b\";0"), vec!["0", "\"a;b\"", "0"]);
}

#[test]
fn fields_are_substituted() {
    let rich = crate::office::pptx::xml::parse(
        r#"<c:rich xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:p><a:fld id="1" type="VALUE"><a:rPr b="1"/><a:t>[VALUE]</a:t></a:fld><a:r><a:t> units</a:t></a:r></a:p></c:rich>"#,
    )
    .unwrap();
    let out = substitute_fields(&rich, &|t: &str| {
        if t == "VALUE" {
            Some("42".to_string())
        } else {
            None
        }
    });
    assert_eq!(out.all_text(), "42 units");
}

#[test]
fn pie_slice_paths() {
    assert!(slice_path(50.0, 50.0, 0.0, 40.0, 1.0, 0.0, 90.0).is_some());
    assert!(slice_path(50.0, 50.0, 20.0, 40.0, 1.0, 0.0, 360.0).is_some());
    assert!(slice_path(50.0, 50.0, 0.0, 40.0, 0.5, 30.0, 0.5).is_some());
}

#[test]
fn legacy_style_palettes() {
    assert!(matches!(style_palette(1), Palette::Gray));
    assert!(matches!(style_palette(2), Palette::Cycle { .. }));
    assert!(matches!(style_palette(3), Palette::Linear(_)));
    assert!(matches!(style_palette(42), Palette::Cycle { .. }));
}

/// Builds a one-slide package holding `chart_xml` in a full-slide frame and
/// renders it, returning the scene.
fn render_chart(chart_xml: &str) -> crate::office::pptx::scene::Scene {
    let frame = r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="2" name="Chart"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="0" y="0"/><a:ext cx="6000000" cy="4000000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId1"/></a:graphicData></a:graphic></p:graphicFrame>"#;
    render_frame(frame, "chart", "chart1.xml", chart_xml)
}

/// Renders one graphic frame whose `rId1` points at `ppt/charts/<name>`.
fn render_frame(
    frame: &str,
    rel_kind: &str,
    name: &str,
    chart_xml: &str,
) -> crate::office::pptx::scene::Scene {
    use std::io::Write;
    let ns_r = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let rel = |id: &str, kind: &str, target: &str| {
        format!(r#"<Relationship Id="{id}" Type="{ns_r}/{kind}" Target="{target}"/>"#)
    };
    let rels = |body: String| {
        format!(
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
        )
    };
    let parts: Vec<(String, String)> = vec![
        (
            "_rels/.rels".into(),
            rels(rel("rId1", "officeDocument", "ppt/presentation.xml")),
        ),
        (
            "ppt/presentation.xml".into(),
            format!(
                r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="{ns_r}"><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
            ),
        ),
        (
            "ppt/_rels/presentation.xml.rels".into(),
            rels(rel("rId1", "slide", "slides/slide1.xml")),
        ),
        (
            "ppt/slides/slide1.xml".into(),
            format!(
                r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="{ns_r}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><p:cSld><p:spTree>{frame}</p:spTree></p:cSld></p:sld>"#
            ),
        ),
        (
            "ppt/slides/_rels/slide1.xml.rels".into(),
            rels(rel("rId1", rel_kind, &format!("../charts/{name}"))),
        ),
        (format!("ppt/charts/{name}"), chart_xml.to_string()),
    ];
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, body) in &parts {
            z.start_file(
                name.as_str(),
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }
    let cfg = crate::office::pptx::config::Config::default();
    let doc = crate::office::pptx::document::Document::open(buf.into_inner(), &cfg.limits).unwrap();
    let fonts =
        crate::office::pptx::fonts::FontSystem::new(&[], false, &std::collections::HashMap::new());
    let chain = doc.chain(&doc.slides[0].part).unwrap();
    let images = crate::office::pptx::render::ImageCache::new(cfg.limits.max_image_pixels, &fonts);
    SlideBuilder::new(&doc, &fonts, &images, &cfg, &chain, 1).build()
}

fn chart_space(plot: &str) -> String {
    format!(
        r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea>{plot}</c:plotArea><c:legend><c:legendPos val="r"/></c:legend></c:chart></c:chartSpace>"#
    )
}

fn ser(idx: usize, vals: &[&str]) -> String {
    let pts: String = vals
        .iter()
        .enumerate()
        .map(|(i, v)| format!(r#"<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>"#))
        .collect();
    format!(
        r#"<c:ser><c:idx val="{idx}"/><c:order val="{idx}"/><c:val><c:numRef><c:numCache><c:ptCount val="{}"/>{pts}</c:numCache></c:numRef></c:val></c:ser>"#,
        vals.len()
    )
}

#[test]
fn doughnut_with_ragged_series_does_not_panic() {
    // The second ring has fewer points than the first (a corpus crash).
    let plot = format!(
        r#"<c:doughnutChart><c:varyColors val="1"/>{}{}<c:holeSize val="50"/></c:doughnutChart>"#,
        ser(0, &["1", "2", "3", "4"]),
        ser(1, &["5", "6"])
    );
    let scene = render_chart(&chart_space(&plot));
    assert!(scene.items.iter().any(|i| matches!(i, Item::Path(_))));
}

#[test]
fn malformed_charts_do_not_panic() {
    let cases = [
        // No axes, no data, NaN-like cache values.
        r#"<c:barChart><c:barDir val="col"/></c:barChart>"#.to_string(),
        format!(
            r#"<c:barChart><c:barDir val="bar"/><c:grouping val="percentStacked"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:scaling><c:max val="0"/><c:min val="0"/></c:scaling><c:crossAx val="1"/></c:valAx>"#,
            ser(0, &["#N/A", "1e308", "-1e308"])
        ),
        format!(
            r#"<c:lineChart>{}{}</c:lineChart>"#,
            ser(0, &["1"]),
            ser(1, &[])
        ),
        format!(
            r#"<c:scatterChart>{}</c:scatterChart>"#,
            ser(0, &["0", "0"])
        ),
        format!(r#"<c:pieChart>{}</c:pieChart>"#, ser(0, &["0", "-0"])),
        format!(
            r#"<c:areaChart><c:grouping val="stacked"/>{}{}<c:axId val="9"/><c:axId val="8"/></c:areaChart><c:valAx><c:axId val="8"/><c:scaling><c:logBase val="10"/></c:scaling></c:valAx>"#,
            ser(0, &["3", "-4"]),
            ser(1, &["1"])
        ),
    ];
    for plot in cases {
        drop(render_chart(&chart_space(&plot)));
    }
}

fn chart_warnings(scene: &crate::office::pptx::scene::Scene) -> Vec<String> {
    scene
        .warnings
        .iter()
        .filter(|(c, _)| c.starts_with("CHART"))
        .map(|(c, d)| format!("{c}: {d}"))
        .collect()
}

fn paths(scene: &crate::office::pptx::scene::Scene) -> usize {
    scene
        .items
        .iter()
        .filter(|i| matches!(i, Item::Path(_)))
        .count()
}

fn cat_ser(idx: usize, cats: &[&str], vals: &[&str]) -> String {
    let cpts: String = cats
        .iter()
        .enumerate()
        .map(|(i, v)| format!(r#"<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>"#))
        .collect();
    ser(idx, vals).replacen(
        "<c:val>",
        &format!(
            r#"<c:cat><c:strRef><c:strCache><c:ptCount val="{}"/>{cpts}</c:strCache></c:strRef></c:cat><c:val>"#,
            cats.len()
        ),
        1,
    )
}

#[test]
fn radar_styles_render() {
    let axes = r#"<c:axId val="1"/><c:axId val="2"/></STYLE><c:catAx><c:axId val="1"/><c:majorGridlines/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:majorGridlines/><c:crossAx val="1"/></c:valAx>"#;
    let mut counts = Vec::new();
    for style in ["standard", "marker", "filled"] {
        let plot = format!(
            r#"<c:radarChart><c:radarStyle val="{style}"/>{}{}{}"#,
            cat_ser(0, &["a", "b", "c", "d", "e"], &["1", "4", "3", "", "2"]),
            ser(1, &["2", "2", "2", "2", "2"]),
            axes.replace("STYLE", "c:radarChart")
        );
        let scene = render_chart(&chart_space(&plot));
        assert!(chart_warnings(&scene).is_empty(), "{style}");
        counts.push(paths(&scene));
    }
    // Markers add paths over the plain lines.
    assert!(counts[1] > counts[0], "{counts:?}");
}

#[test]
fn surface_charts_render_bands() {
    for kind in ["surfaceChart", "surface3DChart"] {
        let plot = format!(
            r#"<c:{kind}>{}{}{}<c:axId val="1"/><c:axId val="2"/><c:axId val="3"/></c:{kind}><c:catAx><c:axId val="1"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:crossAx val="1"/></c:valAx><c:serAx><c:axId val="3"/><c:crossAx val="2"/></c:serAx>"#,
            ser(0, &["1", "5", "2", "0"]),
            ser(1, &["3", "8", "1", "4"]),
            ser(2, &["0", "2", "6", "9"]),
        );
        let scene = render_chart(&chart_space(&plot));
        assert!(chart_warnings(&scene).is_empty(), "{kind}");
        assert!(paths(&scene) > 5, "{kind}");
    }
}

#[test]
fn band_pieces_cover_the_triangle() {
    let tri = [(0.0, 0.0, 0.0), (1.0, 0.0, 10.0), (0.0, 1.0, 5.0)];
    let edges = [0.0, 2.0, 4.0, 6.0, 8.0, 10.0];
    let pieces = surface::band_pieces(tri, &edges);
    let area = |p: &[(f64, f64, f64)]| {
        let mut a = 0.0;
        for i in 0..p.len() {
            let (x0, y0, _) = p[i];
            let (x1, y1, _) = p[(i + 1) % p.len()];
            a += x0 * y1 - x1 * y0;
        }
        a.abs() / 2.0
    };
    let total: f64 = pieces.iter().map(|(_, p)| area(p)).sum();
    assert!((total - 0.5).abs() < 1e-9, "{total}");
    assert_eq!(pieces.len(), 5);
    for (k, p) in &pieces {
        assert!(
            p.iter()
                .all(|v| v.2 >= edges[*k] - 1e-9 && v.2 <= edges[k + 1] + 1e-9)
        );
    }
}

/// A slide frame holding a chartEx part, with a fallback picture reference.
fn render_chartex(cx: &str) -> crate::office::pptx::scene::Scene {
    let frame = r#"<mc:AlternateContent><mc:Choice xmlns:cx1="http://schemas.microsoft.com/office/drawing/2015/9/8/chartex" Requires="cx1"><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="2" name="Chart"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="0" y="0"/><a:ext cx="6000000" cy="4000000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/drawing/2014/chartex"><cx:chart xmlns:cx="http://schemas.microsoft.com/office/drawing/2014/chartex" r:id="rId1"/></a:graphicData></a:graphic></p:graphicFrame></mc:Choice><mc:Fallback><p:pic><p:nvPicPr><p:cNvPr id="2" name="Chart"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId9"/></p:blipFill><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="6000000" cy="4000000"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr></p:pic></mc:Fallback></mc:AlternateContent>"#;
    render_frame(frame, "chartEx", "chartEx1.xml", cx)
}

fn cx_space(data: &str, series: &str, extra: &str) -> String {
    format!(
        r#"<cx:chartSpace xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:cx="http://schemas.microsoft.com/office/drawing/2014/chartex"><cx:chartData>{data}</cx:chartData><cx:chart><cx:title><cx:tx><cx:txData><cx:v>T</cx:v></cx:txData></cx:tx></cx:title><cx:plotArea><cx:plotAreaRegion>{series}</cx:plotAreaRegion><cx:axis id="0"><cx:catScaling gapWidth="0.5"/><cx:tickLabels/></cx:axis><cx:axis id="1"><cx:valScaling/><cx:majorGridlines/><cx:tickLabels/></cx:axis></cx:plotArea><cx:legend pos="t"/>{extra}</cx:chart></cx:chartSpace>"#
    )
}

fn cx_data(id: usize, cat_lvls: &[&[&str]], vals: &[&str]) -> String {
    let pts = |v: &[&str]| -> String {
        v.iter()
            .enumerate()
            .map(|(i, x)| format!(r#"<cx:pt idx="{i}">{x}</cx:pt>"#))
            .collect()
    };
    let lvls: String = cat_lvls
        .iter()
        .map(|l| format!(r#"<cx:lvl ptCount="{}">{}</cx:lvl>"#, l.len(), pts(l)))
        .collect();
    let cats = if cat_lvls.is_empty() {
        String::new()
    } else {
        format!(r#"<cx:strDim type="cat">{lvls}</cx:strDim>"#)
    };
    format!(
        r#"<cx:data id="{id}">{cats}<cx:numDim type="val"><cx:lvl ptCount="{}" formatCode="General">{}</cx:lvl></cx:numDim></cx:data>"#,
        vals.len(),
        pts(vals)
    )
}

fn cx_series(layout: &str, data: usize, inner: &str) -> String {
    format!(
        r#"<cx:series layoutId="{layout}"><cx:tx><cx:txData><cx:v>S{data}</cx:v></cx:txData></cx:tx><cx:dataLabels pos="outEnd"><cx:visibility seriesName="0" categoryName="0" value="1"/></cx:dataLabels><cx:dataId val="{data}"/>{inner}</cx:series>"#
    )
}

fn chart_kinds(scene: &crate::office::pptx::scene::Scene) -> Vec<String> {
    scene
        .charts
        .iter()
        .filter_map(|c| c["description"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn chartex_layouts_render_natively() {
    let cats: &[&str] = &["a", "b", "c", "d"];
    let cases = [
        (
            "waterfall",
            cx_space(
                &cx_data(0, &[cats], &["10", "-4", "3", "9"]),
                &cx_series(
                    "waterfall",
                    0,
                    r#"<cx:layoutPr><cx:subtotals><cx:idx val="3"/></cx:subtotals></cx:layoutPr>"#,
                ),
                "",
            ),
            "cxWaterfallChart",
        ),
        (
            "funnel",
            cx_space(
                &cx_data(0, &[cats], &["40", "30", "20", "5"]),
                &cx_series("funnel", 0, ""),
                "",
            ),
            "cxFunnelChart",
        ),
        (
            "boxWhisker",
            cx_space(
                &format!(
                    "{}{}",
                    cx_data(
                        0,
                        &[&["x", "x", "y", "y", "x", "y", "x"]],
                        &["1", "2", "3", "4", "5", "6", "40"]
                    ),
                    cx_data(1, &[], &["2", "3", "4"])
                ),
                &format!(
                    "{}{}",
                    cx_series(
                        "boxWhisker",
                        0,
                        r#"<cx:layoutPr><cx:visibility meanLine="1" meanMarker="1" nonoutliers="1" outliers="1"/><cx:statistics quartileMethod="inclusive"/></cx:layoutPr>"#
                    ),
                    cx_series("boxWhisker", 1, "")
                ),
                "",
            ),
            "cxBoxWhiskerChart",
        ),
        (
            "treemap",
            cx_space(
                &cx_data(
                    0,
                    &[&["l1", "l2", "l3", "l4"], &["p", "p", "q", ""]],
                    &["5", "3", "2", "1"],
                ),
                &cx_series(
                    "treemap",
                    0,
                    r#"<cx:layoutPr><cx:parentLabelLayout val="banner"/></cx:layoutPr>"#,
                ),
                "",
            ),
            "cxTreemapChart",
        ),
        (
            "sunburst",
            cx_space(
                &cx_data(
                    0,
                    &[&["l1", "l2", "l3"], &["p", "p", "q"]],
                    &["5", "3", "2"],
                ),
                &cx_series("sunburst", 0, ""),
                "",
            ),
            "cxSunburstChart",
        ),
        (
            "histogram",
            cx_space(
                &cx_data(0, &[], &["1", "2", "2", "3", "7", "8", "8", "9", "15"]),
                &cx_series(
                    "clusteredColumn",
                    0,
                    r#"<cx:layoutPr><cx:binning intervalClosed="r"><cx:binCount val="3"/></cx:binning></cx:layoutPr>"#,
                ),
                "",
            ),
            "barChart",
        ),
        (
            "pareto",
            cx_space(
                &cx_data(0, &[&["a", "b", "a", "c"]], &["1", "5", "2", "4"]),
                &format!(
                    "{}{}",
                    cx_series("clusteredColumn", 0, ""),
                    r#"<cx:series layoutId="paretoLine" ownerIdx="0"><cx:axisId val="2"/></cx:series>"#
                ),
                "",
            ),
            "barChart, lineChart",
        ),
    ];
    for (name, xml, kind) in cases {
        let scene = render_chartex(&xml);
        assert!(
            chart_warnings(&scene).is_empty(),
            "{name}: {:?}",
            chart_warnings(&scene)
        );
        assert_eq!(chart_kinds(&scene), vec![kind.to_string()], "{name}");
        assert!(paths(&scene) > 3, "{name}");
    }
}

#[test]
fn region_maps_use_the_fallback_picture() {
    let xml = cx_space(
        &cx_data(0, &[&["WA"]], &["1"]),
        &cx_series("regionMap", 0, ""),
        "",
    );
    let scene = render_chartex(&xml);
    assert_eq!(
        chart_warnings(&scene),
        vec!["CHART_PARTIAL: Chart: regionMap".to_string()]
    );
    assert!(scene.charts.is_empty());
}

#[test]
fn waterfall_running_totals() {
    let root = crate::office::pptx::xml::parse(&cx_space(
        &cx_data(0, &[&["a", "b", "c", "d"]], &["10", "-4", "3", "9"]),
        &cx_series(
            "waterfall",
            0,
            r#"<cx:layoutPr><cx:subtotals><cx:idx val="3"/></cx:subtotals></cx:layoutPr>"#,
        ),
        "",
    ))
    .unwrap();
    let synth = chartex::translate(&root, None).unwrap();
    let data = crate::office::pptx::chart::read_chart(&synth, "x");
    assert_eq!(data.groups[0].kind, "cxWaterfallChart");
    let s = &data.groups[0].series[0];
    let ser_node = data.groups[0].node.child("c:ser").unwrap().clone();
    let g = Grp {
        kind: Kind::Waterfall,
        node: data.groups[0].node.clone(),
        sers: vec![SerInfo {
            s: s.clone(),
            node: ser_node,
            vals: s.values.clone(),
            xs: Vec::new(),
            fmt: "General".into(),
            cat_fmt: None,
            range_texts: Vec::new(),
        }],
        grouping: Grouping::Clustered,
        vary: false,
        ax: (None, None),
        cat: 0,
        val: 0,
        secondary: false,
    };
    assert_eq!(
        g.stack()[0],
        vec![
            Some((0.0, 10.0)),
            Some((10.0, 6.0)),
            Some((6.0, 9.0)),
            Some((0.0, 9.0))
        ]
    );
}

#[test]
fn quartiles_match_excel() {
    let v: Vec<f64> = (1..=10).map(f64::from).collect();
    assert_eq!(chartex::quantile(&v, 0.25, true), 2.75);
    assert_eq!(chartex::quantile(&v, 0.25, false), 3.25);
    assert_eq!(chartex::quantile(&v, 0.75, true), 8.25);
    let st = chartex::box_stats(&[1.0, 2.0, 3.0, 4.0, 100.0], false).unwrap();
    assert_eq!(st.outliers, vec![100.0]);
    assert_eq!((st.lo, st.hi), (1.0, 4.0));
}

#[test]
fn tree_numbering_and_squarify() {
    let leaves: Vec<String> = ["x", "y", "z"].iter().map(|s| (*s).to_string()).collect();
    let parents: Vec<String> = ["p", "q", "p"].iter().map(|s| (*s).to_string()).collect();
    let t = chartex::build_tree(&[leaves, parents], &[Some(4.0), Some(2.0), Some(1.0)]);
    let by = |l: &str| t.iter().find(|n| n.label == l).unwrap();
    // Pre-order: p, x, z, q, y.
    assert_eq!(
        ["p", "x", "z", "q", "y"].map(|l| by(l).pre),
        [0, 1, 2, 3, 4]
    );
    assert_eq!(by("p").value, 5.0);
    assert_eq!(by("y").top, 1);
    let r = Rect {
        x: 0.0,
        y: 0.0,
        w: 60.0,
        h: 40.0,
    };
    let rects = chartex::squarify(&[6.0, 3.0, 2.0, 1.0], r);
    let areas: Vec<f64> = rects.iter().map(|q| q.w * q.h).collect();
    for (a, v) in areas.iter().zip([6.0, 3.0, 2.0, 1.0]) {
        assert!((a - v * 200.0).abs() < 1e-6, "{areas:?}");
    }
}
