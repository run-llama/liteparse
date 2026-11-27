use super::*;

#[test]
fn rectangle_closes_implicitly_and_keeps_distances() {
    let c = Contour {
        points: vec![(0., 0.), (1., 0.), (1., 1.), (0., 1.)],
        through: false,
    };
    assert_eq!(
        c.bands(10., 20., 100., 50., [1., 2., 3., 4.]).unwrap(),
        vec![(19., 72., 7., 114.)]
    );
}

#[test]
fn through_keeps_a_concave_gap_and_tight_fills_it() {
    let mut c = Contour {
        points: vec![
            (0., 0.),
            (0.25, 0.),
            (0.25, 0.75),
            (0.75, 0.75),
            (0.75, 0.),
            (1., 0.),
            (1., 1.),
            (0., 1.),
        ],
        through: true,
    };
    let bands = c.bands(0., 0., 100., 100., [0.; 4]).unwrap();
    assert!(
        bands
            .iter()
            .filter(|b| b.0 < 20. && b.1 > 10.)
            .all(|b| b.3 <= 25. || b.2 >= 75.)
    );
    c.through = false;
    assert!(
        c.bands(0., 0., 100., 100., [0.; 4])
            .unwrap()
            .iter()
            .any(|b| b.0 < 20. && b.1 > 10. && b.2 == 0. && b.3 == 100.)
    );
}

#[test]
fn slanted_edges_follow_the_contour() {
    let c = Contour {
        points: vec![(0.5, 0.), (1., 1.), (0., 1.)],
        through: false,
    };
    let bands = c.bands(0., 0., 100., 100., [0.; 4]).unwrap();
    let first = bands[0];
    assert!(first.2 > 49. && first.3 < 51.);
    assert!(bands.last().unwrap().3 > 99.);
}

#[test]
fn malformed_coordinates_fall_back_and_flat_extents_are_empty() {
    let c = Contour {
        points: vec![(0., -100000.), (1., -100000.), (1., 1.)],
        through: false,
    };
    assert!(c.bands(0., 0., 100., 100., [0.; 4]).is_none());
    assert!(c.bands(0., 0., 100., 0., [0.; 4]).unwrap().is_empty());
}
