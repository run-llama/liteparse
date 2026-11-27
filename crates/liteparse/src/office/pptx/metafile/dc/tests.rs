use super::*;

fn apply(t: Transform, x: f32, y: f32) -> (f32, f32) {
    (t.sx * x + t.kx * y + t.tx, t.ky * x + t.sy * y + t.ty)
}

fn device() -> Space {
    Space::Device {
        dev_to_px: Transform::identity(),
        dev_per_mm: (4.0, 4.0),
    }
}

#[test]
fn colorref_is_bgr_packed() {
    assert_eq!(
        Rgb::from_colorref(0x00FF_8040),
        Rgb {
            r: 0x40,
            g: 0x80,
            b: 0xFF
        }
    );
}

#[test]
fn mm_text_only_translates_by_origins() {
    let dc = Dc {
        window_org: (10.0, 20.0),
        viewport_org: (1.0, 2.0),
        window_ext: (5.0, 5.0),
        ..Dc::default()
    };
    // Extents are ignored outside the isotropic/anisotropic modes.
    assert_eq!(apply(dc.transform(&device()), 10.0, 20.0), (1.0, 2.0));
    assert_eq!(apply(dc.transform(&device()), 11.0, 20.0), (2.0, 2.0));
}

#[test]
fn anisotropic_scales_each_axis_and_can_flip() {
    let dc = Dc {
        map_mode: MM_ANISOTROPIC,
        window_ext: (1000.0, 1000.0),
        viewport_ext: (100.0, -50.0),
        viewport_org: (0.0, 50.0),
        ..Dc::default()
    };
    assert_eq!(apply(dc.transform(&device()), 1000.0, 0.0), (100.0, 50.0));
    assert_eq!(apply(dc.transform(&device()), 0.0, 1000.0), (0.0, 0.0));
}

#[test]
fn isotropic_keeps_aspect_with_the_smaller_scale() {
    let dc = Dc {
        map_mode: MM_ISOTROPIC,
        window_ext: (100.0, 100.0),
        viewport_ext: (200.0, 50.0),
        ..Dc::default()
    };
    assert_eq!(apply(dc.transform(&device()), 100.0, 100.0), (50.0, 50.0));
}

#[test]
fn metric_modes_are_y_up_and_use_device_resolution() {
    // MM_HIMETRIC: 0.01 mm per unit at 4 device px per mm.
    let dc = Dc {
        map_mode: 3,
        ..Dc::default()
    };
    let (x, y) = apply(dc.transform(&device()), 100.0, 100.0);
    assert!((x - 4.0).abs() < 1e-4 && (y + 4.0).abs() < 1e-4, "{x},{y}");
}

#[test]
fn window_space_maps_window_onto_pixmap() {
    let dc = Dc {
        window_org: (100.0, 100.0),
        window_ext: (200.0, 100.0),
        ..Dc::default()
    };
    let t = dc.transform(&Space::Window {
        px_w: 400.0,
        px_h: 200.0,
    });
    assert_eq!(apply(t, 100.0, 100.0), (0.0, 0.0));
    assert_eq!(apply(t, 300.0, 200.0), (400.0, 200.0));
}

#[test]
fn world_transform_applies_before_page_transform() {
    let dc = Dc {
        world: Transform::from_translate(5.0, 0.0),
        viewport_org: (1.0, 0.0),
        ..Dc::default()
    };
    assert_eq!(apply(dc.transform(&device()), 0.0, 0.0), (6.0, 0.0));
}

#[test]
fn modify_world_modes() {
    let w = Transform::from_scale(2.0, 2.0);
    let x = Transform::from_translate(1.0, 0.0);
    assert_eq!(modify_world(w, x, 1), Transform::identity());
    // Left multiply: translate first, then scale.
    assert_eq!(apply(modify_world(w, x, 2), 0.0, 0.0), (2.0, 0.0));
    // Right multiply: scale first, then translate.
    assert_eq!(apply(modify_world(w, x, 3), 0.0, 0.0), (1.0, 0.0));
    assert_eq!(modify_world(w, x, 4), x);
    assert_eq!(modify_world(w, x, 99), w);
}

#[test]
fn zero_extents_do_not_produce_non_finite_transforms() {
    let dc = Dc {
        map_mode: MM_ANISOTROPIC,
        window_ext: (0.0, 0.0),
        ..Dc::default()
    };
    let t = dc.transform(&device());
    assert!(t.sx.is_finite() && t.sy.is_finite());
}
