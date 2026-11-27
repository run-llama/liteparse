use super::*;

/// The whole-page blur this module replaces, kept as the reference.
fn reference_blur(data: &mut [u8], w: usize, h: usize, r: usize) {
    fn pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, horizontal: bool) {
        let (n, lines) = if horizontal { (w, h) } else { (h, w) };
        let idx = |line: usize, i: usize| {
            if horizontal {
                line * w + i
            } else {
                i * w + line
            }
        };
        let win = (2 * r + 1) as u32;
        for line in 0..lines {
            let mut sum: u32 = 0;
            for i in 0..=r.min(n - 1) {
                sum += src[idx(line, i)] as u32;
            }
            for i in 0..n {
                dst[idx(line, i)] = (sum / win) as u8;
                if i + r + 1 < n {
                    sum += src[idx(line, i + r + 1)] as u32;
                }
                if i >= r {
                    sum -= src[idx(line, i - r)] as u32;
                }
            }
        }
    }
    let mut tmp = vec![0u8; w * h];
    for _ in 0..3 {
        pass(data, &mut tmp, w, h, r, true);
        pass(&tmp, data, w, h, r, false);
    }
}

fn mask_with(w: u32, h: u32, ink: &[(usize, usize, u8)]) -> Mask {
    let mut m = Mask::new(w, h).unwrap();
    for (x, y, v) in ink {
        m.data_mut()[y * w as usize + x] = *v;
    }
    m
}

#[test]
fn matches_whole_page_blur() {
    // Ink in the middle, against each edge and in a corner; radii up to the page size.
    let cases: &[&[(usize, usize, u8)]] = &[
        &[(20, 15, 255), (22, 16, 90), (25, 18, 7)],
        &[(0, 0, 255), (39, 29, 200)],
        &[(0, 10, 255), (5, 29, 30)],
        &[(39, 0, 128)],
    ];
    for ink in cases {
        for r in [1usize, 2, 3, 7, 20, 64] {
            let mut fast = mask_with(40, 30, ink);
            let mut slow = fast.data().to_vec();
            let region = box_blur(&mut fast, r, (0, 0, 40, 30)).unwrap();
            reference_blur(&mut slow, 40, 30, r);
            assert_eq!(fast.data(), &slow[..], "r={r} ink={ink:?}");
            // Nothing outside the returned region is inked.
            for (i, v) in fast.data().iter().enumerate() {
                let (x, y) = (i % 40, i / 40);
                let inside = x >= region.0 && x < region.2 && y >= region.1 && y < region.3;
                assert!(inside || *v == 0);
            }
        }
    }
}

#[test]
fn empty_mask_has_no_region() {
    let mut m = Mask::new(8, 8).unwrap();
    assert_eq!(box_blur(&mut m, 2, (0, 0, 8, 8)), None);
    assert_eq!(
        ink_bounds(&[0, 0, 3, 0], 2, (0, 0, 2, 2)),
        Some((0, 1, 1, 2))
    );
    // Only the given window is scanned.
    assert_eq!(ink_bounds(&[0, 0, 3, 0], 2, (1, 0, 2, 2)), None);
}

#[test]
fn reach_grows_and_clamps() {
    assert_eq!(reach((10.2, 5.0, 20.5, 6.0), 100, 100), (8, 3, 23, 8));
    assert_eq!(
        reach((-50.0, -50.0, 500.0, 500.0), 100, 80),
        (0, 0, 100, 80)
    );
    assert_eq!(
        reach((200.0, 200.0, 300.0, 300.0), 100, 80),
        (100, 80, 100, 80)
    );
}

#[test]
fn reciprocal_division_is_exact() {
    for r in 1..=64u32 {
        let win = 2 * r + 1;
        let d = Divide::new(win);
        for sum in 0..=255 * win {
            assert_eq!(d.div(sum) as u32, sum / win, "win={win} sum={sum}");
        }
    }
}
