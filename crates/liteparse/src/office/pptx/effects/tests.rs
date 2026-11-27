use super::*;
use crate::office::pptx::color::ColorScheme;
use crate::office::pptx::config::Limits;
use crate::office::pptx::package::Package;
use crate::office::pptx::theme::Theme;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

fn with_ctx(theme: &Theme, f: impl FnOnce(&StyleCtx)) {
    // An empty zip (end-of-central-directory record only).
    let mut zip = vec![0x50, 0x4B, 0x05, 0x06];
    zip.extend([0u8; 18]);
    let pkg = Package::open(zip, &Limits::default()).expect("empty package");
    let colors = ColorScheme::default();
    f(&StyleCtx {
        theme,
        colors: &colors,
        part: "ppt/slides/slide1.xml",
        theme_part: "ppt/theme/theme1.xml",
        pkg: &pkg,
    });
}

fn xml(s: &str) -> std::rc::Rc<XNode> {
    crate::office::pptx::xml::parse(s).expect("xml")
}

#[test]
fn reads_own_glow_and_soft_edge() {
    let lst = xml(&format!(
        r#"<a:effectLst {NS}><a:glow rad="127000"><a:srgbClr val="FF0000"><a:alpha val="40000"/></a:srgbClr></a:glow><a:softEdge rad="63500"/></a:effectLst>"#
    ));
    with_ctx(&Theme::default(), |ctx| {
        let g = read_glow(Some(&lst), None, ctx).expect("glow");
        assert!(g.glow);
        assert!((g.blur - 10.0).abs() < 1e-9);
        assert!((g.color.r - 1.0).abs() < 1e-6 && (g.color.a - 0.4).abs() < 1e-3);
        assert_eq!(read_soft_edge(Some(&lst), None, ctx), Some(5.0));
    });
}

#[test]
fn own_empty_list_beats_theme_effect_style() {
    let style = xml(&format!(
        r#"<a:effectStyle {NS}><a:effectLst><a:softEdge rad="12700"/></a:effectLst></a:effectStyle>"#
    ));
    let theme = Theme {
        effect_styles: vec![style],
        ..Theme::default()
    };
    let r = xml(&format!(r#"<a:effectRef {NS} idx="1"/>"#));
    let empty = xml(&format!(r#"<a:effectLst {NS}/>"#));
    with_ctx(&theme, |ctx| {
        assert_eq!(read_soft_edge(None, Some(&r), ctx), Some(1.0));
        assert_eq!(read_soft_edge(Some(&empty), Some(&r), ctx), None);
        assert!(read_glow(None, Some(&r), ctx).is_none());
    });
}

#[test]
fn silhouette_of_unfilled_shape_is_its_stroke() {
    let g = crate::office::pptx::geom::build(None, 100.0 * EMU_PER_PT, 50.0 * EMU_PER_PT);
    let filled = silhouette(&g, Transform::identity(), true, None).expect("fill");
    let b = filled.bounds();
    assert!((b.width() - 100.0).abs() < 1e-3 && (b.height() - 50.0).abs() < 1e-3);
    assert!(silhouette(&g, Transform::identity(), false, None).is_none());
}

#[test]
fn only_a_multiply_fill_overlay_is_read() {
    let lst = |blend: &str| {
        xml(&format!(
            r#"<a:effectLst {NS}><a:fillOverlay blend="{blend}"><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></a:fillOverlay></a:effectLst>"#
        ))
    };
    let bounds = crate::office::pptx::scene::Rect {
        x: 0.0,
        y: 0.0,
        w: 10.0,
        h: 10.0,
    };
    with_ctx(&Theme::default(), |ctx| {
        let mult = read_fill_overlay(Some(&lst("mult")), None, ctx, bounds);
        assert!(matches!(mult, Some(Paint::Solid(c)) if c.g > 0.99 && c.r < 0.01));
        assert!(read_fill_overlay(Some(&lst("screen")), None, ctx, bounds).is_none());
    });
}
