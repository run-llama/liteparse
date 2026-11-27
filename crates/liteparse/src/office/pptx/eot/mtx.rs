//! MicroType Express (MTX) decompression: the container, the LZCOMP decoder and
//! its adaptive Huffman coder. The decompressed blocks are Compact Table Format
//! data that [`super::ctf`] turns back into an sfnt.
//!
//! Written from the W3C Member Submission "MicroType Express (MTX) Font Format"
//! (2008), sections 2-3 and the LZCOMP decoding procedure in its Appendix C.
//!
//! Container: one version byte, then three big-endian 24-bit values (the LZCOMP
//! copy limit, and the offsets of the second and third blocks). The first block
//! starts at byte 10. Each block is compressed separately:
//!
//! 1. font tables, with `glyf`/`loca`/`cvt `/`hdmx`/`VDMX` in CTF form;
//! 2. the glyphs' initial PUSH data;
//! 3. the glyphs' remaining instructions.

/// Decompresses an MTX font to a plain sfnt.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 10 {
        return Err("MTX data too short".into());
    }
    let be24 =
        |i: usize| (data[i] as usize) << 16 | (data[i + 1] as usize) << 8 | data[i + 2] as usize;
    let version = data[0];
    let off2 = be24(4);
    let off3 = be24(7);
    if !(10 <= off2 && off2 <= off3 && off3 <= data.len()) {
        return Err(format!("MTX block offsets {off2}/{off3} out of order"));
    }
    let block = |s: &[u8]| -> Result<Vec<u8>, String> {
        if s.is_empty() {
            Ok(Vec::new())
        } else {
            lzcomp_unpack(s, version)
        }
    };
    let rest = block(&data[10..off2]).map_err(|e| format!("MTX block 1: {e}"))?;
    let push = block(&data[off2..off3]).map_err(|e| format!("MTX block 2: {e}"))?;
    let code = block(&data[off3..]).map_err(|e| format!("MTX block 3: {e}"))?;
    super::ctf::to_sfnt(&rest, &push, &code)
}

/// MSB-first bit reader (LZCOMP's BITIO).
pub(super) struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u8,
}

impl<'a> BitReader<'a> {
    pub(super) fn new(data: &'a [u8]) -> Self {
        BitReader {
            data,
            pos: 0,
            bit: 0,
        }
    }

    pub(super) fn bit(&mut self) -> Result<bool, String> {
        let byte = *self.data.get(self.pos).ok_or("unexpected end of data")?;
        let v = byte & (0x80 >> self.bit) != 0;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.pos += 1;
        }
        Ok(v)
    }

    pub(super) fn value(&mut self, bits: u32) -> Result<u32, String> {
        let mut v = 0;
        for _ in 0..bits {
            v = v << 1 | self.bit()? as u32;
        }
        Ok(v)
    }
}

#[derive(Clone, Copy)]
struct Node {
    up: usize,
    left: usize,
    right: usize,
    /// Symbol for a leaf; `None` for an internal node.
    code: Option<u16>,
    weight: u64,
}

const ROOT: usize = 1;

/// Adaptive Huffman coder over the symbols `0..range` (LZCOMP's AHUFF).
///
/// The tree lives in an array: index 1 is the root, node `i`'s children start as
/// `2i` and `2i + 1`, and leaves start at `range + symbol`. Weights stay sorted
/// in decreasing index order (the sibling property); bumping a weight swaps the
/// node with the first node of equal weight before incrementing it. Encoder and
/// decoder must evolve the tree identically, so the initial weights are part of
/// the format.
pub(super) struct Ahuff {
    tree: Vec<Node>,
    symbol_index: Vec<usize>,
}

impl Ahuff {
    pub(super) fn new(range: usize) -> Ahuff {
        let blank = Node {
            up: 0,
            left: 0,
            right: 0,
            code: None,
            weight: 0,
        };
        let mut tree = vec![blank; 2 * range];
        // Index 0 is unused; a huge weight stops the equal-weight scan there.
        tree[0].weight = u64::MAX;
        for (i, n) in tree.iter_mut().enumerate().skip(2) {
            n.up = i / 2;
            n.weight = 1;
        }
        for (i, n) in tree.iter_mut().enumerate().take(range).skip(1) {
            n.left = 2 * i;
            n.right = 2 * i + 1;
        }
        for i in 0..range {
            tree[range + i].code = Some(i as u16);
        }
        for i in (1..range).rev() {
            tree[i].weight = tree[2 * i].weight + tree[2 * i + 1].weight;
        }
        let mut h = Ahuff {
            tree,
            symbol_index: (0..range).map(|i| range + i).collect(),
        };
        if range > 256 && range < 512 {
            // The LZCOMP symbol alphabet: favour the first two copy codes and the
            // DUP2/DUP4 single-byte copies.
            h.update(h.symbol_index[256]);
            h.update(h.symbol_index[257]);
            for _ in 0..12 {
                h.update(h.symbol_index[range - 3]);
            }
            for _ in 0..6 {
                h.update(h.symbol_index[range - 2]);
            }
        } else {
            for _ in 0..2 {
                for s in 0..range {
                    h.update(h.symbol_index[s]);
                }
            }
        }
        h
    }

    fn swap(&mut self, a: usize, b: usize) {
        let (upa, upb) = (self.tree[a].up, self.tree[b].up);
        self.tree.swap(a, b);
        self.tree[a].up = upa;
        self.tree[b].up = upb;
        for i in [a, b] {
            match self.tree[i].code {
                Some(c) => self.symbol_index[c as usize] = i,
                None => {
                    let (l, r) = (self.tree[i].left, self.tree[i].right);
                    self.tree[l].up = i;
                    self.tree[r].up = i;
                }
            }
        }
    }

    /// Increments the weight of node `a` and its ancestors, keeping the sibling
    /// property by swapping with the lowest-index node of equal weight.
    fn update(&mut self, mut a: usize) {
        while a != ROOT {
            let w = self.tree[a].weight;
            let mut b = a - 1;
            if self.tree[b].weight == w {
                while self.tree[b].weight == w {
                    b -= 1;
                }
                b += 1;
                if b > ROOT {
                    self.swap(a, b);
                    a = b;
                }
            }
            self.tree[a].weight = w + 1;
            a = self.tree[a].up;
        }
        self.tree[ROOT].weight += 1;
    }

    pub(super) fn read(&mut self, bits: &mut BitReader) -> Result<u16, String> {
        let mut a = ROOT;
        let code = loop {
            a = if bits.bit()? {
                self.tree[a].right
            } else {
                self.tree[a].left
            };
            if let Some(c) = self.tree[a].code {
                break c;
            }
        };
        self.update(a);
        Ok(code)
    }

    /// Test encoder: writes `symbol` as bits (MSB first) and adapts the tree.
    #[cfg(test)]
    pub(super) fn write(&mut self, symbol: u16, out: &mut Vec<bool>) {
        let leaf = self.symbol_index[symbol as usize];
        let mut path = Vec::new();
        let mut a = leaf;
        while a != ROOT {
            let up = self.tree[a].up;
            path.push(self.tree[up].right == a);
            a = up;
        }
        out.extend(path.iter().rev());
        self.update(leaf);
    }
}

/// History bytes LZCOMP preloads before the output so that copies can start at
/// once: every (k, j) pair for k < 32, j < 96, then each byte value 4 times.
pub(super) const PRELOAD_LEN: usize = 2 * 32 * 96 + 4 * 256;

fn preload() -> Vec<u8> {
    let mut v = Vec::with_capacity(PRELOAD_LEN);
    for k in 0..32u8 {
        for j in 0..96u8 {
            v.push(k);
            v.push(j);
        }
    }
    for j in 0..=255u8 {
        v.extend_from_slice(&[j; 4]);
    }
    v
}

/// Number of 3-bit distance groups needed to address `len` bytes, as LZCOMP
/// sizes its symbol alphabet.
pub(super) fn dist_ranges(len: usize) -> usize {
    let mut n = 1;
    while 1 + (1usize << (3 * n)) - 1 < len {
        n += 1;
    }
    n
}

/// Copies from beyond this distance are one byte longer than coded.
const MAX_2BYTE_DIST: usize = 512;

/// Decodes one LZCOMP stream. `version` is the MTX version byte: from version 2
/// on, the first bit says whether the output is also run-length coded.
///
/// Symbols (main tree, alphabet `256 + 8 × ranges + 3`): a byte literal; DUP2,
/// DUP4, DUP6 (copy one byte from 2/4/6 back); or a copy item whose symbol
/// carries the number of 3-bit distance digits and the first 2-bit length digit.
/// Further length digits come from the length tree (bit 2 = more digits
/// follow), distance digits from the distance tree.
pub(super) fn lzcomp_unpack(data: &[u8], version: u8) -> Result<Vec<u8>, String> {
    let mut bits = BitReader::new(data);
    let run_length = version != 1 && bits.bit()?;
    let mut dist_tree = Ahuff::new(8);
    let mut len_tree = Ahuff::new(8);
    let out_len = bits.value(24)? as usize;
    let ranges = dist_ranges(out_len);
    let dup2 = 256 + 8 * ranges;
    let mut sym_tree = Ahuff::new(dup2 + 3);

    let mut hist = preload();
    hist.reserve(out_len);
    let end = PRELOAD_LEN + out_len;
    while hist.len() < end {
        let sym = sym_tree.read(&mut bits)? as usize;
        let pos = hist.len();
        if sym < 256 {
            hist.push(sym as u8);
        } else if (dup2..dup2 + 3).contains(&sym) {
            let back = 2 * (sym - dup2 + 1);
            hist.push(hist[pos - back]);
        } else {
            let first = sym - 256;
            let n_dist = first / 8 + 1;
            if n_dist > ranges {
                return Err("copy distance out of range".into());
            }
            let mut digit = first % 8;
            let mut len = 0usize;
            loop {
                len = len << 2 | (digit & 3);
                if digit & 4 == 0 {
                    break;
                }
                digit = len_tree.read(&mut bits)? as usize;
                if len > usize::MAX >> 4 {
                    return Err("copy length overflow".into());
                }
            }
            len += 2;
            let mut dist = 0usize;
            for _ in 0..n_dist {
                dist = dist << 3 | dist_tree.read(&mut bits)? as usize;
            }
            dist += 1;
            if dist >= MAX_2BYTE_DIST {
                len += 1;
            }
            // `dist` counts back from the last byte copied, not the first.
            let start = (pos + 1)
                .checked_sub(dist + len)
                .ok_or("copy before start of history")?;
            if pos + len > end {
                return Err("copy past end of output".into());
            }
            for j in 0..len {
                let v = hist[start + j];
                hist.push(v);
            }
        }
    }
    let out = hist.split_off(PRELOAD_LEN);
    Ok(if run_length { unrun(&out) } else { out })
}

/// Undoes LZCOMP's run-length layer: the first byte is the escape; `esc 0` is a
/// literal escape byte, `esc n v` is `n` copies of `v`.
fn unrun(data: &[u8]) -> Vec<u8> {
    let Some((&esc, rest)) = data.split_first() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(rest.len());
    let mut i = 0;
    while i < rest.len() {
        let b = rest[i];
        i += 1;
        if b != esc {
            out.push(b);
            continue;
        }
        let Some(&count) = rest.get(i) else { break };
        i += 1;
        if count == 0 {
            out.push(esc);
            continue;
        }
        let Some(&v) = rest.get(i) else { break };
        i += 1;
        out.extend(std::iter::repeat_n(v, count as usize));
    }
    out
}
