use super::*;

fn bbox(g: &Geometry) -> (f64, f64, f64, f64) {
    let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in &g.paths {
        for c in &p.cmds {
            let pts: Vec<(f64, f64)> = match *c {
                Cmd::Move(x, y) | Cmd::Line(x, y) => vec![(x, y)],
                Cmd::Quad(_, _, x, y) => vec![(x, y)],
                Cmd::Cubic(_, _, _, _, x, y) => vec![(x, y)],
                Cmd::Close => vec![],
            };
            for (x, y) in pts {
                b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
        }
    }
    b
}

fn preset(name: &str) -> XNode {
    let xml = format!(
        r#"<a:prstGeom xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" prst="{name}"><a:avLst/></a:prstGeom>"#
    );
    (*crate::office::pptx::xml::parse(&xml).unwrap()).clone()
}

#[test]
fn ellipse_spans_its_box() {
    // 100pt x 50pt
    let g = build(Some(&preset("ellipse")), 1_270_000.0, 635_000.0);
    let (l, t, r, b) = bbox(&g);
    assert!((l - 0.0).abs() < 0.01 && (t - 0.0).abs() < 0.01, "{l} {t}");
    assert!(
        (r - 100.0).abs() < 0.01 && (b - 50.0).abs() < 0.01,
        "{r} {b}"
    );
    // The ellipse text rect is inset (idx/idy at 45 degrees).
    assert!(
        g.text_rect.l > 14.0 && g.text_rect.l < 15.0,
        "{}",
        g.text_rect.l
    );
}

#[test]
fn every_preset_evaluates_inside_reasonable_bounds() {
    let names: Vec<String> = PRESETS.with(|p| p.keys().cloned().collect());
    assert!(names.len() > 180, "{}", names.len());
    for name in names {
        let g = build(Some(&preset(&name)), 1_270_000.0, 1_270_000.0);
        let (l, t, r, b) = bbox(&g);
        assert!(
            l > -200.0 && t > -200.0 && r < 300.0 && b < 300.0,
            "{name}: {l} {t} {r} {b}"
        );
    }
}

#[test]
fn round_rect_uses_adjust_override() {
    let xml = r#"<a:prstGeom xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" prst="roundRect"><a:avLst><a:gd name="adj" fmla="val 50000"/></a:avLst></a:prstGeom>"#;
    let node = crate::office::pptx::xml::parse(xml).unwrap();
    let g = build(Some(&node), 1_270_000.0, 635_000.0);
    // With adj=50% the corner radius is half the short side (25pt); first point is (0, 25).
    assert!(
        matches!(g.paths[0].cmds[0], Cmd::Move(x, y) if x.abs() < 0.01 && (y - 25.0).abs() < 0.01)
    );
}
