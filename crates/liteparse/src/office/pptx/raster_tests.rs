//! Tests for the raster scanners. `reference` freezes the scanners as they were before the
//! speed rewrite; the semantic tests pin their quirks by hand, and the differential tests
//! (generated images, and real page renders behind `#[ignore]`) require the shipped
//! scanners to return exactly what the reference returns.

use super::*;

/// The original `is_solid_fill` / `find_solid_rects`, copied verbatim. Do not edit: this
/// is the behaviour the optimised scanners must reproduce byte for byte.
mod reference {
    use super::super::{BPP, FIND_RECTS_MIN_DIMENSION_DENOM, SolidRect};

    /// One BGR pixel's colour packed in memory order: `(B << 16) | (G << 8) | R`.
    #[inline]
    fn pixel_color(buf: &[u8], offset: usize) -> u32 {
        (u32::from(buf[offset]) << 16)
            | (u32::from(buf[offset + 1]) << 8)
            | u32::from(buf[offset + 2])
    }

    /// Every pixel's colour bytes equal the first pixel's.
    pub fn is_solid_fill(buf: &[u8], width: usize, height: usize, stride: usize) -> bool {
        if width == 0 || height == 0 || stride < width * BPP || buf.len() < stride * height {
            return false;
        }
        let first = pixel_color(buf, 0);
        for y in 0..height {
            let row = y * stride;
            for x in 0..width {
                if pixel_color(buf, row + x * BPP) != first {
                    return false;
                }
            }
        }
        true
    }

    /// The solid rectangles and/or lines in the raster; white regions are never reported.
    pub fn find_solid_rects(
        buf: &[u8],
        width: usize,
        height: usize,
        stride: usize,
        rectangles: bool,
        lines: bool,
    ) -> Vec<SolidRect> {
        let mut found = Vec::new();
        if (!rectangles && !lines) || width == 0 || height == 0 || stride < width * BPP {
            return found;
        }
        let mut visited = vec![false; width * height];
        let min_dimension = (height / FIND_RECTS_MIN_DIMENSION_DENOM).max(2);
        let at = |x: usize, y: usize| pixel_color(buf, y * stride + x * BPP);
        for y in 0..height {
            for x in 0..width {
                if visited[y * width + x] {
                    continue;
                }
                let color = at(x, y);
                visited[y * width + x] = true;
                if color & 0xffffff == 0xffffff {
                    continue;
                }
                // Probe one pixel `min_dimension` away along each axis before flooding, so
                // isolated specks never start the (expensive) region walk.
                let mut matches = 0;
                if x + min_dimension - 1 < width && at(x + min_dimension - 1, y) == color {
                    matches += 1;
                }
                if y + min_dimension - 1 < height && at(x, y + min_dimension - 1) == color {
                    matches += 1;
                }
                if matches < 1 {
                    continue;
                }
                // Walk rows downward while the left column keeps the colour; the region's
                // width is the narrowest same-colour run among its rows.
                let mut rect_width: Option<usize> = None;
                let mut yy = y;
                while yy < height {
                    if yy != y && (visited[yy * width + x] || at(x, yy) != color) {
                        break;
                    }
                    let mut xx = x + 1;
                    while xx < width && !visited[yy * width + xx] && at(xx, yy) == color {
                        xx += 1;
                    }
                    let row_width = xx - x;
                    rect_width = Some(rect_width.map_or(row_width, |w| w.min(row_width)));
                    yy += 1;
                }
                let rect_height = yy - y;
                let rect_width = rect_width.unwrap_or(0);
                for vy in y..y + rect_height {
                    let row = vy * width;
                    visited[row + x..row + x + rect_width].fill(true);
                }
                let keep = if matches == 1 && lines {
                    rect_height >= min_dimension || rect_width >= min_dimension
                } else if matches > 1 && rectangles {
                    rect_height >= min_dimension && rect_width >= min_dimension
                } else {
                    false
                };
                if keep {
                    found.push(SolidRect {
                        left: x,
                        top: y,
                        width: rect_width,
                        height: rect_height,
                        color,
                    });
                }
            }
        }
        found
    }
}

// ---------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------

/// Colours in these tests are written in the scanners' packing, `(B << 16) | (G << 8) | R`,
/// so a reported `color` equals the value that was painted.
const W: u32 = 0xffffff;
const K: u32 = 0x000000;
const G: u32 = 0x808080;
const RED: u32 = 0x0000ff;

/// A BGR raster with an explicit row stride. Padding bytes are left as `pad`.
struct Img {
    w: usize,
    h: usize,
    stride: usize,
    buf: Vec<u8>,
}

impl Img {
    fn new(w: usize, h: usize, fill: u32) -> Img {
        Img::padded(w, h, fill, 0, 0)
    }

    fn padded(w: usize, h: usize, fill: u32, pad: usize, pad_byte: u8) -> Img {
        let stride = w * BPP + pad;
        let mut img = Img {
            w,
            h,
            stride,
            buf: vec![pad_byte; stride * h],
        };
        img.rect(0, 0, w, h, fill);
        img
    }

    fn set(&mut self, x: usize, y: usize, c: u32) {
        if x < self.w && y < self.h {
            let o = y * self.stride + x * BPP;
            self.buf[o] = (c >> 16) as u8;
            self.buf[o + 1] = (c >> 8) as u8;
            self.buf[o + 2] = c as u8;
        }
    }

    fn get(&self, x: usize, y: usize) -> u32 {
        let o = y * self.stride + x * BPP;
        (u32::from(self.buf[o]) << 16)
            | (u32::from(self.buf[o + 1]) << 8)
            | u32::from(self.buf[o + 2])
    }

    /// Fills `[x, x+w) × [y, y+h)`, clipped to the raster.
    fn rect(&mut self, x: usize, y: usize, w: usize, h: usize, c: u32) {
        for yy in y..(y + h).min(self.h) {
            for xx in x..(x + w).min(self.w) {
                self.set(xx, yy, c);
            }
        }
    }

    /// The same pixels re-laid with `pad` extra bytes per row holding `pad_byte`.
    fn repadded(&self, pad: usize, pad_byte: u8) -> Img {
        let mut out = Img::padded(self.w, self.h, W, pad, pad_byte);
        for y in 0..self.h {
            for x in 0..self.w {
                out.set(x, y, self.get(x, y));
            }
        }
        out
    }
}

type Tuple = (usize, usize, usize, usize, u32);

fn tuples(rects: &[SolidRect]) -> Vec<Tuple> {
    rects
        .iter()
        .map(|r| (r.left, r.top, r.width, r.height, r.color))
        .collect()
}

const FLAGS: [(bool, bool); 4] = [(true, true), (true, false), (false, true), (false, false)];

/// Runs both scanners under every flag combination, asserts they agree, and returns the
/// shipped scanner's `(rectangles, lines)` result.
fn scan(img: &Img, rectangles: bool, lines: bool) -> Vec<Tuple> {
    for (r, l) in FLAGS {
        let new = find_solid_rects(&img.buf, img.w, img.h, img.stride, r, l);
        let old = reference::find_solid_rects(&img.buf, img.w, img.h, img.stride, r, l);
        assert_eq!(
            tuples(&new),
            tuples(&old),
            "find_solid_rects diverges from the reference ({}x{} stride {}, flags {r}/{l})",
            img.w,
            img.h,
            img.stride
        );
    }
    let solid_new = is_solid_fill(&img.buf, img.w, img.h, img.stride);
    let solid_old = reference::is_solid_fill(&img.buf, img.w, img.h, img.stride);
    assert_eq!(
        solid_new, solid_old,
        "is_solid_fill diverges from the reference"
    );
    tuples(&find_solid_rects(
        &img.buf, img.w, img.h, img.stride, rectangles, lines,
    ))
}

/// Invariants every result must satisfy: non-white, solid, disjoint, in raster order.
fn assert_well_formed(img: &Img, rects: &[Tuple]) {
    let mut owner = vec![usize::MAX; img.w * img.h];
    for (i, &(x, y, w, h, c)) in rects.iter().enumerate() {
        assert_ne!(c, W, "white reported");
        assert!(w >= 1 && h >= 1 && x + w <= img.w && y + h <= img.h);
        for yy in y..y + h {
            for xx in x..x + w {
                assert_eq!(img.get(xx, yy), c, "rect {i} is not solid");
                assert_eq!(owner[yy * img.w + xx], usize::MAX, "rects overlap");
                owner[yy * img.w + xx] = i;
            }
        }
    }
    for pair in rects.windows(2) {
        assert!(
            (pair[0].1, pair[0].0) < (pair[1].1, pair[1].0),
            "not in scan order"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Semantic tests
// ---------------------------------------------------------------------------------------

#[test]
fn white_is_never_reported() {
    let img = Img::new(16, 12, W);
    assert!(scan(&img, true, true).is_empty());
    // A white hole inside a grey page: the hole is skipped, the grey is tiled around it.
    let mut img = Img::new(6, 4, G);
    img.rect(2, 1, 2, 2, W);
    let rects = scan(&img, true, true);
    assert!(rects.iter().all(|r| r.4 == G));
    assert_well_formed(&img, &rects);
    assert_eq!(rects.iter().map(|r| r.2 * r.3).sum::<usize>(), 24 - 4);
}

#[test]
fn near_white_differing_in_one_channel_is_not_white() {
    for c in [0xfffffe, 0xfffeff, 0xfeffff] {
        let mut img = Img::new(8, 8, W);
        img.rect(2, 2, 4, 4, c);
        assert_eq!(scan(&img, true, false), vec![(2, 2, 4, 4, c)]);
    }
}

#[test]
fn colours_differing_in_one_channel_are_separate_regions() {
    // Two 3×4 blocks side by side, differing only in the red byte, then only in blue.
    for (a, b) in [(0x000000, 0x000001), (0x102030, 0x112030)] {
        let mut img = Img::new(8, 6, W);
        img.rect(1, 1, 3, 4, a);
        img.rect(4, 1, 3, 4, b);
        assert_eq!(
            scan(&img, true, false),
            vec![(1, 1, 3, 4, a), (4, 1, 3, 4, b)]
        );
    }
}

#[test]
fn colour_is_packed_in_memory_order() {
    // Bytes B=0x11, G=0x22, R=0x33 pack as 0x112233.
    let mut img = Img::new(4, 4, W);
    for y in 0..2 {
        for x in 0..2 {
            let o = y * img.stride + x * BPP;
            img.buf[o..o + 3].copy_from_slice(&[0x11, 0x22, 0x33]);
        }
    }
    assert_eq!(scan(&img, true, false), vec![(0, 0, 2, 2, 0x112233)]);
}

#[test]
fn block_is_a_rect_and_only_reported_with_rectangles() {
    let mut img = Img::new(8, 8, W);
    img.rect(2, 2, 4, 4, RED);
    assert_eq!(scan(&img, true, false), vec![(2, 2, 4, 4, RED)]);
    assert_eq!(scan(&img, true, true), vec![(2, 2, 4, 4, RED)]);
    // Both probes match, so it is never a "line", even with lines on.
    assert!(scan(&img, false, true).is_empty());
}

#[test]
fn thin_bars_are_lines_and_only_reported_with_lines() {
    // Vertical 1-px line: the right probe fails, the down probe matches.
    let mut img = Img::new(8, 8, W);
    img.rect(3, 1, 1, 6, K);
    assert!(scan(&img, true, false).is_empty());
    assert_eq!(scan(&img, false, true), vec![(3, 1, 1, 6, K)]);
    // Horizontal 1-px line.
    let mut img = Img::new(8, 8, W);
    img.rect(1, 4, 6, 1, K);
    assert!(scan(&img, true, false).is_empty());
    assert_eq!(scan(&img, true, true), vec![(1, 4, 6, 1, K)]);
}

#[test]
fn specks_never_start_a_walk() {
    let mut img = Img::new(8, 8, W);
    img.set(3, 3, K);
    img.set(7, 7, K);
    img.set(0, 0, RED);
    assert!(scan(&img, true, true).is_empty());
    // A 2×1 pair is a line of width 2 (= min_dimension on a short page).
    img.set(4, 3, K);
    assert_eq!(scan(&img, true, true), vec![(3, 3, 2, 1, K)]);
}

#[test]
fn min_dimension_scales_with_height() {
    // 599 rows -> min_dimension 2; 600 rows -> 3; 799 -> 3; 800 -> 4.
    for (h, md) in [(399, 2), (400, 2), (599, 2), (600, 3), (799, 3), (800, 4)] {
        let mut img = Img::new(10, h, W);
        // A square exactly at the threshold, and one a pixel below it.
        img.rect(1, 10, md, md, K);
        img.rect(6, 10, md - 1, md - 1, RED);
        // Below the threshold both probes land past the block, so it is never walked.
        assert_eq!(
            scan(&img, true, false),
            vec![(1, 10, md, md, K)],
            "height {h}"
        );
    }
}

#[test]
fn line_needs_one_axis_at_min_dimension() {
    // Page height 800 -> min_dimension 4. A 1×4 vertical bar is a line, 1×3 is not even
    // probed into (the down probe lands one past it).
    let mut img = Img::new(12, 800, W);
    img.rect(1, 20, 1, 4, K);
    img.rect(8, 20, 1, 3, K);
    // A 4×1 bar and a 3×1 bar.
    img.rect(6, 40, 4, 1, RED);
    img.rect(6, 60, 3, 1, RED);
    assert_eq!(
        scan(&img, false, true),
        vec![(1, 20, 1, 4, K), (6, 40, 4, 1, RED)]
    );
}

#[test]
fn probe_sees_through_a_gap_but_the_walk_does_not() {
    // min_dimension 3 (600 rows): the probes at x+2 and y+2 match through a one-pixel
    // gap, so the walk runs, but the run stops at the gap: width 1, not kept.
    let mut img = Img::new(6, 600, W);
    img.set(1, 1, K);
    img.set(3, 1, K);
    img.set(1, 3, K);
    img.set(2, 1, RED);
    assert!(scan(&img, true, true).is_empty());
    // With the gap filled (and the pixel below cleared) the right probe finds a 3×1 line.
    img.set(2, 1, K);
    img.set(1, 3, W);
    assert_eq!(scan(&img, true, true), vec![(1, 1, 3, 1, K)]);
}

#[test]
fn stride_padding_is_ignored_whatever_it_holds() {
    let mut img = Img::new(9, 7, W);
    img.rect(1, 1, 4, 3, K);
    img.rect(6, 0, 1, 7, RED);
    img.rect(0, 5, 9, 2, G);
    let tight = scan(&img, true, true);
    assert!(!tight.is_empty());
    for pad in [1, 2, 3, 5, 8, 13] {
        for pad_byte in [0x00, 0xff, 0x80] {
            let p = img.repadded(pad, pad_byte);
            assert_eq!(scan(&p, true, true), tight, "pad {pad} byte {pad_byte:#x}");
        }
    }
    // Padding that continues the colour of the last column must not extend a run.
    let mut img = Img::new(4, 4, W);
    img.rect(2, 0, 2, 4, K);
    let p = img.repadded(6, 0x00);
    assert_eq!(scan(&p, true, true), vec![(2, 0, 2, 4, K)]);
}

#[test]
fn last_row_may_omit_its_padding() {
    let mut img = Img::new(5, 5, W).repadded(4, 0x7f);
    img.rect(0, 0, 3, 5, K);
    let full = scan(&img, true, true);
    img.buf.truncate(4 * img.stride + 5 * BPP);
    assert_eq!(scan(&img, true, true), full);
}

#[test]
#[should_panic]
fn short_buffer_panics() {
    let mut img = Img::new(5, 5, K);
    img.buf.pop();
    find_solid_rects(&img.buf, img.w, img.h, img.stride, true, true);
}

#[test]
#[should_panic]
fn reference_short_buffer_panics() {
    let mut img = Img::new(5, 5, K);
    img.buf.pop();
    reference::find_solid_rects(&img.buf, img.w, img.h, img.stride, true, true);
}

#[test]
fn degenerate_sizes() {
    for (w, h) in [
        (0, 0),
        (0, 5),
        (5, 0),
        (1, 1),
        (1, 2),
        (2, 1),
        (2, 2),
        (1, 9),
        (9, 1),
    ] {
        for fill in [W, K] {
            let img = Img::new(w, h, fill);
            let rects = scan(&img, true, true);
            assert_well_formed(&img, &rects);
        }
    }
    assert!(scan(&Img::new(1, 1, K), true, true).is_empty());
    assert_eq!(scan(&Img::new(2, 2, K), true, false), vec![(0, 0, 2, 2, K)]);
    assert_eq!(scan(&Img::new(1, 9, K), false, true), vec![(0, 0, 1, 9, K)]);
    assert_eq!(scan(&Img::new(9, 1, K), false, true), vec![(0, 0, 9, 1, K)]);
    assert_eq!(scan(&Img::new(2, 1, K), false, true), vec![(0, 0, 2, 1, K)]);
    // Zero dimensions and a too-small stride report nothing (and never read the buffer).
    assert!(find_solid_rects(&[], 0, 0, 0, true, true).is_empty());
    assert!(find_solid_rects(&[0; 12], 2, 2, 5, true, true).is_empty());
    assert!(find_solid_rects(&[0; 12], 2, 2, 6, false, false).is_empty());
}

#[test]
fn non_white_page_is_one_rect() {
    let img = Img::new(10, 10, G);
    assert_eq!(scan(&img, true, false), vec![(0, 0, 10, 10, G)]);
}

#[test]
fn background_is_tiled_around_a_block_in_scan_order() {
    // Grey 6×4 page with a 2×2 black block at (2, 1). Worked by hand:
    // (0,0) walks all 4 rows, narrowest run 2 (rows 1-2) -> rect 2×4.
    // (2,0) right probe grey, down probe black -> line, row 0 only, width 4.
    // (2,1) the black block; (4,1) grey 2×3; (2,3) grey line stopped by the visited (4,3).
    let mut img = Img::new(6, 4, G);
    img.rect(2, 1, 2, 2, K);
    assert_eq!(
        scan(&img, true, true),
        vec![
            (0, 0, 2, 4, G),
            (2, 0, 4, 1, G),
            (2, 1, 2, 2, K),
            (4, 1, 2, 3, G),
            (2, 3, 2, 1, G),
        ]
    );
    assert_eq!(
        scan(&img, true, false),
        vec![(0, 0, 2, 4, G), (2, 1, 2, 2, K), (4, 1, 2, 3, G)]
    );
}

#[test]
fn l_shape_width_is_the_narrowest_row() {
    // Rows 0-1 are 6 wide, rows 2-5 are 2 wide: the first walk takes the 2-wide column
    // the whole way down and leaves (2..6, 0..2) unmarked; it is found again later.
    let mut img = Img::new(8, 8, W);
    img.rect(0, 0, 6, 2, K);
    img.rect(0, 2, 2, 4, K);
    assert_eq!(
        scan(&img, true, true),
        vec![(0, 0, 2, 6, K), (2, 0, 4, 2, K)]
    );
}

#[test]
fn walk_stops_at_visited_pixels_of_the_same_colour() {
    // (3,0) claims a 3×4 block first; the later walk from (0,2) would run 6 wide by colour,
    // but stops at the already visited (3,2).
    let mut img = Img::new(6, 6, W);
    img.rect(3, 0, 3, 4, K);
    img.rect(0, 2, 6, 2, K);
    assert_eq!(
        scan(&img, true, true),
        vec![(3, 0, 3, 4, K), (0, 2, 3, 2, K)]
    );
}

#[test]
fn column_walk_stops_at_a_visited_left_pixel() {
    // A 2-wide column (x 4-5) is claimed first. The walk from (3,0) runs down column 3
    // and its run on rows 0-1 is cut at x 4 by the visited block; at row 2 the left
    // pixel is black and unvisited so the walk continues.
    let mut img = Img::new(8, 8, W);
    img.rect(4, 0, 2, 3, K);
    img.rect(3, 1, 1, 4, K);
    img.rect(3, 1, 5, 1, K);
    let rects = scan(&img, true, true);
    assert_well_formed(&img, &rects);
}

#[test]
fn staircase_rescans_the_unmarked_overflow() {
    // Each row is two pixels narrower than the one above: every walk keeps only the
    // narrowest width, so the overflow of each row is re-walked from its own start.
    let mut img = Img::new(18, 10, W);
    for y in 0..8 {
        img.rect(0, y, 16 - 2 * y, 1, K);
    }
    let want: Vec<Tuple> = (0..8).map(|k| (2 * k, 0, 2, 8 - k, K)).collect();
    assert_eq!(scan(&img, true, true), want);
    // The last step is one row tall: a line, dropped without `lines`.
    assert_eq!(scan(&img, true, false), want[..7]);
    // One pixel per step: every walk is 1 wide, too thin for anything.
    let mut img = Img::new(10, 10, W);
    for y in 0..8 {
        img.rect(0, y, 8 - y, 1, K);
    }
    assert!(scan(&img, true, true).is_empty());
    // Mirrored: rows grow, so each walk's first row is the narrowest.
    let mut img = Img::new(10, 10, W);
    for y in 0..8 {
        img.rect(0, y, y + 1, 1, K);
    }
    let rects = scan(&img, true, true);
    assert_well_formed(&img, &rects);
}

#[test]
fn circles_and_nested_shapes_match_the_reference() {
    let mut img = Img::new(41, 37, G);
    for y in 0..37i64 {
        for x in 0..41i64 {
            let (dx, dy) = (x - 20, y - 18);
            if dx * dx + dy * dy <= 15 * 15 {
                img.set(x as usize, y as usize, K);
            }
            if dx * dx * 4 + dy * dy <= 8 * 8 {
                img.set(x as usize, y as usize, RED);
            }
        }
    }
    img.rect(18, 16, 5, 5, G);
    let rects = scan(&img, true, true);
    assert_well_formed(&img, &rects);
    assert_eq!(rects[0], (0, 0, 5, 37, G));
}

#[test]
fn adjacent_same_colour_regions_merge_by_walk_rules() {
    // Two touching same-colour blocks of different heights form one region; the first
    // walk follows the left column's height.
    let mut img = Img::new(10, 10, W);
    img.rect(1, 1, 3, 6, K);
    img.rect(4, 3, 3, 2, K);
    assert_eq!(
        scan(&img, true, true),
        vec![(1, 1, 3, 6, K), (4, 3, 3, 2, K)]
    );
}

#[test]
fn solid_fill_detection() {
    assert!(is_solid_fill(&Img::new(3, 2, W).buf, 3, 2, 9));
    assert!(is_solid_fill(&Img::new(1, 1, K).buf, 1, 1, 3));
    let mut img = Img::new(5, 4, G);
    assert!(is_solid_fill(&img.buf, 5, 4, 15));
    img.set(4, 3, 0x808081);
    assert!(!is_solid_fill(&img.buf, 5, 4, 15));
    let mut img = Img::new(5, 4, G);
    img.set(1, 0, 0x818080);
    assert!(!is_solid_fill(&img.buf, 5, 4, 15));
    // Garbage padding is ignored, but the buffer must hold every full stride.
    let img = Img::new(5, 4, G).repadded(7, 0x00);
    assert!(is_solid_fill(&img.buf, 5, 4, img.stride));
    assert!(!is_solid_fill(
        &img.buf[..img.buf.len() - 1],
        5,
        4,
        img.stride
    ));
    // Degenerate shapes.
    assert!(!is_solid_fill(&[], 0, 0, 0));
    assert!(!is_solid_fill(&[0; 6], 2, 0, 6));
    assert!(!is_solid_fill(&[0; 6], 0, 2, 0));
    assert!(!is_solid_fill(&[0; 12], 2, 2, 5));
    for (w, h, pad) in [(1, 1, 0), (2, 3, 1), (7, 1, 3), (1, 7, 2), (4, 4, 0)] {
        let base = Img::new(w, h, RED).repadded(pad, 0x33);
        for y in 0..h {
            for x in 0..w {
                for delta in [0x010000, 0x000100, 0x000001] {
                    let mut img = Img::new(w, h, RED).repadded(pad, 0x33);
                    img.set(x, y, RED ^ delta);
                    assert_eq!(
                        is_solid_fill(&img.buf, w, h, img.stride),
                        reference::is_solid_fill(&img.buf, w, h, img.stride)
                    );
                    // A single pixel is always uniform.
                    assert_eq!(is_solid_fill(&img.buf, w, h, img.stride), w * h == 1);
                }
            }
        }
        assert!(is_solid_fill(&base.buf, w, h, base.stride));
    }
}

// ---------------------------------------------------------------------------------------
// Randomised differential test
// ---------------------------------------------------------------------------------------

/// SplitMix64: tiny, deterministic, good enough to drive the generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `lo..=hi`.
    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next() % (hi - lo + 1) as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick(&mut self, palette: &[u32]) -> u32 {
        palette[self.range(0, palette.len() - 1)]
    }
}

/// A small palette so colours repeat: white, near-whites, black, one-channel neighbours.
fn palette(rng: &mut Rng) -> Vec<u32> {
    let n = rng.range(1, 5);
    let mut p = Vec::with_capacity(n);
    for _ in 0..n {
        let c = match rng.range(0, 6) {
            0 => W,
            1 => W ^ (1 << (8 * rng.range(0, 2))),
            2 => K,
            3 if !p.is_empty() => {
                let base: u32 = p[rng.range(0, p.len() - 1)];
                base ^ (1 << (8 * rng.range(0, 2) + rng.range(0, 7)))
            }
            _ => (rng.next() & 0xffffff) as u32,
        };
        p.push(c);
    }
    p
}

fn paint_shape(img: &mut Img, rng: &mut Rng, pal: &[u32]) {
    let (w, h) = (img.w, img.h);
    let x = rng.range(0, w - 1);
    let y = rng.range(0, h - 1);
    let c = rng.pick(pal);
    match rng.range(0, 9) {
        // Filled rectangle.
        0 | 1 => {
            let rw = rng.range(1, w.min(48));
            let rh = rng.range(1, h.min(48));
            img.rect(x, y, rw, rh, c);
        }
        // Horizontal or vertical rule, 1-3 px thick.
        2 => {
            let t = rng.range(1, 3);
            if rng.chance(50) {
                img.rect(x, y, rng.range(1, w), t, c);
            } else {
                img.rect(x, y, t, rng.range(1, h), c);
            }
        }
        // Noise specks.
        3 => {
            for _ in 0..rng.range(1, 20) {
                let (sx, sy) = (rng.range(0, w - 1), rng.range(0, h - 1));
                img.set(sx, sy, rng.pick(pal));
            }
        }
        // Text-like glyph blobs: strokes of one colour with an anti-aliased fringe.
        4 => {
            let fringe = c ^ 0x0a0a0a;
            for g in 0..rng.range(1, 8) {
                let gx = x + g * rng.range(4, 9);
                let gh = rng.range(3, 12);
                for _ in 0..rng.range(1, 4) {
                    let sx = gx + rng.range(0, 4);
                    let sy = y + rng.range(0, gh);
                    let (sw, sh) = (rng.range(1, 3), rng.range(1, gh));
                    img.rect(sx.saturating_sub(1), sy, sw + 2, sh, fringe);
                    img.rect(sx, sy, sw, sh, c);
                }
            }
        }
        // Filled circle or ellipse.
        5 => {
            let rx = rng.range(1, 30) as i64;
            let ry = rng.range(1, 30) as i64;
            for yy in (y as i64 - ry).max(0)..(y as i64 + ry + 1).min(h as i64) {
                for xx in (x as i64 - rx).max(0)..(x as i64 + rx + 1).min(w as i64) {
                    let (dx, dy) = (xx - x as i64, yy - y as i64);
                    if dx * dx * ry * ry + dy * dy * rx * rx <= rx * rx * ry * ry {
                        img.set(xx as usize, yy as usize, c);
                    }
                }
            }
        }
        // Gradient: the colour steps every `k` columns (or rows).
        6 => {
            let k = rng.range(1, 6);
            let (gw, gh) = (rng.range(1, w), rng.range(1, h));
            let horizontal = rng.chance(50);
            for yy in y..(y + gh).min(h) {
                for xx in x..(x + gw).min(w) {
                    let step = if horizontal {
                        (xx - x) / k
                    } else {
                        (yy - y) / k
                    };
                    img.set(xx, yy, (c.wrapping_add(step as u32 * 0x010101)) & 0xffffff);
                }
            }
        }
        // Staircase, growing or shrinking per row.
        7 => {
            let steps = rng.range(1, 20);
            let grow = rng.chance(50);
            let dx = rng.range(1, 3);
            for s in 0..steps {
                let rw = if grow { 1 + s * dx } else { (steps - s) * dx };
                let left = if rng.chance(70) { x } else { x + s % 3 };
                img.rect(left, y + s, rw, 1, c);
            }
        }
        // Patch of palette noise.
        _ => {
            let (pw, ph) = (rng.range(1, 16), rng.range(1, 16));
            for yy in y..(y + ph).min(h) {
                for xx in x..(x + pw).min(w) {
                    img.set(xx, yy, rng.pick(pal));
                }
            }
        }
    }
}

fn random_image(rng: &mut Rng) -> Img {
    let (w, h) = match rng.range(0, 19) {
        0..=9 => (rng.range(1, 64), rng.range(1, 64)),
        10 | 11 => (rng.range(1, 12), rng.range(200, 1300)),
        12 | 13 => (rng.range(100, 400), rng.range(1, 12)),
        14..=16 => (rng.range(64, 160), rng.range(64, 160)),
        17 => (rng.range(16, 40), rng.range(400, 900)),
        _ => (rng.range(0, 3), rng.range(0, 3)),
    };
    let pal = palette(rng);
    let pad = if rng.chance(30) { rng.range(1, 9) } else { 0 };
    let pad_byte = match rng.range(0, 3) {
        0 => 0xff,
        1 => 0x00,
        _ => rng.next() as u8,
    };
    let bg = match rng.range(0, 19) {
        0..=10 => W,
        11..=16 => pal[0],
        _ => u32::MAX,
    };
    let mut img = Img::padded(w, h, if bg == u32::MAX { W } else { bg }, pad, pad_byte);
    if w == 0 || h == 0 {
        return img;
    }
    if bg == u32::MAX {
        for y in 0..h {
            for x in 0..w {
                img.set(x, y, rng.pick(&pal));
            }
        }
    }
    for _ in 0..rng.range(0, 24) {
        paint_shape(&mut img, rng, &pal);
    }
    // Garbage padding: randomise it per row so it can never look uniform.
    if pad > 0 && rng.chance(50) {
        for y in 0..h {
            for b in 0..pad {
                img.buf[y * img.stride + w * BPP + b] = rng.next() as u8;
            }
        }
    }
    // Sometimes drop the last row's padding: the scanners must not need it.
    if rng.chance(20) {
        img.buf.truncate((h - 1) * img.stride + w * BPP);
    }
    img
}

#[test]
/// 1,000 generated images by default; `RECTS_RANDOM_CASES` raises the count for a deeper
/// run (the final rewrite was checked with 200,000).
fn randomized_differential_against_reference() {
    let cases: usize = std::env::var("RECTS_RANDOM_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);
    let mut rng = Rng(0x05ee_d0f5_011d);
    let mut non_empty = 0;
    for _ in 0..cases {
        let img = random_image(&mut rng);
        let rects = scan(&img, true, true);
        if !rects.is_empty() {
            non_empty += 1;
        }
        if img.buf.len() >= img.stride * img.h {
            assert_well_formed(&img, &rects);
        }
    }
    // The generator must actually produce regions, not only empty results.
    assert!(
        non_empty * 2 > cases,
        "only {non_empty} of {cases} images had rects"
    );
}

// ---------------------------------------------------------------------------------------
// Real-page differential (opt-in)
// ---------------------------------------------------------------------------------------

/// Raw BGR page: `b"BGR1"`, then u32 LE width, height, stride, then `stride * height` bytes.
/// `scripts/raw_bgr_pages.py` converts PNG renders into this format.
fn read_raw(path: &std::path::Path) -> Img {
    let data = std::fs::read(path).expect("read raw page");
    assert_eq!(&data[..4], b"BGR1", "{}", path.display());
    let u = |i: usize| u32::from_le_bytes(data[i..i + 4].try_into().unwrap()) as usize;
    let (w, h, stride) = (u(4), u(8), u(12));
    let buf = data[16..].to_vec();
    assert_eq!(buf.len(), stride * h, "{}", path.display());
    Img { w, h, stride, buf }
}

/// Compares the shipped scanners with the reference on every `*.bgr` page under
/// `RECTS_RAW_DIR` (run with `--release --ignored`), and times both. With
/// `RECTS_TIMING_OUT` set, writes `name width height ref_us new_us rects` per page.
#[test]
#[ignore = "needs RECTS_RAW_DIR pointing at raw BGR pages (scripts/raw_bgr_pages.py)"]
fn real_pages_match_reference() {
    use std::time::Instant;
    let dir = std::env::var("RECTS_RAW_DIR").expect("set RECTS_RAW_DIR");
    let limit: usize = std::env::var("RECTS_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("read RECTS_RAW_DIR")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "bgr"))
        .collect();
    paths.sort();
    paths.truncate(limit);
    assert!(!paths.is_empty(), "no .bgr pages in {dir}");
    let mut tsv = String::new();
    let (mut ref_total, mut new_total, mut mismatches) = (0u128, 0u128, 0usize);
    for (i, path) in paths.iter().enumerate() {
        let img = read_raw(path);
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let t = Instant::now();
        let old = reference::find_solid_rects(&img.buf, img.w, img.h, img.stride, true, true);
        let ref_us = t.elapsed().as_micros();
        let t = Instant::now();
        let new = find_solid_rects(&img.buf, img.w, img.h, img.stride, true, true);
        let new_us = t.elapsed().as_micros();
        ref_total += ref_us;
        new_total += new_us;
        let mut same = tuples(&old) == tuples(&new);
        for (r, l) in [(true, false), (false, true)] {
            let a = reference::find_solid_rects(&img.buf, img.w, img.h, img.stride, r, l);
            let b = find_solid_rects(&img.buf, img.w, img.h, img.stride, r, l);
            same &= tuples(&a) == tuples(&b);
        }
        same &= is_solid_fill(&img.buf, img.w, img.h, img.stride)
            == reference::is_solid_fill(&img.buf, img.w, img.h, img.stride);
        // Every 7th page also runs with garbage stride padding.
        if i % 7 == 0 {
            let p = img.repadded(5, 0xa5);
            let a = reference::find_solid_rects(&p.buf, p.w, p.h, p.stride, true, true);
            let b = find_solid_rects(&p.buf, p.w, p.h, p.stride, true, true);
            same &= tuples(&a) == tuples(&b);
        }
        if !same {
            mismatches += 1;
            eprintln!("MISMATCH {name}");
        }
        tsv.push_str(&format!(
            "{name}\t{}\t{}\t{ref_us}\t{new_us}\t{}\n",
            img.w,
            img.h,
            old.len()
        ));
    }
    if let Ok(out) = std::env::var("RECTS_TIMING_OUT") {
        std::fs::write(out, tsv).expect("write timings");
    }
    eprintln!(
        "{} pages, {mismatches} mismatches; reference {:.1} ms, new {:.1} ms",
        paths.len(),
        ref_total as f64 / 1000.0,
        new_total as f64 / 1000.0
    );
    assert_eq!(mismatches, 0);
}
