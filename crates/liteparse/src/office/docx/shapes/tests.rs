use super::*;

fn build_group(body: &str, attrs: &str) -> ShapeSpec {
    let node = crate::office::pptx::xml::parse(&format!(r#"<wpg:wgp xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><wpg:grpSpPr><a:xfrm {attrs}><a:off x="0" y="0"/><a:ext cx="1270000" cy="2540000"/><a:chOff x="100" y="200"/><a:chExt cx="1000" cy="1000"/></a:xfrm></wpg:grpSpPr>{body}</wpg:wgp>"#)).unwrap();
    let pkg = crate::office::pptx::package::Package::open(
        crate::office::docx::testutil::docx("", None, None),
        &Default::default(),
    )
    .unwrap();
    let theme = crate::office::pptx::theme::Theme::default();
    let ctx = StyleCtx {
        theme: &theme,
        colors: &theme.colors,
        part: "word/document.xml",
        theme_part: "",
        pkg: &pkg,
    };
    build(&node, 100.0, 200.0, &ctx, &|_| Vec::new())
}

fn chart(attrs: &str) -> String {
    format!(
        r#"<wpg:graphicFrame><wpg:xfrm {attrs}><a:off x="200" y="300"/><a:ext cx="400" cy="300"/></wpg:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rChart"/></a:graphicData></a:graphic></wpg:graphicFrame>"#
    )
}

#[test]
fn group_chart_maps_nonuniform_child_coordinates_once() {
    let group = build_group(&chart(""), "");
    assert_eq!(group.graphics.len(), usize::from(true));

    let g = &group.graphics[0];
    assert_eq!((g.w, g.h), (40.0, 60.0));
    assert_eq!(g.transform, Transform::from_translate(10.0, 20.0));
    assert_eq!(g.part, "word/document.xml");
    assert!(group.items.is_empty() && group.texts.is_empty());
}

#[test]
fn child_and_ancestor_turns_compose_like_existing_shape_paths() {
    let child_attrs = r#"rot="5400000" flipH="1""#;
    let shape = format!(
        r#"<wps:wsp><wps:spPr><a:xfrm {child_attrs}><a:off x="200" y="300"/><a:ext cx="400" cy="300"/></a:xfrm><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:ln><a:noFill/></a:ln></wps:spPr></wps:wsp>"#
    );
    let inner = format!(
        r#"<wpg:grpSp><wpg:grpSpPr><a:xfrm rot="1800000" flipV="1"><a:off x="100" y="200"/><a:ext cx="1000" cy="1000"/><a:chOff x="100" y="200"/><a:chExt cx="1000" cy="1000"/></a:xfrm></wpg:grpSpPr>{shape}{}</wpg:grpSp>"#,
        chart(child_attrs)
    );
    let group = build_group(&inner, r#"rot="10800000" flipH="1""#);

    let g = &group.graphics[0];
    // Three labelled basis points distinguish flips and corner permutations.
    let mut basis = [
        tiny_skia::Point::from_xy(0.0, 0.0),
        tiny_skia::Point::from_xy(1.0, 0.0),
        tiny_skia::Point::from_xy(0.0, 1.0),
    ];
    g.transform.map_points(&mut basis);
    let expected = [
        (43.660254, 69.01924),
        (43.160254, 68.15321),
        (42.794228, 69.51924),
    ];
    for (point, (x, y)) in basis.iter().zip(expected) {
        assert!(
            (point.x - x).abs() < 0.001 && (point.y - y).abs() < 0.001,
            "mapped basis {point:?}, expected ({x}, {y})"
        );
    }
    let mut corners = [
        tiny_skia::Point::from_xy(0.0, 0.0),
        tiny_skia::Point::from_xy(g.w as f32, 0.0),
        tiny_skia::Point::from_xy(g.w as f32, g.h as f32),
        tiny_skia::Point::from_xy(0.0, g.h as f32),
    ];
    g.transform.map_points(&mut corners);
    let bounds = group
        .items
        .iter()
        .find_map(|i| {
            if let Item::Path(p) = i {
                Some(p.path.bounds())
            } else {
                None
            }
        })
        .unwrap();
    let minx = corners.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let maxx = corners
        .iter()
        .map(|p| p.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let miny = corners.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let maxy = corners
        .iter()
        .map(|p| p.y)
        .fold(f32::NEG_INFINITY, f32::max);
    for (a, b) in [
        (bounds.left(), minx),
        (bounds.right(), maxx),
        (bounds.top(), miny),
        (bounds.bottom(), maxy),
    ] {
        assert!((a - b).abs() < 0.001, "{a} != {b}");
    }
}

fn upright_box(transform: Transform) -> TextBox {
    TextBox {
        rect: Rect {
            x: 10.0,
            y: 20.0,
            w: 40.0,
            h: 100.0,
        },
        inset: [1.0, 2.0, 3.0, 4.0],
        v_anchor: "ctr".into(),
        blocks: Vec::new(),
        bg: None,
        font_color: None,
        transform: Some(transform),
        upright: true,
        fit: false,
        clip: true,
        grow: false,
        nowrap: false,
        vert: None,
    }
}

#[test]
fn upright_frame_uses_complete_placement_and_preserves_source_properties() {
    let child = Transform::from_translate(30.0, 70.0)
        .pre_rotate(90.0)
        .pre_translate(-30.0, -70.0);
    let parent = Transform::from_translate(100.0, 120.0)
        .pre_rotate(270.0)
        .pre_translate(-100.0, -120.0);
    for (transform, expected_size) in [
        (child, (100.0, 40.0)),
        (child.post_concat(parent), (40.0, 100.0)),
        (child.post_scale(-1.0, 1.0), (100.0, 40.0)),
    ] {
        let mut source = upright_box(transform);
        source.vert = Some("vert270".into());
        let projected = source.upright_frame();
        let mut center = tiny_skia::Point::from_xy(30.0, 70.0);
        transform.map_point(&mut center);
        assert!((projected.rect.x + projected.rect.w / 2.0 - center.x as f64).abs() < 0.001);
        assert!((projected.rect.y + projected.rect.h / 2.0 - center.y as f64).abs() < 0.001);
        assert_eq!((projected.rect.w, projected.rect.h), expected_size);
        assert!(projected.transform.is_none());
        assert_eq!(projected.inset, source.inset);
        assert_eq!(projected.vert, source.vert);
        assert_eq!(projected.v_anchor, source.v_anchor);
        assert!(source.transform.is_some());
        source.upright = false;
        let unchanged = source.upright_frame();
        assert_eq!(unchanged.transform, source.transform);
        assert_eq!((unchanged.rect.w, unchanged.rect.h), (40.0, 100.0));
    }
}

#[test]
fn inline_translation_carries_pending_graphics_and_text_once() {
    let source = Transform::from_translate(8.0, 9.0).pre_rotate(90.0);
    let mut spec = ShapeSpec {
        texts: vec![upright_box(source)],
        graphics: vec![PendingGraphic {
            data: crate::office::pptx::xml::parse_word(
                r#"<a:graphicData xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"/>"#,
            )
            .unwrap(),
            part: "word/document.xml".into(),
            w: 40.0,
            h: 100.0,
            transform: source,
            insertion: 0,
        }],
        ..Default::default()
    };
    let old = spec.texts[0].upright_frame();
    spec.translate(30.0, -30.0);
    let new = spec.texts[0].upright_frame();
    assert!((new.rect.x - old.rect.x - 30.0).abs() < 0.001);
    assert!((new.rect.y - old.rect.y + 30.0).abs() < 0.001);
    assert_eq!(
        spec.graphics[0].transform,
        source.post_translate(30.0, -30.0)
    );
    assert_eq!((spec.graphics[0].w, spec.graphics[0].h), (40.0, 100.0));
}
