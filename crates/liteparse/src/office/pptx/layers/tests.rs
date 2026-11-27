use super::*;
use crate::office::pptx::scene::{PathItem, ShapeInfo};

fn shape(inherited: bool) -> ShapeInfo {
    ShapeInfo {
        id: String::new(),
        name: String::new(),
        parent: None,
        kind: "sp",
        placeholder: None,
        bounds: crate::office::pptx::scene::Rect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        },
        rotation: 0.0,
        alt_text: None,
        hyperlink: None,
        paragraphs: Vec::new(),
        inherited,
        hidden: false,
    }
}

fn rect_item(x: f32, shape: usize) -> Item {
    let path =
        tiny_skia::PathBuilder::from_rect(tiny_skia::Rect::from_xywh(x, 0.0, 1.0, 1.0).unwrap());
    Item::Path(PathItem {
        path,
        fill: None,
        stroke: None,
        has_curve: false,
        shape: Some(shape),
        from_text: false,
    })
}

/// Shapes 0 and 1 come from the layout, 2 from the slide.
fn scene(xs: &[f32]) -> Scene {
    let mut s = Scene {
        shapes: vec![shape(true), shape(true), shape(false)],
        ..Default::default()
    };
    for (i, x) in xs.iter().enumerate() {
        s.items.push(rect_item(*x, i.min(2)));
    }
    s
}

const KEY: TargetKey = (10, 10, [0; 4]);

#[test]
fn reuses_the_shared_prefix() {
    let cache = LayerCache::default();
    let pm = Pixmap::new(10, 10).unwrap();
    // First slide: nothing cached, snapshot after the two inherited items.
    let p = cache.plan(&scene(&[1.0, 2.0, 3.0]), KEY);
    assert_eq!(
        (p.start, p.snapshot_at, p.base.is_some()),
        (0, Some(2), false)
    );
    cache.store(&p, &pm);
    // Same layout: resume after the inherited items, no new snapshot.
    let p = cache.plan(&scene(&[1.0, 2.0, 5.0]), KEY);
    assert_eq!((p.start, p.snapshot_at, p.base.is_some()), (2, None, true));
    // Other render size: no reuse.
    let p = cache.plan(&scene(&[1.0, 2.0, 5.0]), (20, 10, [0; 4]));
    assert_eq!(p.start, 0);
}

#[test]
fn narrows_to_the_part_slides_share() {
    let cache = LayerCache::default();
    let pm = Pixmap::new(10, 10).unwrap();
    // The second inherited item differs per slide (a slide number, say).
    let p = cache.plan(&scene(&[1.0, 2.0, 3.0]), KEY);
    cache.store(&p, &pm);
    let p = cache.plan(&scene(&[1.0, 7.0, 3.0]), KEY);
    assert_eq!((p.start, p.snapshot_at), (0, Some(1)));
    cache.store(&p, &pm);
    assert_eq!(cache.entries.lock().unwrap().len(), 1);
    let p = cache.plan(&scene(&[1.0, 8.0, 3.0]), KEY);
    assert_eq!((p.start, p.snapshot_at), (1, None));
}

fn picture_item(part: &str, shape: usize) -> Item {
    Item::Image(crate::office::pptx::scene::ImageItem {
        image: crate::office::pptx::scene::ImageRef {
            part: part.to_string(),
            effects: Vec::new(),
        },
        transform: tiny_skia::Transform::identity(),
        crop: (0.0, 0.0, 0.0, 0.0),
        alpha: 1.0,
        clip: None,
        shape: Some(shape),
        alt_text: None,
    })
}

#[test]
fn extends_a_shorter_hit_over_a_repeated_inherited_picture() {
    let cache = LayerCache::default();
    let pm = Pixmap::new(10, 10).unwrap();
    // Slide 1: one inherited rect, then its own rect.
    let mut s1 = scene(&[1.0]);
    s1.items.push(rect_item(5.0, 2));
    let p = cache.plan(&s1, KEY);
    assert_eq!(p.snapshot_at, Some(1));
    cache.store(&p, &pm);
    // Slides 2 and 3 also inherit a picture: resume after the rect; the longer
    // prefix is kept once a second slide repeats it.
    let mut s2 = scene(&[1.0]);
    s2.items.push(picture_item("ppt/media/image1.png", 1));
    s2.items.push(rect_item(5.0, 2));
    let p = cache.plan(&s2, KEY);
    assert_eq!((p.start, p.snapshot_at), (1, None));
    let p = cache.plan(&s2, KEY);
    assert_eq!((p.start, p.snapshot_at), (1, Some(2)));
    cache.store(&p, &pm);
    assert_eq!(cache.entries.lock().unwrap().len(), 2);
    let p = cache.plan(&s2, KEY);
    assert_eq!((p.start, p.snapshot_at), (2, None));
}

#[test]
fn covers_the_slides_own_pictures_once_they_repeat() {
    let cache = LayerCache::default();
    let pm = Pixmap::new(10, 10).unwrap();
    // Inherited rect, the slide's own picture, then its own rect.
    let slide = |part: &str| {
        let mut s = scene(&[1.0]);
        s.items.push(picture_item(part, 2));
        s.items.push(rect_item(5.0, 2));
        s
    };
    let p = cache.plan(&slide("ppt/media/image1.png"), KEY);
    assert_eq!(p.snapshot_at, Some(1));
    cache.store(&p, &pm);
    let p = cache.plan(&slide("ppt/media/image1.png"), KEY);
    assert_eq!((p.start, p.snapshot_at), (1, Some(2)));
    cache.store(&p, &pm);
    let p = cache.plan(&slide("ppt/media/image1.png"), KEY);
    assert_eq!((p.start, p.snapshot_at), (2, None));
    // Another picture, once: resume after the inherited part, no new snapshot.
    let p = cache.plan(&slide("ppt/media/image2.png"), KEY);
    assert_eq!((p.start, p.snapshot_at), (1, None));
}
