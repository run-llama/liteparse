use super::*;
use tiny_skia::{Paint, PathBuilder, Rect};

fn square(l: f32, t: f32, r: f32, b: f32) -> tiny_skia::Path {
    PathBuilder::from_rect(Rect::from_ltrb(l, t, r, b).expect("rect"))
}

#[test]
fn box_coverage_is_linear_across_a_straight_edge() {
    // Left half inked: coverage steps by 1/(2r+1) per pixel around the edge.
    let (w, h, r) = (20, 9, 3);
    let data: Vec<u8> = (0..w * h)
        .map(|i| if i % w < 10 { 255 } else { 0 })
        .collect();
    let lut = table(|c| c);
    let row = box_alpha(&data, w, h, r, (0, 4, w, 5), &lut);
    for (x, got) in row.iter().enumerate().take(14).skip(7) {
        let want = (13 - x) as f32 / 7.0 * 255.0;
        assert!((*got as f32 - want).abs() <= 0.5, "x={x} {got} vs {want}");
    }
}

/// A red square inside a soft-edge group on white: what it leaves on row 50.
fn soft_row(radius: f64, l: f32, r: f32) -> Vec<[u8; 4]> {
    let mut pm = Pixmap::new(100, 100).expect("pixmap");
    pm.fill(tiny_skia::Color::WHITE);
    let path = square(l, 20.0, r, 80.0);
    let s = SoftEdge {
        path: path.clone(),
        radius,
    };
    let g = begin(&pm, &s, Transform::identity(), 1.0);
    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 0, 0, 255);
    pm.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    if let Some(g) = g {
        end(&mut pm, g);
    }
    (0..100)
        .map(|x| {
            let p = pm.pixel(x, 50).expect("pixel");
            [p.red(), p.green(), p.blue(), p.alpha()]
        })
        .collect()
}

#[test]
fn soft_edge_fades_inward_from_the_outline() {
    // rad 12 px: transparent up to 4.2 px inside the outline, opaque from 16.2 px.
    let row = soft_row(12.0, 20.0, 80.0);
    for p in &row[20..24] {
        assert_eq!(*p, [255, 255, 255, 255]);
    }
    assert_eq!(row[37], [255, 0, 0, 255]);
    assert_eq!(row[50], [255, 0, 0, 255]);
    for x in 20..40 {
        assert!(row[x + 1][1] <= row[x][1], "x={x}: {:?}", &row[18..42]);
    }
    let mid = row[29][1];
    assert!((100..156).contains(&mid), "{mid}");
}

#[test]
fn soft_edge_ignores_the_page_border() {
    // The square runs off the left edge of the page: no fade there.
    let row = soft_row(12.0, -40.0, 60.0);
    assert_eq!(row[0], [255, 0, 0, 255]);
    assert_eq!(row[59], [255, 255, 255, 255]);
}

#[test]
fn glow_is_full_on_the_outline_and_fades_outside() {
    let mut mask = Mask::new(100, 100).expect("mask");
    mask.fill_path(
        &square(40.0, 40.0, 60.0, 60.0),
        FillRule::Winding,
        false,
        Transform::identity(),
    );
    let region = glow_mask(&mut mask, 10.0, (0, 0, 100, 100)).expect("ink");
    let a = |x: usize| mask.data()[50 * 100 + x];
    assert_eq!(a(50), 255);
    assert!(a(60) > 200, "{}", a(60));
    for x in 60..70 {
        assert!(a(x + 1) <= a(x));
    }
    assert!(a(71) < 8, "{}", a(71));
    assert!(region.0 <= 31 && region.2 >= 69);
}

#[test]
fn feathered_mask_matches_the_soft_edge_fade() {
    let mut mask = Mask::new(100, 100).expect("mask");
    let path = square(20.0, 20.0, 80.0, 80.0);
    mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
    feather_mask(&mut mask, 12.0, (18, 18, 82, 82));
    let row = soft_row(12.0, 20.0, 80.0);
    for (x, px) in row.iter().enumerate().take(82).skip(18) {
        // The group's green channel shows how much red (alpha) got through.
        let alpha = 255 - px[1];
        let got = mask.data()[50 * 100 + x];
        assert!(got.abs_diff(alpha) <= 1, "x={x}: {got} vs {alpha}");
    }
}
