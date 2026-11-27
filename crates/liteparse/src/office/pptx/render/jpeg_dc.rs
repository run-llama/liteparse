//! JPEG decoding at 1/8 scale from the DC coefficients only, for pictures too
//! large to decode in full (posters embed 180-megapixel photos).
//!
//! Each 8×8 block's DC coefficient is its average sample, so the DC plane is a
//! box-filtered 1/8 downscale. Memory is one value per block. Huffman baseline,
//! extended and progressive files are read; progressive AC scans are skipped.
//! Arithmetic coding, lossless and 12-bit files return an error.

use image::{DynamicImage, RgbImage};

struct Component {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
    /// Blocks per row / column, padded to whole MCUs.
    bw: usize,
    bh: usize,
    /// DC coefficient (dequantization pending) of every block.
    dc: Vec<i32>,
}

#[derive(Clone, Default)]
struct Huffman {
    /// Largest code of each length (`-1` = none), and the index of its first symbol.
    maxcode: [i32; 18],
    valptr: [i32; 17],
    mincode: [i32; 17],
    symbols: Vec<u8>,
}

impl Huffman {
    fn new(counts: &[u8], symbols: &[u8]) -> Huffman {
        let mut t = Huffman {
            maxcode: [-1; 18],
            symbols: symbols.to_vec(),
            ..Default::default()
        };
        let (mut code, mut k) = (0i32, 0i32);
        for len in 1..=16 {
            let n = counts[len - 1] as i32;
            t.valptr[len] = k;
            t.mincode[len] = code;
            code += n;
            k += n;
            t.maxcode[len] = if n > 0 { code - 1 } else { -1 };
            code <<= 1;
        }
        t.maxcode[17] = i32::MAX;
        t
    }
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
    /// A marker was reached: further reads return zero bits.
    at_marker: bool,
}

impl Bits<'_> {
    fn fill(&mut self) {
        while self.n <= 56 {
            let mut byte = 0u8;
            if !self.at_marker && self.pos < self.data.len() {
                byte = self.data[self.pos];
                if byte == 0xFF {
                    match self.data.get(self.pos + 1) {
                        Some(0) => self.pos += 2,
                        _ => {
                            self.at_marker = true;
                            byte = 0;
                        }
                    }
                } else {
                    self.pos += 1;
                }
            }
            self.acc |= (byte as u64) << (56 - self.n);
            self.n += 8;
        }
    }

    fn bits(&mut self, k: u32) -> i32 {
        if k == 0 {
            return 0;
        }
        if self.n < k {
            self.fill();
        }
        let v = (self.acc >> (64 - k)) as i32;
        self.acc <<= k;
        self.n -= k;
        v
    }

    fn decode(&mut self, t: &Huffman) -> Result<u8, String> {
        let mut code = self.bits(1);
        let mut len = 1;
        while code > t.maxcode[len] {
            len += 1;
            if len > 16 {
                return Err("bad Huffman code".into());
            }
            code = (code << 1) | self.bits(1);
        }
        let i = t.valptr[len] + code - t.mincode[len];
        t.symbols
            .get(i as usize)
            .copied()
            .ok_or_else(|| "bad Huffman code".into())
    }

    /// Value of a `s`-bit magnitude category (JPEG `EXTEND`).
    fn receive_extend(&mut self, s: u32) -> i32 {
        if s == 0 {
            return 0;
        }
        let v = self.bits(s);
        if v < 1 << (s - 1) {
            v - (1 << s) + 1
        } else {
            v
        }
    }

    /// Skips to just past the next RSTn marker (restart interval boundary).
    fn restart(&mut self) {
        self.acc = 0;
        self.n = 0;
        self.at_marker = false;
        while self.pos + 1 < self.data.len() {
            if self.data[self.pos] == 0xFF && (0xD0..=0xD7).contains(&self.data[self.pos + 1]) {
                self.pos += 2;
                return;
            }
            self.pos += 1;
        }
    }
}

/// Index just past the entropy-coded segment starting at `pos` (the next
/// marker that is neither stuffing nor a restart).
fn skip_entropy(b: &[u8], mut pos: usize) -> usize {
    while pos + 1 < b.len() {
        if b[pos] == 0xFF && b[pos + 1] != 0 && !(0xD0..=0xD7).contains(&b[pos + 1]) {
            return pos;
        }
        pos += 1;
    }
    b.len()
}

/// Pixel size from the frame header, without decoding.
pub(super) fn size(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    if !b.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            return None;
        }
        let m = b[i + 1];
        if m == 0xFF {
            i += 1;
            continue;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if matches!(m, 0xC0..=0xCF) && !matches!(m, 0xC4 | 0xC8 | 0xCC) && i + 9 <= b.len() {
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

/// Decodes the picture at 1/8 of its size (rounded up) as RGB.
pub(super) fn decode(b: &[u8]) -> Result<DynamicImage, String> {
    let bad = || "truncated JPEG".to_string();
    if !b.starts_with(&[0xFF, 0xD8]) {
        return Err("not a JPEG".into());
    }
    let mut qdc = [1i32; 4];
    let mut dc_tables: [Huffman; 4] = Default::default();
    let mut ac_tables: [Huffman; 4] = Default::default();
    let mut comps: Vec<Component> = Vec::new();
    let (mut width, mut height, mut progressive) = (0usize, 0usize, false);
    let (mut mcux, mut mcuy, mut hmax, mut vmax) = (0usize, 0usize, 1usize, 1usize);
    let mut restart_interval = 0usize;
    let mut adobe_transform: Option<u8> = None;
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            return Err("bad marker".into());
        }
        let m = b[i + 1];
        if m == 0xFF {
            i += 1;
            continue;
        }
        if m == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        let seg = b.get(i + 4..i + 2 + len).ok_or_else(bad)?;
        match m {
            0xDB => {
                let mut p = 0;
                while p < seg.len() {
                    let (pq, tq) = (seg[p] >> 4, (seg[p] & 3) as usize);
                    // The DC quantizer is the first entry in zigzag order.
                    qdc[tq] = if pq == 0 {
                        *seg.get(p + 1).ok_or_else(bad)? as i32
                    } else {
                        u16::from_be_bytes([seg[p + 1], *seg.get(p + 2).ok_or_else(bad)?]) as i32
                    };
                    p += 1 + 64 * (pq as usize + 1);
                }
            }
            0xC4 => {
                let mut p = 0;
                while p + 17 <= seg.len() {
                    let (class, id) = (seg[p] >> 4, (seg[p] & 3) as usize);
                    let counts = &seg[p + 1..p + 17];
                    let n: usize = counts.iter().map(|&c| c as usize).sum();
                    let syms = seg.get(p + 17..p + 17 + n).ok_or_else(bad)?;
                    let t = Huffman::new(counts, syms);
                    if class == 0 {
                        dc_tables[id] = t;
                    } else {
                        ac_tables[id] = t;
                    }
                    p += 17 + n;
                }
            }
            0xDD => restart_interval = u16::from_be_bytes([seg[0], seg[1]]) as usize,
            0xEE if seg.starts_with(b"Adobe") && seg.len() >= 12 => {
                adobe_transform = Some(seg[11]);
            }
            0xC0..=0xC2 => {
                if seg[0] != 8 {
                    return Err(format!("{}-bit JPEG", seg[0]));
                }
                progressive = m == 0xC2;
                height = u16::from_be_bytes([seg[1], seg[2]]) as usize;
                width = u16::from_be_bytes([seg[3], seg[4]]) as usize;
                let n = seg[5] as usize;
                for k in 0..n {
                    let c = seg.get(6 + 3 * k..9 + 3 * k).ok_or_else(bad)?;
                    let (h, v) = ((c[1] >> 4).max(1) as usize, (c[1] & 15).max(1) as usize);
                    comps.push(Component {
                        id: c[0],
                        h,
                        v,
                        tq: (c[2] & 3) as usize,
                        bw: 0,
                        bh: 0,
                        dc: Vec::new(),
                    });
                }
                if width == 0 || height == 0 || !matches!(n, 1 | 3 | 4) {
                    return Err("unsupported JPEG frame".into());
                }
                hmax = comps.iter().map(|c| c.h).max().unwrap_or(1);
                vmax = comps.iter().map(|c| c.v).max().unwrap_or(1);
                mcux = width.div_ceil(8 * hmax);
                mcuy = height.div_ceil(8 * vmax);
                for c in &mut comps {
                    c.bw = mcux * c.h;
                    c.bh = mcuy * c.v;
                    c.dc = vec![0; c.bw * c.bh];
                }
            }
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Err("unsupported JPEG coding".into());
            }
            0xDA => {
                if comps.is_empty() {
                    return Err("scan before frame".into());
                }
                let ns = seg[0] as usize;
                let mut scan = Vec::new();
                for k in 0..ns {
                    let id = *seg.get(1 + 2 * k).ok_or_else(bad)?;
                    let t = seg[2 + 2 * k];
                    let ci = comps.iter().position(|c| c.id == id).ok_or_else(bad)?;
                    scan.push((ci, (t >> 4) as usize & 3, (t & 3) as usize));
                }
                let p = 1 + 2 * ns;
                let (ss, se, ah, al) = (seg[p], seg[p + 1], seg[p + 2] >> 4, seg[p + 2] & 15);
                let start = i + 2 + len;
                let end = skip_entropy(b, start);
                if !progressive || ss == 0 {
                    let mut bits = Bits {
                        data: &b[start..end],
                        pos: 0,
                        acc: 0,
                        n: 0,
                        at_marker: false,
                    };
                    let job = Scan {
                        scan: &scan,
                        dc_tables: &dc_tables,
                        ac_tables: &ac_tables,
                        // A baseline scan carries all AC coefficients after each DC.
                        skip_ac: !progressive && se > 0,
                        refine: progressive && ah != 0,
                        al: if progressive { al as u32 } else { 0 },
                        restart_interval,
                        mcux,
                        mcuy,
                        width,
                        height,
                        hmax,
                        vmax,
                    };
                    job.run(&mut comps, &mut bits)?;
                }
                i = end;
                continue;
            }
            _ => {}
        }
        i += 2 + len;
    }
    if comps.is_empty() {
        return Err("no JPEG frame".into());
    }
    // Plane values per output pixel (one per 8×8 block at full resolution).
    let (ow, oh) = (width.div_ceil(8), height.div_ceil(8));
    let sample = |c: &Component, x: usize, y: usize| -> f32 {
        let bx = (x * c.h / hmax).min(c.bw - 1);
        let by = (y * c.v / vmax).min(c.bh - 1);
        (c.dc[by * c.bw + bx] * qdc[c.tq]) as f32 / 8.0 + 128.0
    };
    let rgb_ids = comps.len() == 3 && comps[0].id == b'R' && comps[1].id == b'G';
    let ycc = match comps.len() {
        3 => adobe_transform != Some(0) && !rgb_ids,
        4 => adobe_transform == Some(2),
        _ => false,
    };
    let mut out = RgbImage::new(ow as u32, oh as u32);
    for y in 0..oh {
        for x in 0..ow {
            let s: Vec<f32> = comps.iter().map(|c| sample(c, x, y)).collect();
            let px = match s.len() {
                1 => [s[0]; 3],
                _ => {
                    let mut rgb = if ycc {
                        let (yy, cb, cr) = (s[0], s[1] - 128.0, s[2] - 128.0);
                        [
                            yy + 1.402 * cr,
                            yy - 0.344_136 * cb - 0.714_136 * cr,
                            yy + 1.772 * cb,
                        ]
                    } else {
                        [s[0], s[1], s[2]]
                    };
                    if s.len() == 4 {
                        // Adobe CMYK/YCCK store inverted inks: the value is already
                        // 255 - ink, so RGB = value × (1 - K ink).
                        let k = s[3].clamp(0.0, 255.0) / 255.0;
                        for v in &mut rgb {
                            *v = v.clamp(0.0, 255.0) * k;
                        }
                    }
                    rgb
                }
            };
            out.put_pixel(
                x as u32,
                y as u32,
                image::Rgb(px.map(|v| v.round().clamp(0.0, 255.0) as u8)),
            );
        }
    }
    Ok(DynamicImage::ImageRgb8(out))
}

struct Scan<'a> {
    /// (component index, DC table, AC table) per scan component.
    scan: &'a [(usize, usize, usize)],
    dc_tables: &'a [Huffman; 4],
    ac_tables: &'a [Huffman; 4],
    skip_ac: bool,
    refine: bool,
    al: u32,
    restart_interval: usize,
    mcux: usize,
    mcuy: usize,
    /// Frame size and largest sampling factors, for non-interleaved scans.
    width: usize,
    height: usize,
    hmax: usize,
    vmax: usize,
}

impl Scan<'_> {
    fn run(&self, comps: &mut [Component], bits: &mut Bits) -> Result<(), String> {
        let mut pred = vec![0i32; self.scan.len()];
        let mut units = 0usize;
        if self.scan.len() == 1 {
            // A single-component scan walks that component's blocks without MCU padding.
            let ci = self.scan[0].0;
            let (h, v, bw) = (comps[ci].h, comps[ci].v, comps[ci].bw);
            let cols = (self.width * h).div_ceil(self.hmax).div_ceil(8);
            let rows = (self.height * v).div_ceil(self.vmax).div_ceil(8);
            for by in 0..rows {
                for bx in 0..cols {
                    self.restart_check(bits, &mut pred, &mut units);
                    self.block(0, &mut comps[ci], bits, &mut pred[0], by * bw + bx)?;
                }
            }
            return Ok(());
        }
        for my in 0..self.mcuy {
            for mx in 0..self.mcux {
                self.restart_check(bits, &mut pred, &mut units);
                for (s, &(ci, _, _)) in self.scan.iter().enumerate() {
                    let c = &mut comps[ci];
                    let (h, v, bw) = (c.h, c.v, c.bw);
                    for yy in 0..v {
                        for xx in 0..h {
                            let idx = (my * v + yy) * bw + mx * h + xx;
                            self.block(s, c, bits, &mut pred[s], idx)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Before each MCU: at a restart boundary, resync on the RSTn marker.
    fn restart_check(&self, bits: &mut Bits, pred: &mut [i32], units: &mut usize) {
        if self.restart_interval > 0 && *units > 0 && (*units).is_multiple_of(self.restart_interval)
        {
            bits.restart();
            pred.fill(0);
        }
        *units += 1;
    }

    fn block(
        &self,
        s: usize,
        c: &mut Component,
        bits: &mut Bits,
        pred: &mut i32,
        idx: usize,
    ) -> Result<(), String> {
        let (_, td, ta) = self.scan[s];
        if self.refine {
            if bits.bits(1) != 0 {
                c.dc[idx] |= 1 << self.al;
            }
            return Ok(());
        }
        let t = bits.decode(&self.dc_tables[td])? as u32;
        *pred += bits.receive_extend(t);
        c.dc[idx] = *pred << self.al;
        if self.skip_ac {
            let mut k = 1;
            while k < 64 {
                let rs = bits.decode(&self.ac_tables[ta])?;
                let (r, sz) = ((rs >> 4) as usize, (rs & 15) as u32);
                if sz == 0 {
                    if r != 15 {
                        break;
                    }
                    k += 16;
                    continue;
                }
                bits.bits(sz);
                k += r + 1;
            }
        }
        Ok(())
    }
}
