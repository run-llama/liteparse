use super::*;

fn apply(t: Transform, x: f32, y: f32) -> (f32, f32) {
    let mut p = [tiny_skia::Point::from_xy(x, y)];
    t.map_points(&mut p);
    (p[0].x, p[0].y)
}

#[test]
fn local_transform_rotates_about_the_centre() {
    // A 100x50 shape at (10, 20) rotated 90 degrees clockwise.
    let t = local_tf(10.0, 20.0, 100.0, 50.0, 90.0, false, false, false);
    let (x, y) = apply(t, 0.0, 0.0);
    // Top-left corner ends up at the top-right of the rotated footprint.
    assert!(
        (x - 85.0).abs() < 1e-3 && (y - (-5.0)).abs() < 1e-3,
        "{x} {y}"
    );
}

#[test]
fn text_transform_ignores_horizontal_flip() {
    let t = local_tf(0.0, 0.0, 100.0, 50.0, 0.0, true, false, true);
    assert_eq!(apply(t, 0.0, 0.0), (0.0, 0.0));
    let f = local_tf(0.0, 0.0, 100.0, 50.0, 0.0, true, false, false);
    assert_eq!(apply(f, 0.0, 0.0), (100.0, 0.0));
}

#[test]
fn group_map_scales_child_space() {
    let gm = GroupMap {
        tf: Transform::identity(),
        off: (127_000.0, 0.0),
        ch_off: (0.0, 0.0),
        scale: (0.5, 0.5),
    };
    let (x, y, w, h) = gm.map(254_000.0, 127_000.0, 254_000.0, 127_000.0);
    assert_eq!((x, y, w, h), (20.0, 5.0, 10.0, 5.0));
}

#[test]
fn placeholder_families() {
    assert_eq!(ph_family("ctrTitle"), "title");
    assert_eq!(ph_family("subTitle"), "body");
    assert_eq!(ph_family("obj"), "body");
    assert_eq!(ph_family("sldNum"), "sldNum");
}

#[test]
fn smart_art_nodes_follow_the_par_of_tree() {
    let dm = crate::office::pptx::xml::parse(r#"<dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
      <dgm:ptLst>
        <dgm:pt modelId="0" type="doc"/>
        <dgm:pt modelId="1"><dgm:t><a:p><a:r><a:t>Second</a:t></a:r></a:p></dgm:t></dgm:pt>
        <dgm:pt modelId="2"><dgm:t><a:p><a:r><a:t>First</a:t></a:r></a:p></dgm:t></dgm:pt>
        <dgm:pt modelId="3"><dgm:t><a:p><a:r><a:t>Child</a:t></a:r></a:p></dgm:t></dgm:pt>
        <dgm:pt modelId="9" type="sibTrans"/>
      </dgm:ptLst>
      <dgm:cxnLst>
        <dgm:cxn modelId="a" srcId="0" destId="1" srcOrd="1"/>
        <dgm:cxn modelId="b" srcId="0" destId="2" srcOrd="0"/>
        <dgm:cxn modelId="c" srcId="2" destId="3" srcOrd="0"/>
        <dgm:cxn modelId="d" type="presOf" srcId="1" destId="9"/>
      </dgm:cxnLst></dgm:dataModel>"#).unwrap();
    let nodes = smart_art_nodes(&dm);
    assert_eq!(
        nodes,
        vec![
            (0, "First".to_string()),
            (1, "Child".to_string()),
            (0, "Second".to_string())
        ]
    );
}

/// Builds a one-slide package whose shape tree is `tree` and renders it.
fn render_tree(tree: &str) -> crate::office::pptx::scene::Scene {
    use std::io::Write;
    let ns_r = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let rels = |id: &str, kind: &str, target: &str| {
        format!(
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="{id}" Type="{ns_r}/{kind}" Target="{target}"/></Relationships>"#
        )
    };
    let parts = [
        (
            "_rels/.rels".to_string(),
            rels("rId1", "officeDocument", "ppt/presentation.xml"),
        ),
        (
            "ppt/presentation.xml".into(),
            format!(
                r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="{ns_r}"><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
            ),
        ),
        (
            "ppt/_rels/presentation.xml.rels".into(),
            rels("rId1", "slide", "slides/slide1.xml"),
        ),
        (
            "ppt/slides/slide1.xml".into(),
            format!(
                r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="{ns_r}"><p:cSld><p:spTree>{tree}</p:spTree></p:cSld></p:sld>"#
            ),
        ),
    ];
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, body) in &parts {
            z.start_file(name.as_str(), zip::write::SimpleFileOptions::default())
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

/// A group (0, 0, 100 x 100 pt) with `fill`, holding a nested `grpFill` group whose
/// `grpFill` rectangle covers the group's right half.
fn grouped_rect(fill: &str) -> String {
    let xfrm = r#"<a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="1270000"/><a:chOff x="0" y="0"/><a:chExt cx="1270000" cy="1270000"/></a:xfrm>"#;
    format!(
        r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="2" name="G"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr>{xfrm}{fill}</p:grpSpPr><p:grpSp><p:nvGrpSpPr><p:cNvPr id="3" name="H"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr>{xfrm}<a:grpFill/></p:grpSpPr><p:sp><p:nvSpPr><p:cNvPr id="4" name="R"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="635000" y="0"/><a:ext cx="635000" cy="1270000"/></a:xfrm><a:prstGeom prst="rect"/><a:grpFill/></p:spPr></p:sp></p:grpSp></p:grpSp>"#
    )
}

fn first_fill(scene: &crate::office::pptx::scene::Scene) -> Option<Paint> {
    scene.items.iter().find_map(|i| match i {
        Item::Path(p) => p.fill.clone(),
        _ => None,
    })
}

#[test]
fn group_fill_comes_from_the_nearest_filled_group() {
    let scene = render_tree(&grouped_rect(
        r#"<a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>"#,
    ));
    match first_fill(&scene) {
        Some(Paint::Solid(c)) => assert!(c.r > 0.99 && c.g < 0.01, "{c:?}"),
        other => panic!("{other:?}"),
    }
    assert!(first_fill(&render_tree(&grouped_rect("<a:noFill/>"))).is_none());
    assert!(first_fill(&render_tree(&grouped_rect(""))).is_none());
}

#[test]
fn group_gradient_spans_the_group_box() {
    let grad = r#"<a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="000000"/></a:gs><a:gs pos="100000"><a:srgbClr val="FFFFFF"/></a:gs></a:gsLst><a:lin ang="0"/></a:gradFill>"#;
    match first_fill(&render_tree(&grouped_rect(grad))) {
        Some(Paint::Linear { start, end, .. }) => {
            assert!(
                start.0.abs() < 1e-3 && (end.0 - 100.0).abs() < 1e-3,
                "{start:?} {end:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn picture_fill_turns_with_its_shape_unless_told_not_to() {
    let image = |rot_with_shape| Paint::Image {
        image: crate::office::pptx::scene::ImageRef {
            part: "ppt/media/a.png".into(),
            effects: Vec::new(),
        },
        rect: Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
        },
        tile: None,
        alpha: 1.0,
        frame: Transform::identity(),
        rot_with_shape,
    };
    let tf = local_tf(10.0, 20.0, 100.0, 50.0, 90.0, false, true, false);
    match transform_paint(image(true), &tf) {
        Paint::Image { rect, frame, .. } => {
            assert_eq!(
                rect,
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 100.0,
                    h: 50.0
                }
            );
            assert_eq!(frame, tf);
        }
        other => panic!("{other:?}"),
    }
    match transform_paint(image(false), &tf) {
        Paint::Image { rect, frame, .. } => {
            // Upright over the turned footprint: 50 x 100 about the same centre.
            assert!(
                (rect.x - 35.0).abs() < 1e-3 && (rect.w - 50.0).abs() < 1e-3,
                "{rect:?}"
            );
            assert!(
                (rect.y + 5.0).abs() < 1e-3 && (rect.h - 100.0).abs() < 1e-3,
                "{rect:?}"
            );
            assert_eq!(frame, Transform::identity());
        }
        other => panic!("{other:?}"),
    }
}

const NS: &str = r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

/// A one-slide package whose slide, layout and master trees hold the given shapes.
fn package(slide: &str, layout: &str, master: &str) -> Document {
    package_with_styles(slide, layout, master, "")
}

/// `package` with `p:txStyles` children (`p:otherStyle`...) on the master.
fn package_with_styles(slide: &str, layout: &str, master: &str, styles: &str) -> Document {
    use std::io::Write;
    let ns_r = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let rels = |id: &str, kind: &str, target: &str| {
        format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="{id}" Type="{ns_r}/{kind}" Target="{target}"/></Relationships>"#
        )
    };
    let tree = |root: &str, body: &str| {
        let styles = if root == "sldMaster" {
            format!("<p:txStyles>{styles}</p:txStyles>")
        } else {
            String::new()
        };
        format!(r#"<p:{root} {NS}><p:cSld><p:spTree>{body}</p:spTree></p:cSld>{styles}</p:{root}>"#)
    };
    let parts = [
        (
            "_rels/.rels",
            rels("rId1", "officeDocument", "ppt/presentation.xml"),
        ),
        (
            "ppt/presentation.xml",
            format!(
                r#"<p:presentation {NS}><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
            ),
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            rels("rId1", "slide", "slides/slide1.xml"),
        ),
        ("ppt/slides/slide1.xml", tree("sld", slide)),
        (
            "ppt/slides/_rels/slide1.xml.rels",
            rels("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"),
        ),
        (
            "ppt/slideLayouts/slideLayout1.xml",
            tree("sldLayout", layout),
        ),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            rels("rId1", "slideMaster", "../slideMasters/slideMaster1.xml"),
        ),
        (
            "ppt/slideMasters/slideMaster1.xml",
            tree("sldMaster", master),
        ),
    ];
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, body) in &parts {
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            z.start_file(*name, opts).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }
    Document::open(buf.into_inner(), &Config::default().limits).unwrap()
}

/// A placeholder shape: `ph` is the `p:ph` attributes, `sp_pr` the `p:spPr` children.
fn ph_sp(id: u32, ph: &str, sp_pr: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="ph{id}"/><p:cNvSpPr/><p:nvPr><p:ph {ph}/></p:nvPr></p:nvSpPr><p:spPr>{sp_pr}</p:spPr></p:sp>"#
    )
}

fn xfrm(x: i64) -> String {
    format!(r#"<a:xfrm><a:off x="{x}" y="0"/><a:ext cx="1270000" cy="1270000"/></a:xfrm>"#)
}

/// The (layout, master) placeholder names each slide placeholder inherits from.
fn sources_of(doc: &Document) -> Vec<(Option<String>, Option<String>)> {
    let cfg = Config::default();
    let fonts = FontSystem::new(&[], false, &Default::default());
    let chain = doc.chain(&doc.slides[0].part).unwrap();
    let images = crate::office::pptx::render::ImageCache::new(cfg.limits.max_image_pixels, &fonts);
    let b = SlideBuilder::new(doc, &fonts, &images, &cfg, &chain, 1);
    let name = |n: Option<Rc<XNode>>| {
        n.and_then(|n| {
            n.path(&["p:nvSpPr", "p:cNvPr"])
                .and_then(|c| c.attr("name"))
                .map(String::from)
        })
    };
    tree_of(&chain.slide)
        .unwrap()
        .children
        .iter()
        .filter(|c| placeholder(c).is_some())
        .map(|c| {
            let (l, m) = b.ph_sources(c, PartKind::Slide);
            (name(l), name(m))
        })
        .collect()
}

#[test]
fn orphan_placeholder_inherits_from_the_master_only() {
    let doc = package(
        &[
            ph_sp(2, r#"idx="4294967295""#, &xfrm(0)),
            ph_sp(3, r#"type="title" idx="4294967295""#, &xfrm(0)),
            ph_sp(4, r#"type="body" idx="1""#, ""),
            ph_sp(5, r#"type="ctrTitle""#, ""),
        ]
        .concat(),
        &[
            ph_sp(10, r#"type="title""#, &xfrm(1)),
            ph_sp(11, r#"type="body" idx="1""#, &xfrm(2)),
        ]
        .concat(),
        &[
            ph_sp(20, r#"type="title""#, &xfrm(3)),
            ph_sp(21, r#"type="body" idx="1""#, &xfrm(4)),
        ]
        .concat(),
    );
    let s = |v: &str| Some(v.to_string());
    assert_eq!(
        sources_of(&doc),
        vec![
            // Orphans skip the layout even when it has a placeholder of their type.
            (None, s("ph21")),
            (None, s("ph20")),
            // Others match the layout by idx, then by type family.
            (s("ph11"), s("ph21")),
            (s("ph10"), s("ph20")),
        ]
    );
}

#[test]
fn picture_placeholder_inherits_layout_geometry_and_outline() {
    let pic = r#"<p:pic><p:nvPicPr><p:cNvPr id="2" name="Picture"/><p:cNvPicPr/><p:nvPr><p:ph type="pic" idx="10"/></p:nvPr></p:nvPicPr><p:blipFill/><p:spPr/></p:pic>"#;
    let layout = ph_sp(
        3,
        r#"type="pic" idx="10""#,
        &format!(
            r#"{}<a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom><a:ln w="12700"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:ln>"#,
            xfrm(0)
        ),
    );
    let doc = package(pic, &layout, "");
    let cfg = Config::default();
    let fonts = FontSystem::new(&[], false, &Default::default());
    let chain = doc.chain(&doc.slides[0].part).unwrap();
    let images = crate::office::pptx::render::ImageCache::new(cfg.limits.max_image_pixels, &fonts);
    let scene = SlideBuilder::new(&doc, &fonts, &images, &cfg, &chain, 1).build();
    let pic_idx = scene.shapes.iter().position(|s| s.kind == "pic").unwrap();
    let outline = scene.items.iter().find_map(|i| match i {
        Item::Path(p) if p.shape == Some(pic_idx) && p.stroke.is_some() => Some(p),
        _ => None,
    });
    // The layout's ellipse outline is drawn around the picture.
    assert!(outline.is_some_and(|p| p.has_curve));
}

#[test]
fn metafile_runs_turned_by_their_picture_run_along_the_box() {
    use crate::office::pptx::textlayout::transformed_bbox;
    for (angle, box_) in [
        (0.0, (10.0, 20.0, 40.0, 5.0)),
        (90.0, (10.0, 20.0, 5.0, 40.0)),
    ] {
        let (t, len, asc, desc) = metafile_run_placement(box_, angle);
        assert!((len - 40.0).abs() < 1e-9 && (asc + desc - 5.0).abs() < 1e-9);
        let b = transformed_bbox(&t, 0.0, -asc, len, asc + desc).unwrap();
        let got = (b.x, b.y, b.w, b.h);
        for (g, w) in [got.0, got.1, got.2, got.3]
            .iter()
            .zip([box_.0, box_.1, box_.2, box_.3])
        {
            assert!((g - w).abs() < 1e-3, "{angle}: {got:?} vs {box_:?}");
        }
    }
}

#[test]
fn layout_text_boxes_skip_the_master_other_style() {
    let text_box = |id: u32, text: &str| {
        format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="t{id}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr>{}</p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:rPr lang="en-US"/><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"#,
            xfrm(0)
        )
    };
    let other = r#"<p:otherStyle><a:lvl1pPr><a:defRPr sz="1000"/></a:lvl1pPr></p:otherStyle>"#;
    let doc = package_with_styles(
        &text_box(2, "S"),
        &text_box(3, "L"),
        &text_box(4, "M"),
        other,
    );
    let cfg = Config::default();
    let fonts = FontSystem::new(&[], true, &Default::default());
    let chain = doc.chain(&doc.slides[0].part).unwrap();
    let images = crate::office::pptx::render::ImageCache::new(cfg.limits.max_image_pixels, &fonts);
    let scene = SlideBuilder::new(&doc, &fonts, &images, &cfg, &chain, 1).build();
    let mut sizes: Vec<(String, f64)> = (scene.items.iter())
        .filter_map(|i| match i {
            Item::Text(r) => Some((r.text.clone(), r.size)),
            _ => None,
        })
        .collect();
    sizes.sort_by(|a, b| a.0.cmp(&b.0));
    if sizes.is_empty() {
        return; // no font installed to shape with
    }
    // Only the master's own text box takes otherStyle; the rest get the 18 pt default.
    let want = [("L", 18.0), ("M", 10.0), ("S", 18.0)].map(|(t, s)| (t.to_string(), s));
    assert_eq!(sizes, want.to_vec());
}

/// y (points) of every horizontal stroked segment the slide draws.
fn horizontal_lines(scene: &crate::office::pptx::scene::Scene) -> Vec<f64> {
    (scene.items.iter())
        .filter_map(|i| match i {
            Item::Path(p) if p.stroke.is_some() && p.path.bounds().height() < 0.01 => {
                Some(p.path.bounds().top() as f64)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn empty_table_rows_fit_their_end_marks_and_bands_draw_cell_sides() {
    let cell = |body: &str| {
        format!(
            r#"<a:tc><a:txBody><a:bodyPr/><a:p>{body}</a:p></a:txBody><a:tcPr marT="0" marB="0"/></a:tc>"#
        )
    };
    let row = |h: i64, body: &str| format!(r#"<a:tr h="{h}">{}</a:tr>"#, cell(body));
    let line = r#"<a:ln w="12700"><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill></a:ln>"#;
    // Light Style 2 shape: no inside lines for the whole table, cell sides on band1H.
    let style = format!(
        r#"<a:tableStyle styleId="{{00000000-0000-0000-0000-000000000001}}" styleName="t"><a:wholeTbl><a:tcStyle><a:tcBdr><a:insideH><a:ln><a:noFill/></a:ln></a:insideH></a:tcBdr></a:tcStyle></a:wholeTbl><a:band1H><a:tcStyle><a:tcBdr><a:top>{line}</a:top><a:bottom>{line}</a:bottom></a:tcBdr></a:tcStyle></a:band1H></a:tableStyle>"#
    );
    let table = format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="2" name="Table"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="1270000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr firstRow="1" bandRow="1">{style}</a:tblPr><a:tblGrid><a:gridCol w="2540000"/></a:tblGrid>{}{}{}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#,
        row(127000, r#"<a:endParaRPr lang="en-US" sz="3600"/>"#),
        row(
            254000,
            r#"<a:r><a:rPr lang="en-US" sz="1000"/><a:t>a</a:t></a:r>"#
        ),
        row(
            254000,
            r#"<a:r><a:rPr lang="en-US" sz="1000"/><a:t>b</a:t></a:r>"#
        ),
    );
    let doc = package(&table, "", "");
    let cfg = Config::default();
    let fonts = FontSystem::new(&[], true, &Default::default());
    let chain = doc.chain(&doc.slides[0].part).unwrap();
    let images = crate::office::pptx::render::ImageCache::new(cfg.limits.max_image_pixels, &fonts);
    let scene = SlideBuilder::new(&doc, &fonts, &images, &cfg, &chain, 1).build();
    let ys = horizontal_lines(&scene);
    // The empty 10 pt header grows to its 36 pt end mark (1.2 × 36); the first band
    // row draws its own top and bottom sides although wholeTbl has no inside lines.
    for want in [43.2, 63.2] {
        assert!(
            ys.iter().any(|y| (y - want).abs() < 0.05),
            "{want} in {ys:?}"
        );
    }
}

#[test]
fn newline_in_run_text_starts_a_paragraph() {
    let shape = |text: &str| {
        format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="t"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="6000000" cy="3000000"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:rPr lang="en-US" sz="1800"/><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"#
        )
    };
    let paragraphs = |text: &str| {
        let scene = render_tree(&shape(text));
        scene
            .shapes
            .iter()
            .map(|s| s.paragraphs.len())
            .sum::<usize>()
    };
    assert_eq!(paragraphs("one\ntwo"), 2);
    assert_eq!(paragraphs("one two"), 1);
}

#[test]
fn shape_reaching_far_off_the_slide_renders_its_visible_part() {
    // The rectangle starts 7.9e7 pt left of the slide and ends at its middle. At
    // 1000 px wide its left edge is 1.1e8 px out, past what tiny-skia can
    // anti-alias, and its right edge falls mid-pixel, which is what panicked.
    let far = 1_000_000_000_000i64;
    let sp = format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="R"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{}" y="0"/><a:ext cx="{}" cy="6858000"/></a:xfrm><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:ln w="12700"><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill></a:ln></p:spPr></p:sp>"#,
        -far,
        far + 4_572_000,
    );
    let doc = package(&sp, "", "");
    let cfg = Config::default();
    let fonts = FontSystem::new(&[], true, &Default::default());
    let chain = doc.chain(&doc.slides[0].part).unwrap();
    let images = crate::office::pptx::render::ImageCache::new(cfg.limits.max_image_pixels, &fonts);
    let scene = SlideBuilder::new(&doc, &fonts, &images, &cfg, &chain, 1).build();
    let target = crate::office::pptx::render::RenderTarget {
        region: Rect {
            x: 0.0,
            y: 0.0,
            w: 720.0,
            h: 540.0,
        },
        width: 1000,
        height: 750,
    };
    let pm = crate::office::pptx::render::render(&scene, &target, &doc, &fonts, &images).unwrap();
    let px = |x, y| {
        let p = pm.pixel(x, y).unwrap();
        [p.red(), p.green(), p.blue()]
    };
    assert_eq!(px(100, 375), [255, 0, 0]);
    assert_eq!(px(900, 375), [255, 255, 255]);
}

#[test]
fn a_picture_with_missing_data_draws_powerpoints_empty_box() {
    // An `r:embed` that resolves to nothing: a white box with a black outline over
    // the picture's frame. A linked picture (r:link) is left undrawn.
    let pic = |blip: &str| {
        format!(
            r#"<p:pic><p:nvPicPr><p:cNvPr id="2" name="P"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill>{blip}<a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="635000"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr></p:pic>"#
        )
    };
    let missing = render_tree(&pic(r#"<a:blip r:embed=""/>"#));
    let boxes: Vec<_> = missing
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Path(p) => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(boxes.len(), 1);
    assert!(
        matches!(boxes[0].fill, Some(Paint::Solid(c)) if c == crate::office::pptx::color::Rgba::WHITE)
    );
    assert!(boxes[0].stroke.as_ref().is_some_and(|s| s.width == 1.0));
    let b = boxes[0].path.bounds();
    assert!((b.width() - 100.0).abs() < 0.01 && (b.height() - 50.0).abs() < 0.01);
    let linked = render_tree(&pic(r#"<a:blip r:link="rId9"/>"#));
    assert!(!linked.items.iter().any(|i| matches!(i, Item::Path(_))));
}
