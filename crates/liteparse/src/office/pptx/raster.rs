//! Raster helpers shared by the extract producers (`llamaparse-read` for PDFs,
//! `pptx-read` for presentations): screenshot sizing and the scanners that run on
//! rendered BGR pixels. Moved here verbatim so both producers size and scan pages
//! identically; `llamaparse-read/src/screenshot.rs` documents the output contract.

/// Long-edge cap in pixels when the request does not set `maxEdge`.
pub const MAX_OCR_SIZE: i32 = 1600;
/// The cap when the requested DPI is above `DEFAULT_DPI` ("high-res OCR").
pub const MAX_OCR_SIZE_HIGHRES: i32 = 3200;
pub const DEFAULT_DPI: i32 = 200;
pub const PDF_DPI: f32 = 72.0;
/// Minimum rectangle edge is this fraction of the bitmap height (0.5 %).
pub const FIND_RECTS_MIN_DIMENSION_DENOM: usize = 200;

/// Pixel size and effective DPI for a page of `page_width × page_height` points.
pub fn page_pixel_size(
    page_width: f32,
    page_height: f32,
    dpi: i32,
    max_edge: i32,
) -> (i32, i32, i32) {
    let mut dpi = if dpi <= 0 { DEFAULT_DPI } else { dpi };
    let mut max_edge = max_edge;
    if max_edge <= 0 {
        max_edge = if dpi > DEFAULT_DPI {
            MAX_OCR_SIZE_HIGHRES
        } else {
            MAX_OCR_SIZE
        };
    }
    let max_edge_f = max_edge as f32;
    let mut width = (page_width / PDF_DPI * dpi as f32).floor() as i32;
    let mut height = (page_height / PDF_DPI * dpi as f32).floor() as i32;
    if page_width >= page_height {
        if width > max_edge {
            dpi = (max_edge_f / page_width * PDF_DPI).round() as i32;
            width = max_edge;
            height = (max_edge_f * page_height / page_width).round() as i32;
        }
    } else if height > max_edge {
        dpi = (max_edge_f / page_height * PDF_DPI).round() as i32;
        height = max_edge;
        width = (max_edge_f * page_width / page_height).round() as i32;
    }
    (dpi, width, height)
}

/// Bytes per pixel of the BGR render target.
pub const BPP: usize = 3;

/// Every pixel's colour bytes equal the first pixel's.
pub fn is_solid_fill(buf: &[u8], width: usize, height: usize, stride: usize) -> bool {
    if width == 0 || height == 0 || stride < width * BPP || buf.len() < stride * height {
        return false;
    }
    let row_bytes = width * BPP;
    let first = &buf[..row_bytes];
    // Row 0 is uniform iff it equals itself shifted by one pixel (every pixel equals the
    // one before it); then every other row must equal row 0, padding excluded.
    first[BPP..] == first[..row_bytes - BPP]
        && (1..height).all(|y| &buf[y * stride..y * stride + row_bytes] == first)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolidRect {
    pub left: usize,
    pub top: usize,
    pub width: usize,
    pub height: usize,
    /// The pixel's bytes packed in memory order: `(B << 16) | (G << 8) | R`.
    pub color: u32,
}

/// One BGR pixel's bytes.
type Px = [u8; BPP];

#[inline]
fn px(row: &[u8], x: usize) -> Px {
    let o = x * BPP;
    [row[o], row[o + 1], row[o + 2]]
}

#[inline]
fn pack(c: Px) -> u32 {
    (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2])
}

/// Pixels per chunk in the word-wide comparisons: 8 pixels = 24 bytes = three `u64`s.
const CHUNK: usize = 8;
const CHUNK_BYTES: usize = CHUNK * BPP;
/// Bit `3i` of a 24-bit byte mask stands for pixel `i` of a chunk.
const PIXEL_BITS: u32 = 0x0024_9249;

#[inline]
fn word(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

/// Bit `i` set when byte `i` of `a` equals byte `i` of `b`.
#[inline]
fn eq_bytes(a: u64, b: u64) -> u32 {
    const LOW7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    const HIGH: u64 = 0x8080_8080_8080_8080;
    let v = a ^ b;
    // Exact per-byte zero test (no borrow between bytes): the high bit marks a zero byte.
    let zero = !(((v & LOW7) + LOW7) | v) & HIGH;
    // Gather the eight high bits into the low byte, byte `k` -> bit `k`.
    ((zero >> 7).wrapping_mul(0x0102_0408_1020_4080) >> 56) as u32
}

/// Bit `3i` set when pixel `i` of the 8-pixel chunk at byte `a` of `ra` equals pixel `i`
/// of the chunk at byte `b` of `rb` (all three bytes equal).
#[inline]
fn eq_pixels(ra: &[u8], a: usize, rb: &[u8], b: usize) -> u32 {
    let bytes = eq_bytes(word(ra, a), word(rb, b))
        | eq_bytes(word(ra, a + 8), word(rb, b + 8)) << 8
        | eq_bytes(word(ra, a + 16), word(rb, b + 16)) << 16;
    bytes & bytes >> 1 & bytes >> 2 & PIXEL_BITS
}

/// The same, against one colour repeated: `pattern` is that colour's 24 chunk bytes.
#[inline]
fn eq_color(ra: &[u8], a: usize, pattern: &[u8; CHUNK_BYTES]) -> u32 {
    eq_pixels(ra, a, pattern, 0)
}

fn pattern(color: Px) -> [u8; CHUNK_BYTES] {
    let mut p = [0; CHUNK_BYTES];
    for c in p.as_chunks_mut::<BPP>().0 {
        c.copy_from_slice(&color);
    }
    p
}

/// Per row, which pixels' walks can matter (1 = candidate): not white, equal to their
/// right probe (`row[x + off]`, inside the row) or down probe (`probe_row[x]`, when that
/// row exists), and equal to their right neighbour or the pixel just below. A pixel
/// failing the first two tests is skipped by the scan; one failing only the last walks a
/// 1×1 region, never reported (`min_dimension >= 2`), whose claim is on the scan position
/// and so never consulted. Either way skipping it, claimed or not, changes nothing.
/// Built with simple per-pixel passes the compiler vectorises.
struct Candidates<'a> {
    flags: Vec<u8>,
    built: Vec<bool>,
    scratch: [u8; BLOCK],
    row: &'a [u8],
    /// Row `y + min_dimension - 1` (down probes) and row `y + 1`, when they exist.
    probe_row: Option<&'a [u8]>,
    next_row: Option<&'a [u8]>,
    off: usize,
}

/// `out[i] = (a[i] == b[i]) as u8` for pixels, over the shorter of the two.
#[inline]
fn eq_into(out: &mut [u8], a: &[u8], b: &[u8]) {
    for ((o, p), q) in out
        .iter_mut()
        .zip(a.as_chunks::<BPP>().0)
        .zip(b.as_chunks::<BPP>().0)
    {
        *o = u8::from(p[0] == q[0]) & u8::from(p[1] == q[1]) & u8::from(p[2] == q[2]);
    }
}

#[inline]
fn or_eq_into(out: &mut [u8], a: &[u8], b: &[u8]) {
    for ((o, p), q) in out
        .iter_mut()
        .zip(a.as_chunks::<BPP>().0)
        .zip(b.as_chunks::<BPP>().0)
    {
        *o |= u8::from(p[0] == q[0]) & u8::from(p[1] == q[1]) & u8::from(p[2] == q[2]);
    }
}

/// Pixels per lazily built block of candidate flags.
const BLOCK: usize = 64;

impl<'a> Candidates<'a> {
    fn new(width: usize) -> Candidates<'a> {
        Candidates {
            flags: vec![0; width],
            built: vec![false; width.div_ceil(BLOCK)],
            scratch: [0; BLOCK],
            row: &[],
            probe_row: None,
            next_row: None,
            off: 1,
        }
    }

    /// Starts a row; its blocks are built on first use, so claimed spans cost nothing.
    fn start_row(
        &mut self,
        row: &'a [u8],
        probe_row: Option<&'a [u8]>,
        next_row: Option<&'a [u8]>,
        off: usize,
    ) {
        (self.row, self.probe_row, self.next_row, self.off) = (row, probe_row, next_row, off);
        self.built.fill(false);
    }

    fn build(&mut self, block: usize) {
        let row = self.row;
        let width = row.len() / BPP;
        let (s, e) = (block * BLOCK, ((block + 1) * BLOCK).min(width));
        let px_of =
            |r: &'a [u8], from: usize| &r[(from * BPP).min(r.len())..(e * BPP).min(r.len())];
        let pixels = &row[s * BPP..e * BPP];
        let flags = &mut self.flags[s..e];
        let grow = &mut self.scratch[..e - s];
        // Probes: equal to the pixel `off` to the right, or to the one in the probe row.
        flags.fill(0);
        eq_into(flags, pixels, &row[((s + self.off) * BPP).min(row.len())..]);
        if let Some(b) = self.probe_row {
            or_eq_into(flags, pixels, px_of(b, s));
        }
        // Growth: equal to the right neighbour, or to the pixel below.
        grow.fill(0);
        eq_into(grow, pixels, &row[((s + 1) * BPP).min(row.len())..]);
        if let Some(b) = self.next_row {
            or_eq_into(grow, pixels, px_of(b, s));
        }
        for ((f, g), c) in flags
            .iter_mut()
            .zip(&*grow)
            .zip(pixels.as_chunks::<BPP>().0)
        {
            let white = u8::from(c[0] == 0xff) & u8::from(c[1] == 0xff) & u8::from(c[2] == 0xff);
            *f &= g & (white ^ 1);
        }
        self.built[block] = true;
    }

    /// The first candidate in `x..` up to the end of `x`'s block: `Ok(candidate)`, or
    /// `Err(block end)` when the block has none.
    #[inline]
    fn next(&mut self, x: usize) -> Result<usize, usize> {
        let block = x / BLOCK;
        if !self.built[block] {
            self.build(block);
        }
        let end = ((block + 1) * BLOCK).min(self.flags.len());
        let flags = &self.flags[x..end];
        let mut i = 0;
        for chunk in flags.as_chunks::<8>().0 {
            let w = u64::from_le_bytes(*chunk);
            if w != 0 {
                return Ok(x + i + w.trailing_zeros() as usize / 8);
            }
            i += 8;
        }
        while i < flags.len() {
            if flags[i] != 0 {
                return Ok(x + i);
            }
            i += 1;
        }
        Err(end)
    }
}

/// One past the last pixel of the run of `color` starting at `from`, capped at `cap`.
#[inline]
fn run_end(row: &[u8], mut from: usize, cap: usize, color: Px) -> usize {
    if from + CHUNK <= cap {
        let p = pattern(color);
        while from + CHUNK <= cap {
            let same = eq_color(row, from * BPP, &p);
            if same != PIXEL_BITS {
                return from + (!same & PIXEL_BITS).trailing_zeros() as usize / BPP;
            }
            from += CHUNK;
        }
    }
    while from < cap && px(row, from) == color {
        from += 1;
    }
    from
}

/// Pixels claimed by an earlier walk, one bit per pixel, rows padded to whole words.
struct Claimed {
    words: Vec<u64>,
    per_row: usize,
    width: usize,
}

impl Claimed {
    fn new(width: usize, height: usize) -> Claimed {
        let per_row = width.div_ceil(64);
        Claimed {
            words: vec![0; per_row * height],
            per_row,
            width,
        }
    }

    #[inline]
    fn get(&self, x: usize, y: usize) -> bool {
        self.words[y * self.per_row + x / 64] >> (x % 64) & 1 != 0
    }

    /// First unclaimed pixel at or after `x` on row `y`, or `width`.
    fn next_clear(&self, x: usize, y: usize) -> usize {
        let row = &self.words[y * self.per_row..(y + 1) * self.per_row];
        let mut w = x / 64;
        let mut bits = !row[w] & (u64::MAX << (x % 64));
        while bits == 0 {
            w += 1;
            if w == row.len() {
                return self.width;
            }
            bits = !row[w];
        }
        (w * 64 + bits.trailing_zeros() as usize).min(self.width)
    }

    /// First claimed pixel in `from..to` on row `y`, or `to`.
    fn next_set(&self, from: usize, to: usize, y: usize) -> usize {
        if from >= to {
            return to;
        }
        let row = &self.words[y * self.per_row..(y + 1) * self.per_row];
        let mut w = from / 64;
        let mut bits = row[w] & (u64::MAX << (from % 64));
        let last = (to - 1) / 64;
        while bits == 0 {
            if w == last {
                return to;
            }
            w += 1;
            bits = row[w];
        }
        (w * 64 + bits.trailing_zeros() as usize).min(to)
    }

    /// Claims `x0..x1` on every row in `rows`.
    fn fill(&mut self, rows: std::ops::Range<usize>, x0: usize, x1: usize) {
        if x0 >= x1 {
            return;
        }
        let (w0, w1) = (x0 / 64, (x1 - 1) / 64);
        let head = u64::MAX << (x0 % 64);
        let tail = u64::MAX >> (63 - (x1 - 1) % 64);
        for y in rows {
            let row = &mut self.words[y * self.per_row..(y + 1) * self.per_row];
            if w0 == w1 {
                row[w0] |= head & tail;
            } else {
                row[w0] |= head;
                row[w0 + 1..w1].fill(u64::MAX);
                row[w1] |= tail;
            }
        }
    }
}

/// Per row, the last same-colour span measured: pixels `lo..hi` all share one colour, and
/// when `exact` the pixel at `hi` differs (or `hi` is the row end). Walks from successive
/// starts keep re-measuring the same background spans; this makes each pixel of a span
/// scanned once instead of once per start.
struct RunCache {
    spans: Vec<(usize, usize, bool)>,
    width: usize,
}

impl RunCache {
    fn new(width: usize, height: usize) -> RunCache {
        RunCache {
            spans: vec![(0, 0, false); height],
            width,
        }
    }

    /// End (exclusive) of the run of `color` starting at `x` on row `y`, capped at `cap`.
    /// `row[x]` must be `color`, and `x < cap <= width`.
    #[inline]
    fn end(&mut self, row: &[u8], x: usize, cap: usize, y: usize, color: Px) -> usize {
        let (lo, hi, exact) = self.spans[y];
        let mut hi = hi;
        if lo <= x && x < hi {
            if cap <= hi {
                return cap;
            }
            if exact {
                return hi;
            }
        } else {
            self.spans[y].0 = x;
            hi = x + 1;
        }
        hi = run_end(row, hi, cap, color);
        self.spans[y].1 = hi;
        self.spans[y].2 = hi < cap || hi == self.width;
        hi
    }
}

/// The solid rectangles and/or lines in the raster; white regions are never reported.
///
/// Scans pixels in raster order. An unclaimed, non-white pixel at `(x, y)` probes the
/// pixel `min_dimension - 1` to its right and the one `min_dimension - 1` below it (whatever
/// their claimed state); with no match it is skipped. Otherwise it walks rows down from
/// `y` while the column-`x` pixel is unclaimed and the same colour (row `y` itself always
/// counts), measuring on each row the run of unclaimed same-colour pixels from `x`. The
/// region is `rect_width` = the narrowest run by `rect_height` = rows walked, and only
/// that rectangle is claimed: wider rows keep their overflow unclaimed, to be walked again
/// from a later start. Claiming happens whether or not the region is reported; one probe
/// match means a line (one side ≥ `min_dimension`), two a rectangle (both sides).
///
/// Speed without changing that output relies on these invariants:
/// - A pixel the scan only *visits* (white, failed probe, or a 1×1 walk) is never
///   consulted again: walks start later in raster order and look only right of their start
///   on their own row and at rows below. So only walks wider or taller than one pixel need
///   to record their claim, and pixels that cannot start such a walk are skipped in bulk
///   (`Candidates`) without consulting the claimed set.
/// - Claimed pixels are never white (a walk only claims its own non-white colour).
/// - Only the minimum run matters, so each row's run is measured up to the current
///   minimum, through a per-row cache of the last measured span (`RunCache`), then cut at
///   the first claimed pixel; once the minimum is 1 rows only need the column test.
///
/// Panics, as the original did, when `buf` ends before the last row's pixels.
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
    let row_bytes = width * BPP;
    assert!(
        buf.len() >= (height - 1) * stride + row_bytes,
        "BGR buffer shorter than {height} rows of stride {stride}"
    );
    let row = |y: usize| &buf[y * stride..y * stride + row_bytes];
    let min_dimension = (height / FIND_RECTS_MIN_DIMENSION_DENOM).max(2);
    let mut claimed = Claimed::new(width, height);
    let mut runs = RunCache::new(width, height);
    let mut candidates = Candidates::new(width);
    for y in 0..height {
        let line = row(y);
        let below = (y + min_dimension - 1 < height).then(|| row(y + min_dimension - 1));
        let next = (y + 1 < height).then(|| row(y + 1));
        candidates.start_row(line, below, next, min_dimension - 1);
        let mut x = 0;
        while x < width {
            if claimed.get(x, y) {
                x = claimed.next_clear(x, y);
                continue;
            }
            x = match candidates.next(x) {
                Ok(x) => x,
                Err(block_end) => {
                    x = block_end;
                    continue;
                }
            };
            if claimed.get(x, y) {
                x = claimed.next_clear(x, y);
                continue;
            }
            let color = px(line, x);
            let right = x + min_dimension - 1 < width && px(line, x + min_dimension - 1) == color;
            let down = below.is_some_and(|b| px(b, x) == color);
            let matches = usize::from(right) + usize::from(down);
            // Every run is at most `width - x`, so this start leaves the minimum unchanged.
            let mut rect_width = width - x;
            let mut yy = y;
            while yy < height {
                let r = row(yy);
                if yy != y && (claimed.get(x, yy) || px(r, x) != color) {
                    break;
                }
                // Once the minimum is 1 no row can lower it: only the column test remains.
                if rect_width > 1 {
                    let colour_end = runs.end(r, x, x + rect_width, yy, color);
                    rect_width = claimed.next_set(x + 1, colour_end, yy) - x;
                }
                yy += 1;
            }
            let rect_height = yy - y;
            claimed.fill(y..yy, x, x + rect_width);
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
                    color: pack(color),
                });
            }
            x += 1;
        }
    }
    found
}

#[cfg(test)]
#[path = "raster_tests.rs"]
mod tests;
