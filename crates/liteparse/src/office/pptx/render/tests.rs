use super::*;

#[test]
fn streams_and_downscales_large_png() {
    // 300x100 RGB: left half red, right half blue; downscale to a 100 px long edge.
    let (w, h) = (300u32, 100u32);
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().unwrap();
        let mut data = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                data.extend_from_slice(if x < 150 { &[255, 0, 0] } else { &[0, 0, 255] });
            }
        }
        wr.write_image_data(&data).unwrap();
    }
    assert_eq!(png_size(&buf), Some((300, 100)));
    let pm = decode_png_downscaled(&buf, 100).unwrap();
    assert_eq!((pm.width(), pm.height()), (100, 34));
    assert_eq!(&pm.data()[0..4], &[255, 0, 0, 255]);
    let last = (pm.width() as usize - 1) * 4;
    assert_eq!(&pm.data()[last..last + 4], &[0, 0, 255, 255]);
}

/// The per-pixel loop `accumulate_row` replaced: output pixel `x / k`.
fn accumulate_reference(data: &[u8], channels: usize, k: usize, acc: &mut [u32]) {
    for x in 0..data.len() / channels {
        let px = &data[x * channels..x * channels + channels];
        let (r, g, b, a) = match channels {
            1 => (px[0], px[0], px[0], 255),
            2 => (px[0], px[0], px[0], px[1]),
            3 => (px[0], px[1], px[2], 255),
            _ => (px[0], px[1], px[2], px[3]),
        };
        let o = (x / k) * 4;
        acc[o] += r as u32;
        acc[o + 1] += g as u32;
        acc[o + 2] += b as u32;
        acc[o + 3] += a as u32;
    }
}

#[test]
fn row_accumulation_matches_per_pixel_sums() {
    let mut seed = 0x2545_f491_u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as u8
    };
    for channels in 1..=4 {
        for (w, k) in [(1usize, 1usize), (7, 1), (10, 3), (23, 7), (64, 8)] {
            let data: Vec<u8> = (0..w * channels).map(|_| next()).collect();
            let ow = w.div_ceil(k);
            let (mut got, mut want) = (vec![1u32; ow * 4], vec![1u32; ow * 4]);
            match channels {
                1 => accumulate_row::<1>(&data, k, &mut got),
                2 => accumulate_row::<2>(&data, k, &mut got),
                3 => accumulate_row::<3>(&data, k, &mut got),
                _ => accumulate_row::<4>(&data, k, &mut got),
            }
            accumulate_reference(&data, channels, k, &mut want);
            assert_eq!(got, want, "channels {channels}, w {w}, k {k}");
        }
    }
}

#[test]
fn to_bgr_drops_alpha_and_swaps_red_and_blue() {
    let mut pm = Pixmap::new(3, 2).unwrap();
    for (i, b) in pm.data_mut().iter_mut().enumerate() {
        *b = i as u8;
    }
    let want: Vec<u8> = pm
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[2], p[1], p[0]])
        .collect();
    assert_eq!(to_bgr(&pm), want);
}

#[test]
fn renders_svg_pictures() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#ff0000"/></svg>"##;
    let pm = render_svg(svg).unwrap();
    assert_eq!(pm.width(), 2 * pm.height());
    let px = pm.pixel(pm.width() / 2, pm.height() / 2).unwrap();
    assert_eq!(
        (px.red(), px.green(), px.blue(), px.alpha()),
        (255, 0, 0, 255)
    );
}

/// 64×48 RGB test picture: four flat quadrants, so 8×8 block averages are exact.
fn quadrants() -> (Vec<u8>, u16, u16) {
    let (w, h) = (64u16, 48u16);
    let mut px = Vec::new();
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(match (x < 32, y < 24) {
                (true, true) => &[200, 30, 30],
                (false, true) => &[30, 200, 30],
                (true, false) => &[30, 30, 200],
                (false, false) => &[240, 240, 240],
            });
        }
    }
    (px, w, h)
}

#[test]
fn jpeg_dc_decodes_baseline_and_progressive_at_one_eighth() {
    use jpeg_encoder::{ColorType, Encoder, SamplingFactor};
    let (px, w, h) = quadrants();
    for (progressive, restart, sampling) in [
        (false, 0, SamplingFactor::R_4_2_0),
        (false, 3, SamplingFactor::R_4_4_4),
        (true, 0, SamplingFactor::R_4_2_0),
        (true, 2, SamplingFactor::R_4_4_4),
    ] {
        let mut buf = Vec::new();
        let mut enc = Encoder::new(&mut buf, 95);
        enc.set_progressive(progressive);
        enc.set_restart_interval(restart);
        enc.set_sampling_factor(sampling);
        enc.encode(&px, w, h, ColorType::Rgb).unwrap();
        assert_eq!(jpeg_dc::size(&buf), Some((64, 48)));
        let img = jpeg_dc::decode(&buf).unwrap().into_rgb8();
        assert_eq!(img.dimensions(), (8, 6));
        let case = (progressive, restart);
        for (x, y, want) in [
            (0, 0, [200, 30, 30]),
            (7, 0, [30, 200, 30]),
            (0, 5, [30, 30, 200]),
        ] {
            let got = img.get_pixel(x, y).0;
            for c in 0..3 {
                assert!(
                    (got[c] as i32 - want[c]).abs() <= 12,
                    "{case:?} {x},{y}: {got:?}"
                );
            }
        }
    }
}

/// Little-endian TIFF with one strip of uncompressed data and the given IFD
/// entries (tag, type, count, value-or-offset), followed by `extra` at 8.
fn tiff_le(entries: &[(u16, u16, u32, u32)], extra: &[u8]) -> Vec<u8> {
    let mut b = b"II*\0".to_vec();
    let ifd = 8 + extra.len() as u32;
    b.extend_from_slice(&ifd.to_le_bytes());
    b.extend_from_slice(extra);
    b.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for &(tag, ty, count, v) in entries {
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&ty.to_le_bytes());
        b.extend_from_slice(&count.to_le_bytes());
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&0u32.to_le_bytes());
    b
}

#[test]
fn decodes_palette_and_cmyk_alpha_tiffs() {
    // 2×1 8-bit palette picture: index 1 (red), index 2 (blue).
    let mut extra = vec![1u8, 2, 0, 0];
    let cmap_at = 8 + extra.len() as u32;
    let mut map = vec![0u16; 768];
    map[1] = 0xFFFF;
    map[512 + 2] = 0xFFFF;
    extra.extend(map.iter().flat_map(|v| v.to_le_bytes()));
    let b = tiff_le(
        &[
            (256, 3, 1, 2),
            (257, 3, 1, 1),
            (258, 3, 1, 8),
            (259, 3, 1, 1),
            (262, 3, 1, 3),
            (273, 4, 1, 8),
            (277, 3, 1, 1),
            (278, 3, 1, 1),
            (279, 4, 1, 2),
            (320, 3, 768, cmap_at),
        ],
        &extra,
    );
    assert!(image::load_from_memory(&b).is_err());
    let img = tiff_extra::decode(&b, 1 << 20).unwrap().into_rgba8();
    assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(img.get_pixel(1, 0).0, [0, 0, 255, 255]);

    // 2×1 CMYK + unassociated alpha: pure cyan opaque, then 50 % black at half alpha.
    let bps_at = 8 + 12u32;
    let mut extra = vec![255u8, 0, 0, 0, 255, 0, 0, 0, 128, 128, 0, 0];
    extra.extend([8u16; 5].iter().flat_map(|v| v.to_le_bytes()));
    let b = tiff_le(
        &[
            (256, 3, 1, 2),
            (257, 3, 1, 1),
            (258, 3, 5, bps_at),
            (259, 3, 1, 1),
            (262, 3, 1, 5),
            (273, 4, 1, 8),
            (277, 3, 1, 5),
            (278, 3, 1, 1),
            (279, 4, 1, 10),
            (338, 3, 1, 2),
        ],
        &extra,
    );
    assert!(image::load_from_memory(&b).is_err());
    let img = tiff_extra::decode(&b, 1 << 20).unwrap().into_rgba8();
    assert_eq!(img.get_pixel(0, 0).0, [0, 255, 255, 255]);
    assert_eq!(img.get_pixel(1, 0).0, [127, 127, 127, 128]);
}

/// Stores `value` under `key` as a finished decode would.
fn put(store: &ImageStore, key: &str, value: Option<Arc<Pixmap>>) {
    let slot = store.slot(key);
    drop(slot.set(value.clone()));
    store.filled(key, value.as_ref());
}

#[test]
fn release_drops_finished_pictures_but_keeps_failures() {
    let store = ImageStore::default();
    let pm = Arc::new(Pixmap::new(4, 4).unwrap());
    put(&store, "ppt/media/a.png", Some(pm.clone()));
    put(&store, "ppt/media/a.png#[Grayscale]", Some(pm.clone()));
    put(&store, "ppt/media/b.png", Some(pm));
    put(&store, "ppt/media/bad.png", None);
    store.release(|part| part != "ppt/media/b.png");
    let inner = store.lock();
    let keys: std::collections::BTreeSet<String> = inner.decoded.keys().cloned().collect();
    assert_eq!(
        keys.into_iter().collect::<Vec<_>>(),
        vec!["ppt/media/b.png", "ppt/media/bad.png"]
    );
    assert_eq!(inner.cached_bytes, 64);
    assert_eq!(inner.lru.len(), 2);
}

#[test]
fn store_evicts_least_recent_past_budget() {
    let store = ImageStore::default();
    // Three pictures of 40% of the budget each: storing the third evicts the
    // least recently used one, which a lookup of `a` makes `b`.
    let edge = ((CACHE_BUDGET_BYTES as f64 * 0.4 / 4.0).sqrt()) as u32;
    let big = || Some(Arc::new(Pixmap::new(edge, edge).unwrap()));
    put(&store, "a", big());
    put(&store, "b", big());
    store.slot("a");
    put(&store, "c", big());
    let inner = store.lock();
    let keys: std::collections::BTreeSet<&str> = inner.decoded.keys().map(String::as_str).collect();
    assert_eq!(keys.into_iter().collect::<Vec<_>>(), vec!["a", "c"]);
    assert_eq!(inner.cached_bytes, 2 * edge as u64 * edge as u64 * 4);
}

#[test]
fn concurrent_requests_decode_a_picture_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let store = Arc::new(ImageStore::default());
    let decodes = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..4 {
            let (store, decodes) = (store.clone(), &decodes);
            s.spawn(move || {
                let fonts = FontSystem::new(&[], false, &HashMap::new());
                let cache = ImageCache::with_store(store, 1 << 20, &fonts);
                for _ in 0..50 {
                    let pm = cache.fill("ppt/media/a.png", || {
                        decodes.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        Some(Arc::new(Pixmap::new(2, 2).unwrap()))
                    });
                    assert_eq!(pm.map(|p| p.width()), Some(2));
                }
            });
        }
    });
    assert_eq!(decodes.load(Ordering::SeqCst), 1);
}

#[test]
fn failures_are_sorted_and_reported_once() {
    let store = ImageStore::default();
    for part in ["ppt/media/b.png", "ppt/media/a.png", "ppt/media/b.png"] {
        store
            .lock()
            .failures
            .insert((part.to_string(), "bad".to_string()));
    }
    assert_eq!(
        store.failures(),
        vec![
            ("ppt/media/a.png".to_string(), "bad".to_string()),
            ("ppt/media/b.png".to_string(), "bad".to_string())
        ]
    );
}

#[test]
fn pixel_count_reads_the_header() {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, 300, 20);
        enc.set_color(png::ColorType::Rgb);
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(&vec![0u8; 300 * 20 * 3]).unwrap();
    }
    assert_eq!(pixel_count(&buf), 6000);
    assert_eq!(pixel_count(b"not a picture"), 0);
}

#[test]
fn hairline_rectangles_widen_to_one_device_pixel_and_others_do_not() {
    let rect = |x, y, w, h| {
        tiny_skia::PathBuilder::from_rect(tiny_skia::Rect::from_xywh(x, y, w, h).unwrap())
    };
    let scale = Transform::from_scale(60.0 / 72.0, 60.0 / 72.0);
    // A 0.25 pt border at 60 dpi is about 0.2 px tall: widened to 1 px, same centre.
    let rule = thin_rule(&rect(10.0, 20.0, 100.0, 0.25), scale).unwrap();
    assert!((rule.height() - 1.0).abs() < 1e-4);
    assert!(((rule.top() + rule.bottom()) / 2.0 - 20.125 * 60.0 / 72.0).abs() < 1e-3);
    // Thick rectangles, non-rectangles and rotated canvases are left alone.
    assert!(thin_rule(&rect(10.0, 20.0, 100.0, 5.0), scale).is_none());
    let mut tri = tiny_skia::PathBuilder::new();
    tri.move_to(0.0, 0.0);
    tri.line_to(100.0, 0.1);
    tri.line_to(50.0, 0.2);
    tri.close();
    assert!(thin_rule(&tri.finish().unwrap(), scale).is_none());
    assert!(thin_rule(&rect(10.0, 20.0, 100.0, 0.25), Transform::from_rotate(30.0)).is_none());
}

struct NoAssets;

impl RenderAssets for NoAssets {
    fn read(&self, part: &str) -> crate::office::pptx::error::DocResult<Vec<u8>> {
        Err(crate::office::pptx::error::DocError::Broken(format!(
            "no part {part}"
        )))
    }

    fn read_head(&self, part: &str, _max: u64) -> crate::office::pptx::error::DocResult<Vec<u8>> {
        self.read(part)
    }

    fn part_size(&self, _part: &str) -> Option<u64> {
        None
    }
}

#[test]
fn fill_overlay_multiplies_the_pixels_beneath() {
    // A mid-grey square under a red multiply overlay turns dark red; the white
    // around it, where the overlay also falls, turns pure red.
    let fonts = FontSystem::new(&[], false, &HashMap::new());
    let images = ImageCache::new(1 << 20, &fonts);
    let mut pm = Pixmap::new(20, 10).unwrap();
    pm.fill(tiny_skia::Color::WHITE);
    let grey = tiny_skia::Paint {
        shader: tiny_skia::Shader::SolidColor(tiny_skia::Color::from_rgba8(128, 128, 128, 255)),
        ..Default::default()
    };
    pm.fill_rect(
        tiny_skia::Rect::from_xywh(0.0, 0.0, 10.0, 10.0).unwrap(),
        &grey,
        Transform::identity(),
        None,
    );
    let overlay = PathItem {
        path: tiny_skia::PathBuilder::from_rect(
            tiny_skia::Rect::from_xywh(0.0, 0.0, 20.0, 10.0).unwrap(),
        ),
        fill: Some(Paint::Solid(Rgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        })),
        stroke: None,
        has_curve: false,
        shape: None,
        from_text: false,
    };
    draw_multiply(
        &mut pm,
        &overlay,
        Transform::identity(),
        None,
        &NoAssets,
        &images,
    );
    let px = |x| {
        let p = pm.pixel(x, 5).unwrap();
        (p.red(), p.green(), p.blue())
    };
    assert_eq!(px(5), (128, 0, 0));
    assert_eq!(px(15), (255, 0, 0));
}
