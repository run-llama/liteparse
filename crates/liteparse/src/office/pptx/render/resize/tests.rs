use super::*;

/// Deterministic noise with smooth areas and hard edges.
fn pattern(w: u32, h: u32, n: usize) -> Vec<u8> {
    let mut s: u32 = 12345;
    (0..w as usize * h as usize * n)
        .map(|i| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let x = (i / n) as u32 % w;
            if x < w / 3 {
                (s >> 16) as u8
            } else if x < 2 * w / 3 {
                (x * 255 / w) as u8
            } else if (i / n).is_multiple_of(7) {
                255
            } else {
                0
            }
        })
        .collect()
}

#[test]
fn matches_image_resize_bit_for_bit() {
    for (w, h, edge) in [
        (300u32, 170u32, 128u32),
        (97, 301, 64),
        (1000, 37, 999),
        (65, 65, 64),
    ] {
        let imgs = [
            DynamicImage::ImageLuma8(ImageBuffer::from_raw(w, h, pattern(w, h, 1)).unwrap()),
            DynamicImage::ImageLumaA8(ImageBuffer::from_raw(w, h, pattern(w, h, 2)).unwrap()),
            DynamicImage::ImageRgb8(ImageBuffer::from_raw(w, h, pattern(w, h, 3)).unwrap()),
            DynamicImage::ImageRgba8(ImageBuffer::from_raw(w, h, pattern(w, h, 4)).unwrap()),
        ];
        for img in imgs {
            let ours = fit_triangle(&img, edge).unwrap();
            let theirs = img.resize(edge, edge, image::imageops::FilterType::Triangle);
            assert_eq!(
                (ours.width(), ours.height()),
                (theirs.width(), theirs.height())
            );
            assert_eq!(ours.as_bytes(), theirs.as_bytes(), "{w}x{h} -> {edge}");
            assert_eq!(ours.color(), theirs.color());
        }
    }
}

#[test]
fn leaves_other_formats_to_image() {
    let img = DynamicImage::ImageRgb16(ImageBuffer::new(10, 10));
    assert!(fit_triangle(&img, 4).is_none());
    let small = DynamicImage::ImageRgb8(ImageBuffer::new(4, 4));
    assert!(fit_triangle(&small, 4).is_none());
}
