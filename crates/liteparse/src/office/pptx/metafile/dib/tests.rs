use super::*;

/// BITMAPINFOHEADER for a w x h DIB.
fn info(w: i32, h: i32, bpp: u16, compression: u32, colors: &[[u8; 3]]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&40u32.to_le_bytes());
    v.extend_from_slice(&w.to_le_bytes());
    v.extend_from_slice(&h.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&bpp.to_le_bytes());
    v.extend_from_slice(&compression.to_le_bytes());
    v.extend_from_slice(&[0; 12]);
    v.extend_from_slice(&(colors.len() as u32).to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    for c in colors {
        v.extend_from_slice(&[c[2], c[1], c[0], 0]);
    }
    v
}

fn px(pm: &Pixmap, x: u32, y: u32) -> [u8; 4] {
    let c = pm.pixel(x, y).unwrap();
    [c.red(), c.green(), c.blue(), c.alpha()]
}

#[test]
fn bottom_up_24_bit_rows_are_flipped_and_padded() {
    // 2x2, rows padded to 8 bytes; stored bottom row first.
    let bits = [
        0, 0, 255, 0, 255, 0, 0, 0, /* bottom: red, green */ 255, 0, 0, 255, 255, 255, 0,
        0, /* top: blue, white */
    ];
    let pm = decode(&info(2, 2, 24, BI_RGB, &[]), &bits, false).unwrap();
    assert_eq!(px(&pm, 0, 0), [0, 0, 255, 255]);
    assert_eq!(px(&pm, 1, 0), [255, 255, 255, 255]);
    assert_eq!(px(&pm, 0, 1), [255, 0, 0, 255]);
    assert_eq!(px(&pm, 1, 1), [0, 255, 0, 255]);
}

#[test]
fn top_down_one_bit_uses_the_palette() {
    let bmi = info(3, -1, 1, BI_RGB, &[[10, 20, 30], [200, 100, 50]]);
    let pm = decode(&bmi, &[0b0100_0000, 0, 0, 0], false).unwrap();
    assert_eq!(px(&pm, 0, 0), [10, 20, 30, 255]);
    assert_eq!(px(&pm, 1, 0), [200, 100, 50, 255]);
    assert!(is_monochrome(&bmi));
}

#[test]
fn rle8_runs_and_absolute_mode() {
    let pal = [[0, 0, 0], [255, 0, 0], [0, 0, 255]];
    // Row 0 (bottom): run of 2 x index 1, absolute run [2, 1, 2], end of line;
    // row 1: run of 5 x index 2, end of bitmap.
    let bits = [2, 1, 0, 3, 2, 1, 2, 0, 0, 0, 5, 2, 0, 1];
    let pm = decode(&info(5, 2, 8, BI_RLE8, &pal), &bits, false).unwrap();
    assert_eq!(px(&pm, 0, 1), [255, 0, 0, 255]);
    assert_eq!(px(&pm, 2, 1), [0, 0, 255, 255]);
    assert_eq!(px(&pm, 3, 1), [255, 0, 0, 255]);
    assert_eq!(px(&pm, 4, 0), [0, 0, 255, 255]);
}

#[test]
fn thirty_two_bit_alpha_only_when_asked() {
    let bits = [0, 0, 128, 128]; // B, G, R, A: premultiplied half red
    let opaque = decode(&info(1, 1, 32, BI_RGB, &[]), &bits, false).unwrap();
    assert_eq!(px(&opaque, 0, 0), [128, 0, 0, 255]);
    let blended = decode(&info(1, 1, 32, BI_RGB, &[]), &bits, true).unwrap();
    assert_eq!(px(&blended, 0, 0), [128, 0, 0, 128]);
}

#[test]
fn sixteen_bit_bitfields() {
    let mut bmi = info(1, 1, 16, BI_BITFIELDS, &[]);
    for m in [0xF800u32, 0x07E0, 0x001F] {
        bmi.extend_from_slice(&m.to_le_bytes());
    }
    let pm = decode(&bmi, &0xF800u16.to_le_bytes(), false).unwrap();
    assert_eq!(px(&pm, 0, 0), [255, 0, 0, 255]);
}

#[test]
fn packed_dib_finds_bits_after_the_palette() {
    let mut data = info(1, 1, 8, BI_RGB, &[[0, 0, 0], [9, 8, 7]]);
    data.extend_from_slice(&[1, 0, 0, 0]);
    let pm = decode_packed(&data, false).unwrap();
    assert_eq!(px(&pm, 0, 0), [9, 8, 7, 255]);
}

#[test]
fn truncated_or_bogus_input_never_panics() {
    assert!(decode(&[1, 2, 3], &[], false).is_none());
    assert!(decode(&info(0, 5, 24, BI_RGB, &[]), &[], false).is_none());
    assert!(decode(&info(100_000, 100_000, 24, BI_RGB, &[]), &[], false).is_none());
    // Missing bits leave pixels transparent instead of failing.
    let pm = decode(&info(4, 4, 24, BI_RGB, &[]), &[1, 2, 3], false).unwrap();
    assert_eq!(px(&pm, 0, 0)[3], 0);
    drop(decode(
        &info(4, 4, 4, BI_RLE4, &[]),
        &[0, 2, 255, 255, 0, 0, 7],
        false,
    ));
}
