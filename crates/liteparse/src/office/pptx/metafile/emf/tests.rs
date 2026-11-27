use super::*;

/// Builds EMF bytes: a header with a 100 x 50 device-pixel frame (one pixel =
/// 0.25 mm, i.e. 25 hundredths of a mm) followed by `records` and EMR_EOF.
fn emf(records: &[Vec<u8>]) -> Vec<u8> {
    let mut h = Vec::new();
    let i = |v: &mut Vec<u8>, x: i32| v.extend_from_slice(&x.to_le_bytes());
    i(&mut h, 1);
    i(&mut h, 88);
    for v in [0, 0, 99, 49] {
        i(&mut h, v); // bounds
    }
    for v in [0, 0, 2500, 1250] {
        i(&mut h, v); // frame, 0.01 mm
    }
    h.extend_from_slice(b" EMF");
    i(&mut h, 0x10000);
    i(&mut h, 0); // nBytes, patched below
    i(&mut h, records.len() as i32 + 2);
    h.extend_from_slice(&8u16.to_le_bytes());
    h.extend_from_slice(&0u16.to_le_bytes());
    i(&mut h, 0);
    i(&mut h, 0);
    i(&mut h, 0);
    for v in [1000, 1000, 250, 250] {
        i(&mut h, v); // device px, device mm
    }
    for r in records {
        h.extend_from_slice(r);
    }
    h.extend_from_slice(&rec(14, &[0, 0, 16]));
    let n = h.len() as i32;
    h[48..52].copy_from_slice(&n.to_le_bytes());
    h
}

fn rec(t: u32, params: &[i32]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&t.to_le_bytes());
    v.extend_from_slice(&(8 + params.len() as u32 * 4).to_le_bytes());
    for p in params {
        v.extend_from_slice(&p.to_le_bytes());
    }
    v
}

fn fonts() -> FontSystem {
    FontSystem::new(&[], false, &Default::default())
}

fn px(pm: &Pixmap, x: u32, y: u32) -> [u8; 4] {
    let c = pm.pixel(x, y).unwrap();
    [c.red(), c.green(), c.blue(), c.alpha()]
}

#[test]
fn header_frame_and_device_scale() {
    let bytes = emf(&[]);
    let h = Header::parse(&bytes).unwrap();
    assert_eq!(h.frame, (0, 0, 2500, 1250));
    assert_eq!(h.hmm_per_device(), (25.0, 25.0));
    assert_eq!(h.frame_hmm(), Some((0.0, 0.0, 2500.0, 1250.0)));
}

#[test]
fn description_text_is_not_read_as_micrometers() {
    let mut bytes = emf(&[]);
    // Claim a 120-byte header whose description starts at offset 88.
    bytes[4..8].copy_from_slice(&120u32.to_le_bytes());
    bytes[60..64].copy_from_slice(&4u32.to_le_bytes());
    bytes[64..68].copy_from_slice(&88u32.to_le_bytes());
    bytes.splice(88..88, [0u8; 32]);
    assert_eq!(Header::parse(&bytes).unwrap().micrometers, None);
}

#[test]
fn empty_frame_falls_back_to_bounds() {
    let mut bytes = emf(&[]);
    bytes[24..40].copy_from_slice(&[0; 16]);
    let (_, _, w, h) = Header::parse(&bytes).unwrap().frame_hmm().unwrap();
    assert_eq!((w, h), (2500.0, 1250.0));
}

#[test]
fn blank_stub_with_no_frame_renders_as_a_blank_picture() {
    // Inverted frame and the "nothing drawn" bounds, as Office writes for a
    // blank picture: a transparent placeholder, not a decode failure.
    let set = |b: &mut Vec<u8>, o: usize, v: [i32; 4]| {
        for (k, x) in v.iter().enumerate() {
            b[o + k * 4..o + k * 4 + 4].copy_from_slice(&x.to_le_bytes());
        }
    };
    let mut bytes = emf(&[rec(18, &[2])]);
    set(&mut bytes, 8, [0, 0, -1, -1]);
    set(&mut bytes, 24, [0, 0, -25, -25]);
    let out = render(&bytes, &fonts()).expect("blank picture");
    assert_eq!((out.pixmap.width(), out.pixmap.height()), (1, 1));
    assert_eq!(px(&out.pixmap, 0, 0)[3], 0);
    assert!(out.text.is_empty());
}

#[test]
fn renders_a_filled_rectangle_on_transparent_background() {
    let bytes = emf(&[
        rec(39, &[1, 0, 0x0000_00FF, 0]), // red solid brush
        rec(37, &[1]),
        rec(37, &[0x8000_0008u32 as i32]), // NULL_PEN
        rec(43, &[0, 0, 50, 50]),
    ]);
    let pm = render(&bytes, &fonts()).unwrap().pixmap;
    assert_eq!((pm.width(), pm.height()), (2048, 1024));
    assert_eq!(px(&pm, 400, 500), [255, 0, 0, 255]);
    assert_eq!(px(&pm, 1500, 500)[3], 0);
}

#[test]
fn rectangle_far_left_of_the_frame_fills_its_visible_part() {
    // Its left edge lands 1.6e8 device pixels off the canvas, past the range
    // tiny-skia can anti-alias (found by the `metafile` fuzz target).
    let bytes = emf(&[
        rec(39, &[1, 0, 0x0000_00FF, 0]), // red solid brush
        rec(37, &[1]),
        rec(43, &[-8_000_000, 10, 60, 40]),
    ]);
    let pm = render(&bytes, &fonts()).unwrap().pixmap;
    assert_eq!(px(&pm, 400, 500), [255, 0, 0, 255]);
    assert_eq!(px(&pm, 1500, 500)[3], 0);
}

#[test]
fn path_bracket_fill_and_clip_rect() {
    let bytes = emf(&[
        rec(39, &[1, 0, 0x0000_FF00, 0]),
        rec(37, &[1]),
        rec(37, &[0x8000_0008u32 as i32]),
        rec(30, &[0, 0, 25, 50]), // clip to the left quarter
        rec(59, &[]),
        rec(27, &[0, 0]),
        rec(54, &[100, 0]),
        rec(54, &[100, 50]),
        rec(54, &[0, 50]),
        rec(61, &[]),
        rec(60, &[]),
        rec(62, &[0, 0, 100, 50]),
    ]);
    let pm = render(&bytes, &fonts()).unwrap().pixmap;
    assert_eq!(px(&pm, 100, 500), [0, 255, 0, 255]);
    assert_eq!(px(&pm, 1000, 500)[3], 0);
}

#[test]
fn save_and_restore_dc_restore_the_clip() {
    let bytes = emf(&[
        rec(39, &[1, 0, 0x00FF_0000, 0]),
        rec(37, &[1]),
        rec(33, &[]),
        rec(30, &[0, 0, 1, 1]),
        rec(34, &[-1]),
        rec(43, &[0, 0, 100, 50]),
    ]);
    let pm = render(&bytes, &fonts()).unwrap().pixmap;
    assert_eq!(px(&pm, 1000, 500), [0, 0, 255, 255]);
}

#[test]
fn stretch_dibits_draws_the_bitmap() {
    // 1x1 24-bit DIB, green, stretched over the left half.
    let mut r = rec(
        81,
        &[
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            1,
            1,
            80,
            40,
            120,
            4,
            0,
            0x00CC_0020,
            50,
            50,
        ],
    );
    r.extend_from_slice(&40u32.to_le_bytes());
    for v in [1i32, 1] {
        r.extend_from_slice(&v.to_le_bytes());
    }
    r.extend_from_slice(&1u16.to_le_bytes());
    r.extend_from_slice(&24u16.to_le_bytes());
    r.extend_from_slice(&[0; 24]);
    r.extend_from_slice(&[0, 255, 0, 0]);
    let size = r.len() as u32;
    r[4..8].copy_from_slice(&size.to_le_bytes());
    let pm = render(&emf(&[r]), &fonts()).unwrap().pixmap;
    assert_eq!(px(&pm, 500, 500), [0, 255, 0, 255]);
    assert_eq!(px(&pm, 1500, 500)[3], 0);
}

#[test]
fn malformed_records_stop_cleanly() {
    let mut bytes = emf(&[rec(43, &[0, 0, 10, 10])]);
    // A record claiming to run past the end of the data.
    bytes.extend_from_slice(&rec(43, &[0, 0, 10]));
    let n = bytes.len();
    bytes[n - 12..n - 8].copy_from_slice(&0x7FFF_FFF0u32.to_le_bytes());
    assert!(render(&bytes, &fonts()).is_some());
    // Records truncated in the middle of their parameters are skipped.
    let short = emf(&[
        rec(43, &[0, 0]),
        rec(3, &[0, 0, 0, 0, 1_000_000]),
        rec(8, &[0, 0, 0, 0, 3, 9]),
    ]);
    assert!(render(&short, &fonts()).is_some());
    // Pseudo-random garbage after a valid header.
    let mut seed = 12345u32;
    let mut junk = emf(&[]);
    junk.truncate(88);
    for _ in 0..20_000 {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        junk.push((seed >> 16) as u8);
    }
    drop(render(&junk, &fonts()));
}

#[test]
fn text_runs_are_reported_in_frame_coordinates() {
    let f = FontSystem::new(&[], true, &Default::default());
    if f.resolve("Arial", false, false).is_none() {
        return; // no fonts on this machine
    }
    // EXTCREATEFONTINDIRECTW: ih 1, height -10, face "Arial".
    let mut font = rec(82, &[1, -10, 0, 0, 0, 700, 0, 0, 0]);
    font.truncate(8 + 4 * 6 + 4 + 4); // through lfWeight, then the byte fields
    font.extend_from_slice(&[0; 4]);
    let mut face = [0u8; 64];
    for (i, c) in "Arial".encode_utf16().enumerate() {
        face[i * 2..i * 2 + 2].copy_from_slice(&c.to_le_bytes());
    }
    font.extend_from_slice(&face);
    let size = font.len() as u32;
    font[4..8].copy_from_slice(&size.to_le_bytes());
    // EXTTEXTOUTW "Hi" at (10, 20), baseline-aligned.
    let mut text = rec(84, &[0, 0, 0, 0, 1, 0, 0, 10, 20, 2, 76, 0, 0, 0, 0, 0, 0]);
    text.extend_from_slice(&[b'H', 0, b'i', 0]);
    let size = text.len() as u32;
    text[4..8].copy_from_slice(&size.to_le_bytes());
    let out =
        super::super::render_with_text(&emf(&[font, rec(37, &[1]), rec(22, &[24]), text]), &f)
            .unwrap();
    assert_eq!(out.text.len(), 1);
    let t = &out.text[0];
    assert_eq!(t.text, "Hi");
    assert!(t.bold);
    assert!((t.x - 0.1).abs() < 0.01, "{}", t.x);
    // Baseline at y = 20 of 50: the box ends a little below it.
    assert!(t.y < 0.4 && t.y + t.h > 0.4, "{} {}", t.y, t.h);
    assert!((t.size - 0.2).abs() < 0.01, "{}", t.size);
    assert_eq!(t.angle, 0.0);
}

#[test]
fn gradient_triangles_are_shaded_and_seamless() {
    // A 100 x 50 rectangle as two GRADIENTFILL triangles, black left to white right.
    let mut r = Vec::new();
    let i = |v: &mut Vec<u8>, x: i32| v.extend_from_slice(&x.to_le_bytes());
    for v in [0, 0, -1, -1, 4, 2, 2] {
        i(&mut r, v); // bounds, nVer, nTri, GRADIENT_FILL_TRIANGLE
    }
    for (x, y, c) in [
        (0, 0, 0u16),
        (100, 0, 0xFF00),
        (0, 50, 0),
        (100, 50, 0xFF00),
    ] {
        i(&mut r, x);
        i(&mut r, y);
        for v in [c, c, c, 0] {
            r.extend_from_slice(&v.to_le_bytes());
        }
    }
    for v in [0, 1, 2, 1, 3, 2] {
        i(&mut r, v);
    }
    let mut record = 118u32.to_le_bytes().to_vec();
    record.extend_from_slice(&(8 + r.len() as u32).to_le_bytes());
    record.extend_from_slice(&r);
    let pm = render(&emf(&[record]), &fonts()).unwrap().pixmap;
    // Every pixel is opaque, including along the shared diagonal.
    assert!(pm.pixels().iter().all(|p| p.alpha() == 255));
    let (left, mid, right) = (px(&pm, 10, 500), px(&pm, 1024, 500), px(&pm, 2037, 500));
    assert!(left[0] < 10 && right[0] > 245, "{left:?} {right:?}");
    assert!((mid[0] as i32 - 128).abs() < 6, "{mid:?}");
    assert_eq!(px(&pm, 1024, 10)[0], px(&pm, 1024, 1000)[0]);
}
