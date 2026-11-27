use super::*;

/// Placeable WMF with bbox (0,0)-(200,100) at 100 units/inch, then `records`.
fn wmf(records: &[Vec<u8>]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&PLACEABLE_KEY.to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes());
    for x in [0i16, 0, 200, 100] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(&100u16.to_le_bytes());
    v.extend_from_slice(&[0; 6]);
    for w in [1u16, 9, 0x300, 0, 0, 4, 0, 0, 0] {
        v.extend_from_slice(&w.to_le_bytes());
    }
    for r in records {
        v.extend_from_slice(r);
    }
    v.extend_from_slice(&rec(0, &[]));
    v
}

fn rec(f: u16, params: &[i16]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&(3 + params.len() as u32).to_le_bytes());
    v.extend_from_slice(&f.to_le_bytes());
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
fn layout_reads_the_placeable_header() {
    let l = layout(&wmf(&[])).unwrap();
    assert_eq!(l.frame, (0.0, 0.0, 200.0, 100.0));
    assert_eq!(l.inch, Some(100.0));
    assert_eq!(l.records_at, 40);
    assert!(layout(b"nope").is_none());
}

#[test]
fn non_placeable_uses_the_window() {
    let mut bytes = wmf(&[rec(0x020B, &[5, 10]), rec(0x020C, &[50, 100])]);
    bytes.drain(..22);
    let l = layout(&bytes).unwrap();
    assert_eq!(l.frame, (10.0, 5.0, 100.0, 50.0));
}

#[test]
fn rectangle_with_brush_and_window_mapping() {
    let bytes = wmf(&[
        rec(0x020B, &[0, 0]),
        rec(0x020C, &[10, 20]),          // window 20 x 10 units
        rec(0x02FC, &[0, 0x00FF, 0, 0]), // red brush (colorref 0x000000FF)
        rec(0x012D, &[0]),
        rec(0x02FA, &[5, 0, 0, 0, 0]), // null pen
        rec(0x012D, &[1]),
        rec(0x041B, &[10, 10, 0, 0]), // rectangle (0,0)-(10,10): left half
    ]);
    let out = render(&bytes, &fonts()).unwrap();
    assert_eq!(px(&out.pixmap, 500, 500), [255, 0, 0, 255]);
    assert_eq!(px(&out.pixmap, 1500, 500)[3], 0);
}

#[test]
fn deleted_slots_are_reused_lowest_first() {
    let bytes = wmf(&[
        rec(0x02FC, &[0, 0x00FF, 0, 0]), // slot 0: red
        rec(0x02FC, &[0, 0, 0x00FF, 0]), // slot 1: blue
        rec(0x01F0, &[0]),               // free slot 0
        rec(0x02FC, &[0, -256, 0, 0]),   // slot 0 again: green (0x0000FF00)
        rec(0x012D, &[0]),
        rec(0x02FA, &[5, 0, 0, 0, 0]),
        rec(0x012D, &[2]),
        rec(0x041B, &[100, 200, 0, 0]),
    ]);
    let out = render(&bytes, &fonts()).unwrap();
    assert_eq!(px(&out.pixmap, 100, 100), [0, 255, 0, 255]);
}

#[test]
fn truncated_records_do_not_panic() {
    let mut bytes = wmf(&[rec(0x0324, &[50, 0, 0])]);
    bytes.extend_from_slice(&[9, 0, 0, 0, 0x38, 0x05, 200, 0]);
    assert!(render(&bytes, &fonts()).is_some());
}
