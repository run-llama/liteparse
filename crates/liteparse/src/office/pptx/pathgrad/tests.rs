use super::*;
use crate::office::pptx::color::Rgba;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

fn stop(pos: f32, v: f32) -> GradientStop {
    GradientStop {
        pos,
        color: Rgba {
            r: v,
            g: v,
            b: v,
            a: 1.0,
        },
    }
}

/// A gradient over a 400 × 300 box from `path` and `tile` XML fragments.
fn grad(path: &str, tile: &str, stops: Vec<GradientStop>) -> PathGradient {
    let xml = format!("<a:gradFill {NS}>{path}{tile}</a:gradFill>");
    let node = crate::office::pptx::xml::parse(&xml).unwrap();
    let p = node.child("a:path").unwrap();
    let b = Rect {
        x: 0.0,
        y: 0.0,
        w: 400.0,
        h: 300.0,
    };
    from_xml(&node, p, b, stops)
}

fn two() -> Vec<GradientStop> {
    vec![stop(0.0, 1.0), stop(1.0, 0.0)]
}

fn three() -> Vec<GradientStop> {
    vec![stop(0.0, 1.0), stop(0.4, 0.8), stop(1.0, 0.0)]
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn centred_circle_reaches_the_corners() {
    let g = grad(
        r#"<a:path path="circle"><a:fillToRect l="50000" t="50000" r="50000" b="50000"/></a:path>"#,
        "",
        three(),
    );
    assert!(close(g.contour(200.0, 150.0), 0.0));
    assert!(close(g.contour(0.0, 0.0), 1.0));
    assert!(close(g.contour(400.0, 300.0), 1.0));
    // Circles, not ellipses: equal distances give equal contours.
    assert!(close(g.contour(300.0, 150.0), g.contour(200.0, 250.0)));
    assert!(close(g.contour(300.0, 150.0), 100.0 / 250.0));
    assert!(!g.quadratic);
}

#[test]
fn two_end_stops_are_squared() {
    let g = grad(
        r#"<a:path path="circle"><a:fillToRect l="50000" t="50000" r="50000" b="50000"/></a:path>"#,
        "",
        two(),
    );
    assert!(g.quadratic);
    let s = g.contour(300.0, 150.0);
    assert!(close(g.param(300.0, 150.0), s * s));
}

#[test]
fn crossed_insets_collapse_onto_the_first() {
    // l=50% r=100% spans 0.5 → 0: the focus stays at 50 %, not the 25 % midpoint.
    let g = grad(
        r#"<a:path path="circle"><a:fillToRect l="50000" t="50000" r="100000" b="100000"/></a:path>"#,
        "",
        three(),
    );
    assert_eq!(g.focus, (200.0, 150.0));
    assert_eq!(g.focus_half, (0.0, 0.0));
}

#[test]
fn missing_insets_are_zero() {
    // Bottom-right corner focus: only l and t are written.
    let g = grad(
        r#"<a:path path="circle"><a:fillToRect l="100000" t="100000"/></a:path>"#,
        "",
        three(),
    );
    assert_eq!(g.focus, (400.0, 300.0));
    assert!(close(g.contour(400.0, 300.0), 0.0));
    assert!(close(g.contour(0.0, 0.0), 1.0));
}

#[test]
fn focus_outside_the_box_is_focal() {
    // Focus 80 % above the box; contours are circles moving towards the centre.
    let g = grad(
        r#"<a:path path="circle"><a:fillToRect l="50000" t="-80000" r="50000" b="180000"/></a:path>"#,
        "",
        three(),
    );
    assert_eq!(g.focus, (200.0, -240.0));
    // Contour s: centre y = -240 + 390 s, radius 250 s; its bottom reaches y = 0
    // at s = 240 / 640.
    assert!(close(g.contour(200.0, 0.0), 240.0 / 640.0));
    for (x, y) in [(0.0, 0.0), (400.0, 0.0), (0.0, 300.0), (400.0, 300.0)] {
        assert!(close(g.contour(x, y), 1.0), "corner {x},{y}");
    }
}

#[test]
fn tile_rect_moves_the_outer_circle() {
    // The tile doubles the box to the right and down: concentric about the corner.
    let g = grad(
        r#"<a:path path="circle"><a:fillToRect l="100000" t="100000"/></a:path>"#,
        r#"<a:tileRect r="-100000" b="-100000"/>"#,
        three(),
    );
    assert_eq!(g.outer, (400.0, 300.0));
    assert!(close(g.contour(400.0, 0.0), 300.0 / 500.0));
    assert!(close(g.contour(0.0, 0.0), 1.0));
}

#[test]
fn segment_focus_is_zero_on_the_segment() {
    let g = grad(
        r#"<a:path path="circle"><a:fillToRect l="10000" t="-25000" r="10000" b="125000"/></a:path>"#,
        "",
        three(),
    );
    assert_eq!(g.focus_half, (160.0, 0.0));
    assert!(close(g.contour(100.0, -75.0), 0.0));
    let (a, b) = (g.contour(200.0, 100.0), g.contour(200.0, 250.0));
    assert!(a > 0.0 && a < b && b < 1.0);
}

#[test]
fn rect_path_reaches_the_edges() {
    let g = grad(
        r#"<a:path path="rect"><a:fillToRect l="50000" t="50000" r="50000" b="50000"/></a:path>"#,
        "",
        three(),
    );
    assert!(close(g.contour(400.0, 150.0), 1.0));
    assert!(close(g.contour(200.0, 0.0), 1.0));
    assert!(close(g.contour(300.0, 150.0), 0.5));
    assert!(close(g.contour(300.0, 225.0), 0.5));
}

#[test]
fn shape_path_draws_as_rect() {
    let g = grad(r#"<a:path path="shape"/>"#, "", three());
    assert_eq!(g.kind, PathKind::Rect);
    // No fillToRect: the focus is the whole box.
    assert!(close(g.contour(10.0, 10.0), 0.0));
}

#[test]
fn raster_centre_takes_the_first_stop() {
    let g = grad(
        r#"<a:path path="circle"><a:fillToRect l="50000" t="50000" r="50000" b="50000"/></a:path>"#,
        "",
        three(),
    );
    let pm = g.raster(1.0).unwrap();
    assert_eq!((pm.width(), pm.height()), (400, 300));
    let c = pm.pixel(200, 150).unwrap();
    assert!(c.red() >= 250);
    let k = pm.pixel(0, 0).unwrap();
    assert!(k.red() <= 5);
    // The long side is capped.
    assert_eq!(g.raster(10.0).unwrap().width(), 512);
}
