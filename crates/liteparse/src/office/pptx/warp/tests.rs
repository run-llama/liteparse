use super::*;

fn close(a: Pt, b: Pt, tol: f64) -> bool {
    (a.0 - b.0).abs() < tol && (a.1 - b.1).abs() < tol
}

#[test]
fn every_preset_evaluates() {
    let names: Vec<String> = DEFS.with(|d| d.keys().cloned().collect());
    assert!(names.len() >= 40, "{} presets", names.len());
    for n in names {
        let w = TextWarp::from_preset(&n, &[], 200.0, 100.0);
        assert!(w.is_some(), "{n} did not evaluate");
    }
}

#[test]
fn families_by_path_count() {
    let fit = [
        "textDeflate",
        "textWave4",
        "textArchUpPour",
        "textCanUp",
        "textFadeUp",
    ];
    for n in fit {
        assert!(
            TextWarp::from_preset(n, &[], 200.0, 100.0).unwrap().fits(),
            "{n}"
        );
    }
    for n in ["textArchUp", "textArchDown", "textCircle", "textButton"] {
        assert!(
            !TextWarp::from_preset(n, &[], 200.0, 100.0).unwrap().fits(),
            "{n}"
        );
    }
}

#[test]
fn fit_maps_corners_onto_the_guides() {
    // textSlantUp: top runs from (0, y1) up to (w, 0), bottom from (0, h) to (w, y2).
    let tw = TextWarp::from_preset("textSlantUp", &[], 200.0, 100.0).unwrap();
    let lw = tw
        .line(
            0,
            &[(80.0, 70.0, 20.0)],
            0.0,
            200.0,
            Align::Left,
            Transform::identity(),
        )
        .unwrap();
    let tl = lw.map_box((0.0, 0.0));
    let tr = lw.map_box((200.0, 0.0));
    let br = lw.map_box((200.0, 100.0));
    assert!(tl.1 > 40.0, "{tl:?}");
    assert!(close(tr, (200.0, 0.0), 1e-6), "{tr:?}");
    assert!(close(lw.map_box((0.0, 100.0)), (0.0, 100.0), 1e-6));
    assert!(br.1 < 60.0, "{br:?}");
    // Midway down the box is midway between the guides.
    let m = lw.map_box((100.0, 50.0));
    let (t, b) = (lw.map_box((100.0, 0.0)), lw.map_box((100.0, 100.0)));
    assert!(close(m, ((t.0 + b.0) / 2.0, (t.1 + b.1) / 2.0), 1e-6));
}

#[test]
fn fit_pairs_split_lines() {
    // Two pairs: the first line goes between paths 1/2, the second between 3/4.
    let tw = TextWarp::from_preset("textDeflateInflate", &[], 200.0, 100.0).unwrap();
    let m = [(20.0, 15.0, 5.0), (45.0, 15.0, 5.0)];
    let l0 = tw
        .line(0, &m, 0.0, 200.0, Align::Left, Transform::identity())
        .unwrap();
    let l1 = tw
        .line(1, &m, 0.0, 200.0, Align::Left, Transform::identity())
        .unwrap();
    // Line 0 fills band 0..0.5 of the box; its top-left is the first path's start.
    assert!(close(l0.map_box((0.0, 0.0)), (0.0, 0.0), 1e-6));
    assert!(l1.map_box((0.0, 100.0)).1 > 99.0);
}

#[test]
fn arch_up_stands_text_on_the_arc() {
    let (w, h) = (200.0, 200.0);
    let tw = TextWarp::from_preset("textArchUp", &[], w, h).unwrap();
    // One 20 pt line, centred, baseline at y = 16.
    let m = [(16.0, 16.0, 4.0)];
    let lw = tw
        .line(0, &m, 80.0, 40.0, Align::Center, Transform::identity())
        .unwrap();
    // The middle of the baseline lands on the top of the ellipse; glyphs stand above it.
    let base = lw.map_box((100.0, 16.0));
    assert!(close(base, (100.0, 0.0), 0.5), "{base:?}");
    assert!(lw.map_box((100.0, 0.0)).1 < -15.0);
    // Text keeps its size: 40 pt of baseline span about 40 pt of arc.
    let a = lw.map_box((80.0, 16.0));
    let b = lw.map_box((120.0, 16.0));
    let chord = (b.0 - a.0).hypot(b.1 - a.1);
    assert!((chord - 40.0).abs() < 1.0, "{chord}");
    // Ends of the line bend down.
    assert!(a.1 > 0.0 && b.1 > 0.0);
}

#[test]
fn arch_down_stands_text_on_the_arc() {
    let (w, h) = (200.0, 200.0);
    let tw = TextWarp::from_preset("textArchDown", &[], w, h).unwrap();
    let m = [(16.0, 16.0, 4.0), (40.0, 16.0, 4.0)];
    let l0 = tw
        .line(0, &m, 80.0, 40.0, Align::Center, Transform::identity())
        .unwrap();
    let l1 = tw
        .line(1, &m, 80.0, 40.0, Align::Center, Transform::identity())
        .unwrap();
    // The last line's baseline lands on the bottom of the ellipse, the first above it.
    let base = l1.map_box((100.0, 40.0));
    assert!(close(base, (100.0, 200.0), 0.5), "{base:?}");
    assert!(close(l0.map_box((100.0, 16.0)), (100.0, 176.0), 0.5));
    // Reading direction stays left to right, ends bend up.
    let (a, b) = (l1.map_box((80.0, 40.0)), l1.map_box((120.0, 40.0)));
    assert!(a.0 < b.0 && a.1 < 199.0 && b.1 < 199.0, "{a:?} {b:?}");
}

#[test]
fn circle_hangs_text_inside() {
    let tw = TextWarp::from_preset("textCircle", &[], 200.0, 200.0).unwrap();
    let m = [(16.0, 16.0, 4.0)];
    let lw = tw
        .line(0, &m, 80.0, 40.0, Align::Center, Transform::identity())
        .unwrap();
    // The default path is a full turn clockwise from the left, so centred text
    // lands on the right: its top on the circle, its baseline inside.
    let top = lw.map_box((100.0, 0.0));
    assert!(close(top, (200.0, 100.0), 1.0), "{top:?}");
    let base = lw.map_box((100.0, 16.0));
    assert!(close(base, (184.0, 100.0), 1.0), "{base:?}");
}

#[test]
fn warp_path_round_trips_slide_space() {
    let tw = TextWarp::from_preset("textDeflate", &[], 200.0, 100.0).unwrap();
    let to_slide = Transform::from_translate(50.0, 30.0);
    let lw = tw
        .line(0, &[(80.0, 70.0, 20.0)], 0.0, 200.0, Align::Left, to_slide)
        .unwrap();
    let mut pb = PathBuilder::new();
    pb.move_to(0.0, 0.0);
    pb.line_to(200.0, 0.0);
    pb.line_to(200.0, 100.0);
    pb.close();
    let p = lw.warp_path(&pb.finish().unwrap(), to_slide).unwrap();
    let b = p.bounds();
    // Corners stay on the box corners; the deflated top edge dips into the box.
    assert!((b.left() - 50.0).abs() < 0.01 && (b.top() - 30.0).abs() < 0.01);
    assert!((b.right() - 250.0).abs() < 0.01 && (b.bottom() - 130.0).abs() < 0.01);
    let mid_top = lw.map_point(Point::from_xy(150.0, 30.0));
    assert!(mid_top.y > 40.0, "{mid_top:?}");
}
