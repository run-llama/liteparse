//! Compact Table Format (CTF) → sfnt: the second half of MicroType Express
//! decompression (MTX submission, sections 4-7).
//!
//! The first block is an sfnt whose table directory is intact but whose `glyf`,
//! `cvt `, `hdmx` and `VDMX` tables are re-encoded, and whose `loca` is empty.
//! Glyph instructions are split out: each glyph's initial PUSH values live in the
//! second block and its remaining instructions in the third, both consumed in
//! glyph order. Everything else is copied. The rebuilt font always uses a long
//! (32-bit) `loca` and gets fresh table checksums.

use std::collections::BTreeMap;

/// Big-endian cursor over CTF data.
pub(super) struct Be<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Be<'a> {
    pub(super) fn new(b: &'a [u8]) -> Self {
        Be { b, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|e| *e <= self.b.len())
            .ok_or("CTF data truncated")?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let s = self.take(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }
    fn i16(&mut self) -> Result<i16, String> {
        Ok(self.u16()? as i16)
    }
    fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn peek(&self) -> Result<u8, String> {
        self.b
            .get(self.pos)
            .copied()
            .ok_or_else(|| "CTF data truncated".into())
    }
    fn at_end(&self) -> bool {
        self.pos >= self.b.len()
    }

    /// 255UShort (section 6.1.1): one byte below 253, else a code byte selects
    /// a 16-bit word (253) or 253 + next (255) or 506 + next (254).
    pub(super) fn u255(&mut self) -> Result<u16, String> {
        Ok(match self.u8()? {
            253 => self.u16()?,
            255 => 253 + self.u8()? as u16,
            254 => 506 + self.u8()? as u16,
            c => c as u16,
        })
    }

    /// 255Short (section 6.1.2): like 255UShort with base 250, a 250 prefix that
    /// negates, and 253 for a signed word.
    pub(super) fn i255(&mut self) -> Result<i16, String> {
        let mut code = self.u8()?;
        if code == 253 {
            return self.i16();
        }
        let mut sign = 1i32;
        if code == 250 {
            sign = -1;
            code = self.u8()?;
        }
        let v = match code {
            255 => 250 + self.u8()? as i32,
            254 => 500 + self.u8()? as i32,
            c => c as i32,
        };
        Ok((v * sign) as i16)
    }
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_be_bytes([s[0], s[1]]))
}

/// Rebuilds the sfnt from the three decompressed MTX blocks.
pub fn to_sfnt(rest: &[u8], push: &[u8], code: &[u8]) -> Result<Vec<u8>, String> {
    let mut r = Be::new(rest);
    let sfnt_version = r.u32()?;
    let num_tables = r.u16()? as usize;
    r.take(6)?;
    let mut tables: BTreeMap<[u8; 4], Vec<u8>> = BTreeMap::new();
    for _ in 0..num_tables {
        let tag: [u8; 4] = r.take(4)?.try_into().unwrap_or_default();
        r.u32()?; // checksum: recomputed
        let offset = r.u32()? as usize;
        let length = r.u32()? as usize;
        let data = offset
            .checked_add(length)
            .and_then(|end| rest.get(offset..end))
            .ok_or_else(|| format!("CTF table {} out of bounds", String::from_utf8_lossy(&tag)))?;
        tables.insert(tag, data.to_vec());
    }

    let head = tables.get(b"head").ok_or("CTF font has no head table")?;
    let units_per_em = be16(head, 18).filter(|u| *u > 0).unwrap_or(1000);
    let num_glyphs = tables
        .get(b"maxp")
        .and_then(|m| be16(m, 4))
        .ok_or("CTF font has no maxp table")? as usize;
    let advances = advance_widths(&tables, num_glyphs);

    if let Some(ctf_glyf) = tables.get(b"glyf") {
        let (glyf, loca) = glyf_to_ttf(ctf_glyf, push, code, num_glyphs)?;
        tables.insert(*b"glyf", glyf);
        tables.insert(*b"loca", loca);
        if let Some(head) = tables.get_mut(b"head") {
            // indexToLocFormat = 1: the rebuilt loca has 32-bit offsets.
            if head.len() >= 52 {
                head[50..52].copy_from_slice(&1u16.to_be_bytes());
            }
        }
    } else if tables.get(b"loca").is_some_and(|l| l.is_empty()) {
        tables.remove(b"loca");
    }
    if let Some(cvt) = tables.get(b"cvt ") {
        let decoded = cvt_to_ttf(cvt)?;
        tables.insert(*b"cvt ", decoded);
    }
    // hdmx and VDMX only hold hinted pixel metrics, which this renderer never uses;
    // a table that does not decode is dropped rather than failing the font.
    if let Some(hdmx) = tables.get(b"hdmx") {
        match hdmx_to_ttf(hdmx, &advances, units_per_em) {
            Some(t) => tables.insert(*b"hdmx", t),
            None => tables.remove(b"hdmx"),
        };
    }
    if let Some(vdmx) = tables.get(b"VDMX") {
        match vdmx_to_ttf(vdmx) {
            Some(t) => tables.insert(*b"VDMX", t),
            None => tables.remove(b"VDMX"),
        };
    }
    Ok(write_sfnt(sfnt_version, &tables))
}

/// Advance width per glyph from `hhea`/`hmtx` (the last long metric repeats).
fn advance_widths(tables: &BTreeMap<[u8; 4], Vec<u8>>, num_glyphs: usize) -> Vec<u16> {
    let n_metrics = tables.get(b"hhea").and_then(|h| be16(h, 34)).unwrap_or(0) as usize;
    let hmtx = tables.get(b"hmtx").map(Vec::as_slice).unwrap_or(&[]);
    let mut out = Vec::with_capacity(num_glyphs);
    let mut last = 0;
    for g in 0..num_glyphs {
        if g < n_metrics {
            last = be16(hmtx, 4 * g).unwrap_or(last);
        }
        out.push(last);
    }
    out
}

/// One row of the triplet table (section 5.11), indexed by the low 7 flag bits.
#[derive(Clone, Copy)]
struct Triplet {
    bytes: u8,
    x_bits: u8,
    y_bits: u8,
    dx: u16,
    dy: u16,
    x_neg: bool,
    y_neg: bool,
}

/// Expands the 128-entry triplet table. Rows come in groups that share a byte
/// count, bit widths and one delta, cycling through the other delta and the
/// signs; the table in the spec lists them in exactly this order.
fn triplet(index: u8) -> Triplet {
    let i = index as u16;
    let t = |bytes, x_bits, y_bits, dx, dy, x_neg, y_neg| Triplet {
        bytes,
        x_bits,
        y_bits,
        dx,
        dy,
        x_neg,
        y_neg,
    };
    match i {
        // dx = 0; dy in 8 bits plus 0/256/512/768/1024; sign alternates -, +.
        0..=9 => t(2, 0, 8, 0, (i / 2) * 256, false, i.is_multiple_of(2)),
        10..=19 => t(2, 8, 0, ((i - 10) / 2) * 256, 0, i.is_multiple_of(2), false),
        // 4+4 bits; dx base by group of 16, dy base by group of 4, signs (-,-),(+,-),(-,+),(+,+).
        20..=83 => {
            let k = i - 20;
            t(
                2,
                4,
                4,
                1 + (k / 16) * 16,
                1 + ((k % 16) / 4) * 16,
                k.is_multiple_of(2),
                (k % 4) < 2,
            )
        }
        84..=119 => {
            let k = i - 84;
            t(
                3,
                8,
                8,
                1 + (k / 12) * 256,
                1 + ((k % 12) / 4) * 256,
                k.is_multiple_of(2),
                (k % 4) < 2,
            )
        }
        120..=123 => {
            let k = i - 120;
            t(4, 12, 12, 0, 0, k.is_multiple_of(2), k < 2)
        }
        _ => {
            let k = i - 124;
            t(5, 16, 16, 0, 0, k.is_multiple_of(2), k < 2)
        }
    }
}

/// Rebuilds `glyf` and a long-format `loca` from the CTF glyph stream.
///
/// Some composite glyphs carry the WE_HAVE_INSTRUCTIONS flag but no instruction
/// counts in the stream (the next glyph starts right after the components), and
/// nothing in the data says so up front. Counts are read by default; when a
/// later glyph fails to decode, the most recent such composite is retried
/// without them.
fn glyf_to_ttf(
    ctf: &[u8],
    push: &[u8],
    code: &[u8],
    num_glyphs: usize,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let mut no_counts: Vec<usize> = Vec::new();
    loop {
        match decode_glyphs(ctf, push, code, num_glyphs, &no_counts) {
            Ok(out) => return Ok(out),
            Err((_, Some(retry))) if no_counts.len() < 8 => no_counts.push(retry),
            Err((e, _)) => return Err(e),
        }
    }
}

/// One pass of [`glyf_to_ttf`]. On failure, also returns the composite to retry
/// without instruction counts, if one precedes the failing glyph closely.
#[expect(
    clippy::type_complexity,
    reason = "the (glyf, loca) result and (error, retry) pair are used only by `glyf_to_ttf`"
)]
fn decode_glyphs(
    ctf: &[u8],
    push: &[u8],
    code: &[u8],
    num_glyphs: usize,
    no_counts: &[usize],
) -> Result<(Vec<u8>, Vec<u8>), (String, Option<usize>)> {
    let mut g = Be::new(ctf);
    let mut push = Be::new(push);
    let mut code = Be::new(code);
    let mut glyf = Vec::with_capacity(ctf.len() * 2);
    let mut loca = Vec::with_capacity(4 * (num_glyphs + 1));
    let mut last_counted: Option<usize> = None;
    for gid in 0..num_glyphs {
        loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        // Subset fonts may end the stream early: the remaining glyphs are empty.
        if g.at_end() {
            continue;
        }
        let counts = !no_counts.contains(&gid);
        match read_glyph(&mut g, &mut push, &mut code, counts) {
            Ok((glyph, counted_composite)) => {
                if counted_composite {
                    last_counted = Some(gid);
                }
                glyf.extend_from_slice(&glyph);
                while glyf.len() % 4 != 0 {
                    glyf.push(0);
                }
            }
            Err(e) => {
                let retry = last_counted.filter(|c| gid - c <= 4);
                return Err((format!("glyph {gid}: {e}"), retry));
            }
        }
    }
    loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
    Ok((glyf, loca))
}

/// Decodes one CTF glyph to TrueType `glyf` bytes (empty for a glyph with no
/// contours). The flag is true for a composite whose instruction counts were
/// read; `counts = false` reads such a composite without them.
fn read_glyph(
    g: &mut Be,
    push: &mut Be,
    code: &mut Be,
    counts: bool,
) -> Result<(Vec<u8>, bool), String> {
    let mut contours = g.i16()?;
    if contours == 0 {
        return Ok((Vec::new(), false));
    }
    if contours < 0 {
        return read_composite(g, push, code, counts);
    }
    // 0x7FFF flags an explicit bounding box after the real contour count.
    let mut bbox = None;
    if contours == 0x7FFF {
        contours = g.i16()?;
        bbox = Some([g.i16()?, g.i16()?, g.i16()?, g.i16()?]);
        if contours < 0 {
            return Err("negative contour count".into());
        }
    }
    let contours = contours as usize;
    let mut ends = Vec::with_capacity(contours);
    let mut last: u32 = 0;
    for i in 0..contours {
        let v = g.u255()? as u32;
        last = if i == 0 { v } else { last + v };
        ends.push(last);
    }
    if last > 0xFFFE {
        return Err("too many points".into());
    }
    let n = if contours == 0 { 0 } else { last as usize + 1 };
    let flags = g.take(n)?;
    let mut pts = Vec::with_capacity(n);
    let (mut x, mut y) = (0i32, 0i32);
    for &f in flags {
        let t = triplet(f & 0x7F);
        let nbytes = (t.bytes - 1) as u32;
        let mut data = 0u32;
        for b in g.take(nbytes as usize)? {
            data = data << 8 | *b as u32;
        }
        let total = nbytes * 8;
        let mask = |bits: u8| if bits == 0 { 0 } else { (1u32 << bits) - 1 };
        let mut dx = ((data >> (total - t.x_bits as u32)) & mask(t.x_bits)) as i32;
        let mut dy =
            ((data >> (total - t.x_bits as u32 - t.y_bits as u32)) & mask(t.y_bits)) as i32;
        dx += t.dx as i32;
        dy += t.dy as i32;
        if t.x_neg {
            dx = -dx;
        }
        if t.y_neg {
            dy = -dy;
        }
        x += dx;
        y += dy;
        pts.push((x as i16, y as i16, f & 0x80 == 0));
    }
    let instructions = read_instructions(g, push, code)?;

    let bbox = bbox.unwrap_or_else(|| {
        let mut b = [i16::MAX, i16::MAX, i16::MIN, i16::MIN];
        for &(x, y, _) in &pts {
            b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
        }
        if pts.is_empty() { [0; 4] } else { b }
    });
    let mut out = Vec::with_capacity(12 + 2 * contours + instructions.len() + 5 * n);
    out.extend_from_slice(&(contours as i16).to_be_bytes());
    for v in bbox {
        out.extend_from_slice(&v.to_be_bytes());
    }
    for e in &ends {
        out.extend_from_slice(&(*e as u16).to_be_bytes());
    }
    out.extend_from_slice(&(instructions.len() as u16).to_be_bytes());
    out.extend_from_slice(&instructions);
    write_points(&pts, &mut out);
    Ok((out, false))
}

/// TrueType simple-glyph flags and coordinates: deltas within ±255 take one
/// byte, zero deltas none, runs of equal flags use the repeat flag.
fn write_points(pts: &[(i16, i16, bool)], out: &mut Vec<u8>) {
    let mut flags = Vec::with_capacity(pts.len());
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let (mut px, mut py) = (0i32, 0i32);
    for &(x, y, on) in pts {
        let mut f = on as u8;
        for (d, short, same, buf) in [
            (x as i32 - px, 0x02u8, 0x10u8, &mut xs),
            (y as i32 - py, 0x04, 0x20, &mut ys),
        ] {
            // Deltas wrap like the 16-bit coordinates they come from.
            let d = d as i16 as i32;
            if d == 0 {
                f |= same;
            } else if d.abs() <= 255 {
                f |= short;
                if d > 0 {
                    f |= same;
                }
                buf.push(d.unsigned_abs() as u8);
            } else {
                buf.extend_from_slice(&(d as i16).to_be_bytes());
            }
        }
        px = x as i32;
        py = y as i32;
        flags.push(f);
    }
    let mut i = 0;
    while i < flags.len() {
        let f = flags[i];
        let mut run = 1;
        while i + run < flags.len() && flags[i + run] == f && run < 256 {
            run += 1;
        }
        if run > 1 {
            out.push(f | 0x08);
            out.push((run - 1) as u8);
        } else {
            out.push(f);
        }
        i += run;
    }
    out.extend_from_slice(&xs);
    out.extend_from_slice(&ys);
}

/// Composite glyph: the bounding box and component records are stored as in
/// TrueType; only the instruction counts are re-encoded.
fn read_composite(
    g: &mut Be,
    push: &mut Be,
    code: &mut Be,
    counts: bool,
) -> Result<(Vec<u8>, bool), String> {
    let mut out = Vec::new();
    out.extend_from_slice(&(-1i16).to_be_bytes());
    out.extend_from_slice(g.take(8)?);
    let mut has_instructions = false;
    loop {
        let flags = g.u16()?;
        out.extend_from_slice(&flags.to_be_bytes());
        out.extend_from_slice(g.take(2)?); // glyph index
        let args = if flags & 0x0001 != 0 { 4 } else { 2 };
        let scale = if flags & 0x0008 != 0 {
            2
        } else if flags & 0x0040 != 0 {
            4
        } else if flags & 0x0080 != 0 {
            8
        } else {
            0
        };
        out.extend_from_slice(g.take(args + scale)?);
        has_instructions |= flags & 0x0100 != 0;
        if flags & 0x0020 == 0 {
            break;
        }
    }
    if has_instructions && counts {
        let instructions = read_instructions(g, push, code)?;
        out.extend_from_slice(&(instructions.len() as u16).to_be_bytes());
        out.extend_from_slice(&instructions);
    } else if has_instructions {
        out.extend_from_slice(&0u16.to_be_bytes());
    }
    Ok((out, has_instructions && counts))
}

/// A glyph's `pushCount` and `codeSize` (both 255UShort, from the glyph stream)
/// become PUSH instructions for that many values from the push block, then that
/// many bytes from the instruction block.
fn read_instructions(g: &mut Be, push: &mut Be, code: &mut Be) -> Result<Vec<u8>, String> {
    let push_count = g.u255()? as usize;
    let code_size = g.u255()? as usize;
    let values = read_push_values(push, push_count)?;
    let mut out = Vec::with_capacity(push_count * 2 + code_size + 4);
    encode_push(&values, &mut out);
    out.extend_from_slice(code.take(code_size)?);
    Ok(out)
}

const HOP3: u8 = 251;
const HOP4: u8 = 252;

/// Push values (section 6.2.4): 255Short values, where Hop3 (`A x A`) and Hop4
/// (`A x A y A`) repeat the value two places back.
pub(super) fn read_push_values(r: &mut Be, count: usize) -> Result<Vec<i16>, String> {
    let mut v: Vec<i16> = Vec::with_capacity(count);
    while v.len() < count {
        let c = r.peek()?;
        if c == HOP3 || c == HOP4 {
            r.u8()?;
            let a = *v
                .len()
                .checked_sub(2)
                .and_then(|i| v.get(i))
                .ok_or("hop code too early")?;
            v.push(a);
            v.push(r.i255()?);
            v.push(a);
            if c == HOP4 {
                v.push(r.i255()?);
                v.push(a);
            }
        } else {
            v.push(r.i255()?);
        }
    }
    // A hop code at the very end can expand past the count; the reference
    // decoder keeps the stream position after it and the first `count` values.
    v.truncate(count);
    Ok(v)
}

/// TrueType PUSH instructions for `values`: runs of byte-sized values use
/// PUSHB/NPUSHB, other runs PUSHW/NPUSHW.
pub(super) fn encode_push(values: &[i16], out: &mut Vec<u8>) {
    let is_byte = |v: i16| (0..=255).contains(&v);
    let mut i = 0;
    while i < values.len() {
        let kind = is_byte(values[i]);
        let mut j = i;
        while j < values.len() && is_byte(values[j]) == kind && j - i < 255 {
            j += 1;
        }
        let n = j - i;
        match (kind, n <= 8) {
            (true, true) => out.push(0xB0 + (n as u8 - 1)),
            (false, true) => out.push(0xB8 + (n as u8 - 1)),
            (true, false) => out.extend_from_slice(&[0x40, n as u8]),
            (false, false) => out.extend_from_slice(&[0x41, n as u8]),
        }
        for &v in &values[i..j] {
            if kind {
                out.push(v as u8);
            } else {
                out.extend_from_slice(&v.to_be_bytes());
            }
        }
        i = j;
    }
}

/// `cvt ` (section 5.2): a count, then deltas from the previous value in a
/// 1-3 byte code: below 238 literal, 238 + word, 239-247 negative and 248-255
/// positive with a second byte (value = 238 × index + byte).
pub(super) fn cvt_to_ttf(ctf: &[u8]) -> Result<Vec<u8>, String> {
    let mut r = Be::new(ctf);
    let n = r.u16()? as usize;
    let mut out = Vec::with_capacity(2 * n);
    let mut cur: i16 = 0;
    for _ in 0..n {
        let c = r.u8()?;
        let delta: i32 = match c {
            0..=237 => c as i32,
            238 => r.i16()? as i32,
            239..=247 => -((c as i32 - 239) * 238 + r.u8()? as i32),
            _ => (c as i32 - 247) * 238 + r.u8()? as i32,
        };
        cur = cur.wrapping_add(delta as i16);
        out.extend_from_slice(&cur.to_be_bytes());
    }
    Ok(out)
}

/// LSB-first reader of magnitude-dependent values (section 5.3): `n` one-bits
/// and a zero give magnitude `n`, then a sign bit (1 = negative) when `n > 0`.
pub(super) struct MagReader<'a> {
    b: &'a [u8],
    bit: usize,
}

impl<'a> MagReader<'a> {
    pub(super) fn new(b: &'a [u8]) -> Self {
        MagReader { b, bit: 0 }
    }
    fn bit(&mut self) -> Option<bool> {
        let byte = *self.b.get(self.bit / 8)?;
        let v = byte >> (self.bit % 8) & 1 == 1;
        self.bit += 1;
        Some(v)
    }
    pub(super) fn value(&mut self) -> Option<i32> {
        let mut n = 0i32;
        while self.bit()? {
            n += 1;
            if n > 0x10000 {
                return None;
            }
        }
        if n > 0 && self.bit()? {
            n = -n;
        }
        Some(n)
    }
    /// Byte offset of the next unread whole byte.
    fn next_byte(&self) -> usize {
        self.bit.div_ceil(8)
    }
}

/// A table that could not be compressed keeps its TrueType form, with the
/// version stored as `0xFFFF - version`.
fn uncompressed_copy(ctf: &[u8]) -> Option<Vec<u8>> {
    let v = be16(ctf, 0)?;
    if v < 0x8000 {
        return None;
    }
    let mut out = ctf.to_vec();
    out[0..2].copy_from_slice(&(0xFFFF - v).to_be_bytes());
    Some(out)
}

/// `hdmx` (section 5.4): each width is the linearly scaled advance, rounded,
/// plus a magnitude-coded surprise; one bit stream covers all records.
fn hdmx_to_ttf(ctf: &[u8], advances: &[u16], units_per_em: u16) -> Option<Vec<u8>> {
    if let Some(t) = uncompressed_copy(ctf) {
        return Some(t);
    }
    let mut r = Be::new(ctf);
    let version = r.u16().ok()?;
    let records = r.u16().ok()? as usize;
    let record_size = r.u32().ok()? as usize;
    let num_glyphs = advances.len();
    if record_size < num_glyphs + 2 || record_size > num_glyphs + 64 {
        return None;
    }
    let mut heads = Vec::with_capacity(records);
    for _ in 0..records {
        heads.push((r.u8().ok()?, r.u8().ok()?));
    }
    let mut bits = MagReader::new(&ctf[r.pos..]);
    let upem = units_per_em as i64;
    let mut out = Vec::with_capacity(8 + records * record_size);
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&(records as u16).to_be_bytes());
    out.extend_from_slice(&(record_size as u32).to_be_bytes());
    for (ppem, max_width) in heads {
        let start = out.len();
        out.push(ppem);
        out.push(max_width);
        for &aw in advances {
            let predicted = ((64 * ppem as i64 * aw as i64 + upem / 2) / upem + 32) / 64;
            out.push((predicted + bits.value()? as i64) as u8);
        }
        out.resize(start + record_size, 0);
    }
    Some(out)
}

/// `VDMX` (section 5.5): the header and ratio records are stored as is; each
/// group stores its record count, two multipliers with 11 fractional bits and then,
/// per entry, magnitude-coded errors of the predicted ppem (previous + 1,
/// starting at 8), yMax and yMin. Groups are byte-aligned.
fn vdmx_to_ttf(ctf: &[u8]) -> Option<Vec<u8>> {
    if let Some(t) = uncompressed_copy(ctf) {
        return Some(t);
    }
    let num_recs = be16(ctf, 2)? as usize;
    let num_ratios = be16(ctf, 4)? as usize;
    if num_ratios == 0 {
        return Some(ctf.to_vec());
    }
    let first = be16(ctf, 6 + 4 * num_ratios)? as usize;
    let mut out = ctf.get(..first)?.to_vec();
    let mut pos = first;
    for _ in 0..num_recs {
        let recs = be16(ctf, pos)? as usize;
        let y_max_mult = be16(ctf, pos + 2)? as i16 as i32;
        let y_min_mult = be16(ctf, pos + 4)? as i16 as i32;
        let mut bits = MagReader::new(ctf.get(pos + 6..)?);
        let mut entries = Vec::with_capacity(6 * recs);
        let (mut predicted, mut start, mut end) = (8i32, 0u8, 0u8);
        for j in 0..recs {
            let ppem = predicted + bits.value()?;
            let y_max = (ppem * y_max_mult + 1024) / 2048 + bits.value()?;
            let y_min = -((ppem * y_min_mult + 1024) / 2048) + bits.value()?;
            entries.extend_from_slice(&(ppem as u16).to_be_bytes());
            entries.extend_from_slice(&(y_max as i16).to_be_bytes());
            entries.extend_from_slice(&(y_min as i16).to_be_bytes());
            if j == 0 {
                start = ppem as u8;
            }
            end = ppem as u8;
            predicted = ppem + 1;
        }
        out.extend_from_slice(&(recs as u16).to_be_bytes());
        out.extend_from_slice(&[start, end]);
        out.extend_from_slice(&entries);
        pos += 6 + bits.next_byte();
    }
    Some(out)
}

/// Serialises tables (sorted by tag) into an sfnt with checksums and the head
/// table's checkSumAdjustment.
fn write_sfnt(version: u32, tables: &BTreeMap<[u8; 4], Vec<u8>>) -> Vec<u8> {
    let n = tables.len() as u16;
    let mut pow = 1u16;
    let mut sel = 0u16;
    while pow * 2 <= n {
        pow *= 2;
        sel += 1;
    }
    let mut out = Vec::new();
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&(pow * 16).to_be_bytes());
    out.extend_from_slice(&sel.to_be_bytes());
    out.extend_from_slice(&(n * 16 - pow * 16).to_be_bytes());
    let mut offset = 12 + 16 * tables.len();
    let mut head_at = None;
    for (tag, data) in tables {
        let mut data = data.clone();
        if tag == b"head" && data.len() >= 12 {
            data[8..12].fill(0);
        }
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum(&data).to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (tag, data) in tables {
        if tag == b"head" {
            head_at = Some(out.len());
        }
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    if let Some(at) = head_at.filter(|at| at + 12 <= out.len()) {
        out[at + 8..at + 12].fill(0);
        let adj = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[at + 8..at + 12].copy_from_slice(&adj.to_be_bytes());
    }
    out
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |acc, c| {
        let mut w = [0u8; 4];
        w[..c.len()].copy_from_slice(c);
        acc.wrapping_add(u32::from_be_bytes(w))
    })
}
