//! Lenient source decoding for export: FIB pairs, the piece table, CP text and the
//! character/paragraph FKP runs mapped onto CPs. Malformed optional structures are
//! skipped with a diagnostic so the rest of the document still converts.
use crate::office::doc::{Error, Result, compound::Compound};

/// FibRgFcLcb97 pair indexes ([MS-DOC] 2.5.6).
pub mod pair {
    pub const STSHF: usize = 1;
    pub const PLCFFNDREF: usize = 2;
    pub const PLCFFNDTXT: usize = 3;
    pub const PLCFSED: usize = 6;
    pub const PLCFHDD: usize = 11;
    pub const PLCFBTECHPX: usize = 12;
    pub const PLCFBTEPAPX: usize = 13;
    pub const STTBFFFN: usize = 15;
    pub const DOP: usize = 31;
    pub const CLX: usize = 33;
    pub const PLCSPAMOM: usize = 40;
    pub const DGGINFO: usize = 50;
    pub const PLCFANDREF: usize = 4;
    pub const PLCFANDTXT: usize = 5;
    pub const GRPXSTATNOWNERS: usize = 36;
    pub const PLCSPAHDR: usize = 41;
    pub const PLCFENDREF: usize = 46;
    pub const PLCFENDTXT: usize = 47;
    pub const PLCFTXBXTXT: usize = 56;
    pub const PLCFHDRTXBXTXT: usize = 58;
}

pub fn u16_le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
pub fn u32_le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
pub fn i16_le(b: &[u8], at: usize) -> Option<i16> {
    u16_le(b, at).map(|v| v as i16)
}

/// Story lengths in CP order: main, footnote, header, macro, annotation, endnote,
/// textbox, header textbox. Each story starts where the previous one ends.
pub struct Source {
    pub word: Vec<u8>,
    pub table: Vec<u8>,
    pub data: Vec<u8>,
    pub stories: [usize; 8],
    pairs: usize,
    /// UTF-16 units for every CP in the document, all stories included.
    pub text: Vec<u16>,
    pub pieces: Vec<Piece>,
    pub chars: Vec<Span>,
    pub paras: Vec<ParaSpan>,
    pub diagnostics: Vec<String>,
}

pub struct Piece {
    pub start_cp: usize,
    pub end_cp: usize,
    pub fc: usize,
    pub compressed: bool,
    pub prm: u16,
}

/// A CP range with its raw property modifier group (grpprl).
#[derive(Clone)]
pub struct Span {
    pub start_cp: usize,
    pub end_cp: usize,
    pub grpprl: Vec<u8>,
}

/// A paragraph-formatting CP range. Its end is the CP after the paragraph mark.
#[derive(Clone)]
pub struct ParaSpan {
    pub start_cp: usize,
    pub end_cp: usize,
    pub istd: u16,
    pub grpprl: Vec<u8>,
}

impl Source {
    pub fn open(data: &[u8]) -> Result<Source> {
        let compound = Compound::open(data)?;
        let word = compound.stream("WordDocument")?;
        if u16_le(&word, 0) != Some(0xa5ec) {
            return Err(Error::Unsupported("Word signature"));
        }
        let flags = u16_le(&word, 10).unwrap_or(0);
        if flags & 0x100 != 0 {
            return Err(Error::Unsupported("encrypted binary Word"));
        }
        // Word 6/95 FIBs (nFib < 0xC1) use a different layout and an ANSI piece table.
        if u16_le(&word, 2).unwrap_or(0) < 0xc1 {
            return Err(Error::Unsupported("pre-Word97 binary revision"));
        }
        let pairs = u16_le(&word, 152).ok_or(Error::Invalid("FIB offset table"))? as usize;
        let table_name = if flags & 0x200 != 0 {
            "1Table"
        } else {
            "0Table"
        };
        let table = compound.stream(table_name)?;
        let data_stream = compound.stream("Data").unwrap_or_default();
        let mut stories = [0; 8];
        for (i, len) in stories.iter_mut().enumerate() {
            *len = u32_le(&word, 76 + i * 4).unwrap_or(0) as usize;
        }
        let mut source = Source {
            word,
            table,
            data: data_stream,
            stories,
            pairs,
            text: Vec::new(),
            pieces: Vec::new(),
            chars: Vec::new(),
            paras: Vec::new(),
            diagnostics: Vec::new(),
        };
        source.read_pieces()?;
        source.read_text();
        source.chars = source.read_chars();
        source.paras = source.read_paras();
        Ok(source)
    }

    /// The (offset, length) of a FibRgFcLcb pair in the table stream, when present.
    pub fn pair(&self, index: usize) -> Option<&[u8]> {
        if index >= self.pairs {
            return None;
        }
        let off = u32_le(&self.word, 154 + index * 8)? as usize;
        let len = u32_le(&self.word, 158 + index * 8)? as usize;
        if len == 0 {
            return None;
        }
        self.table.get(off..off.checked_add(len)?)
    }

    /// First CP of story `index` (see `stories`).
    pub fn story_start(&self, index: usize) -> usize {
        self.stories[..index].iter().sum()
    }

    fn read_pieces(&mut self) -> Result<()> {
        let clx = self.pair(pair::CLX).ok_or(Error::Invalid("missing CLX"))?;
        let mut at = 0;
        while clx.get(at) == Some(&1) {
            at += 3 + u16_le(clx, at + 1).ok_or(Error::Invalid("CLX Prc"))? as usize;
        }
        if clx.get(at) != Some(&2) {
            return Err(Error::Invalid("CLX Pcdt"));
        }
        let len = u32_le(clx, at + 1).ok_or(Error::Invalid("CLX Pcdt"))? as usize;
        let plc = clx
            .get(at + 5..at + 5 + len)
            .ok_or(Error::Invalid("CLX PlcPcd"))?;
        if plc.len() < 4 || (plc.len() - 4) % 12 != 0 {
            return Err(Error::Invalid("CLX PlcPcd size"));
        }
        let n = (plc.len() - 4) / 12;
        let mut pieces = Vec::with_capacity(n);
        for i in 0..n {
            let start_cp = u32_le(plc, i * 4).unwrap() as usize;
            let end_cp = u32_le(plc, i * 4 + 4).unwrap() as usize;
            let pcd = 4 * (n + 1) + i * 8;
            let fc_raw = u32_le(plc, pcd + 2).unwrap();
            let compressed = fc_raw & 0x4000_0000 != 0;
            let fc = (fc_raw & 0x3fff_ffff) as usize;
            pieces.push(Piece {
                start_cp,
                end_cp,
                fc: if compressed { fc / 2 } else { fc },
                compressed,
                prm: u16_le(plc, pcd + 6).unwrap(),
            });
        }
        self.pieces = pieces;
        Ok(())
    }

    fn read_text(&mut self) {
        let total: usize = self.stories.iter().sum::<usize>() + 1;
        let mut text = vec![0u16; total];
        for piece in &self.pieces {
            let end = piece.end_cp.min(total);
            for (k, unit) in text[piece.start_cp.min(end)..end].iter_mut().enumerate() {
                *unit = if piece.compressed {
                    self.word
                        .get(piece.fc + k)
                        .map_or(0, |&b| compressed_unit(b))
                } else {
                    u16_le(&self.word, piece.fc + 2 * k).unwrap_or(0)
                };
            }
        }
        self.text = text;
    }

    /// Bin-table pages (PlcBte*) give 512-byte FKPs in the WordDocument stream.
    fn fkp_pages(&self, index: usize) -> Vec<&[u8]> {
        let Some(plc) = self.pair(index) else {
            return Vec::new();
        };
        if plc.len() < 4 || (plc.len() - 4) % 8 != 0 {
            return Vec::new();
        }
        let n = (plc.len() - 4) / 8;
        (0..n)
            .filter_map(|i| {
                let pn = u32_le(plc, 4 * (n + 1) + i * 4)? as usize & 0x3f_ffff;
                self.word.get(pn * 512..pn * 512 + 512)
            })
            .collect()
    }

    /// CHPX FKP runs in FC order: (start_fc, end_fc, grpprl).
    fn chpx_runs(&self) -> Vec<(usize, usize, Vec<u8>)> {
        let mut runs = Vec::new();
        for page in self.fkp_pages(pair::PLCFBTECHPX) {
            let count = page[511] as usize;
            for i in 0..count {
                let (Some(start), Some(end)) = (u32_le(page, i * 4), u32_le(page, i * 4 + 4))
                else {
                    break;
                };
                let off = *page.get(4 * (count + 1) + i).unwrap_or(&0) as usize * 2;
                let grpprl = if off == 0 {
                    Vec::new()
                } else {
                    let cb = page[off] as usize;
                    page.get(off + 1..off + 1 + cb).unwrap_or(&[]).to_vec()
                };
                runs.push((start as usize, end as usize, grpprl));
            }
        }
        runs.sort_by_key(|r| r.0);
        runs
    }

    /// PAPX FKP runs in FC order: (start_fc, end_fc, istd, grpprl).
    fn papx_runs(&self) -> Vec<(usize, usize, u16, Vec<u8>)> {
        let mut runs = Vec::new();
        for page in self.fkp_pages(pair::PLCFBTEPAPX) {
            let count = page[511] as usize;
            for i in 0..count {
                let (Some(start), Some(end)) = (u32_le(page, i * 4), u32_le(page, i * 4 + 4))
                else {
                    break;
                };
                // BxPap is 13 bytes: a word offset to the PapxInFkp, then a PHE.
                let off = *page.get(4 * (count + 1) + i * 13).unwrap_or(&0) as usize * 2;
                let (istd, grpprl) = if off == 0 {
                    (0, Vec::new())
                } else {
                    let cb = page[off] as usize;
                    let (body, len) = if cb == 0 {
                        (off + 2, 2 * *page.get(off + 1).unwrap_or(&0) as usize)
                    } else {
                        (off + 1, 2 * cb - 1)
                    };
                    let bytes = page.get(body..(body + len).min(512)).unwrap_or(&[]);
                    (
                        u16_le(bytes, 0).unwrap_or(0),
                        bytes.get(2..).unwrap_or(&[]).to_vec(),
                    )
                };
                runs.push((start as usize, end as usize, istd, grpprl));
            }
        }
        runs.sort_by_key(|r| r.0);
        runs
    }

    /// Intersect FC-ordered runs with each piece, yielding CP spans in piece order.
    fn map_fc<T: Clone>(
        &self,
        runs: &[(usize, usize, T)],
        mut emit: impl FnMut(usize, usize, Option<&T>),
    ) {
        for piece in &self.pieces {
            let width = if piece.compressed { 1 } else { 2 };
            let end_fc = piece.fc + (piece.end_cp - piece.start_cp) * width;
            let mut fc = piece.fc;
            let first = runs.partition_point(|r| r.1 <= fc);
            for run in &runs[first..] {
                if fc >= end_fc {
                    break;
                }
                if run.0 > fc {
                    let stop = run.0.min(end_fc);
                    emit(
                        piece.start_cp + (fc - piece.fc) / width,
                        piece.start_cp + (stop - piece.fc).div_ceil(width),
                        None,
                    );
                    fc = stop;
                    if fc >= end_fc {
                        break;
                    }
                }
                let stop = run.1.min(end_fc);
                emit(
                    piece.start_cp + (fc - piece.fc) / width,
                    piece.start_cp + (stop - piece.fc).div_ceil(width),
                    Some(&run.2),
                );
                fc = stop;
            }
            if fc < end_fc {
                emit(piece.start_cp + (fc - piece.fc) / width, piece.end_cp, None);
            }
        }
    }

    fn read_chars(&mut self) -> Vec<Span> {
        let runs = self.chpx_runs();
        let mut spans: Vec<Span> = Vec::new();
        self.map_fc(&runs, |start_cp, end_cp, grpprl| {
            if start_cp < end_cp {
                spans.push(Span {
                    start_cp,
                    end_cp,
                    grpprl: grpprl.cloned().unwrap_or_default(),
                });
            }
        });
        spans.sort_by_key(|s| s.start_cp);
        spans
    }

    /// Paragraph spans come from the PAPX run covering each paragraph mark; piece
    /// PRMs on that mark's piece are appended to the PAPX modifiers.
    fn read_paras(&mut self) -> Vec<ParaSpan> {
        let runs: Vec<_> = self
            .papx_runs()
            .into_iter()
            .map(|(a, b, istd, g)| (a, b, (istd, g)))
            .collect();
        type Papx = Option<(u16, Vec<u8>)>;
        let mut by_cp: Vec<(usize, usize, Papx)> = Vec::new();
        self.map_fc(&runs, |start_cp, end_cp, value| {
            if start_cp < end_cp {
                by_cp.push((start_cp, end_cp, value.cloned()));
            }
        });
        by_cp.sort_by_key(|s| s.0);
        // A 0x0C ends a paragraph only as a section mark; elsewhere it is a page break.
        let section_ends: std::collections::HashSet<usize> = self
            .pair(pair::PLCFSED)
            .map(|plc| {
                let n = plc.len().saturating_sub(4) / 16;
                (1..=n)
                    .filter_map(|i| u32_le(plc, i * 4))
                    .map(|cp| cp as usize)
                    .collect()
            })
            .unwrap_or_default();
        let mut paras = Vec::new();
        let mut start = 0;
        for (i, &unit) in self.text.iter().enumerate() {
            if !(matches!(unit, 0x0d | 0x07) || unit == 0x0c && section_ends.contains(&(i + 1))) {
                continue;
            }
            let at = by_cp.partition_point(|s| s.1 <= i);
            let (istd, grpprl) = by_cp
                .get(at)
                .filter(|s| s.0 <= i)
                .and_then(|s| s.2.clone())
                .unwrap_or_default();
            let mut grpprl = self.expand_huge(&grpprl);
            grpprl.extend(self.piece_grpprl(i));
            paras.push(ParaSpan {
                start_cp: start,
                end_cp: i + 1,
                istd,
                grpprl,
            });
            start = i + 1;
        }
        if start < self.text.len() {
            paras.push(ParaSpan {
                start_cp: start,
                end_cp: self.text.len(),
                istd: 0,
                grpprl: Vec::new(),
            });
        }
        paras
    }

    /// sprmPHugePapx moves a paragraph's modifiers into the Data stream (PrcData:
    /// a u16 byte count, then the modifiers). Other modifiers stay in place.
    fn expand_huge(&self, grpprl: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(grpprl.len());
        let mut at = 0;
        for (sprm, op) in sprms(grpprl) {
            let start = at;
            at = op.as_ptr() as usize - grpprl.as_ptr() as usize + op.len();
            if matches!(sprm, 0x6645 | 0x6646) {
                let off = u32_le(op, 0).unwrap_or(u32::MAX) as usize;
                let cb = u16_le(&self.data, off).unwrap_or(0) as usize;
                out.extend_from_slice(self.data.get(off + 2..off + 2 + cb).unwrap_or(&[]));
            } else {
                out.extend_from_slice(&grpprl[start..at]);
            }
        }
        out
    }

    /// Character modifiers for `cp`, with the piece PRM modifiers appended.
    pub fn chpx_at(&self, cp: usize) -> &[u8] {
        let i = self.chars.partition_point(|s| s.end_cp <= cp);
        self.chars
            .get(i)
            .filter(|s| s.start_cp <= cp)
            .map_or(&[], |s| s.grpprl.as_slice())
    }

    /// Piece-level property modifiers (PRM). Prm1 indexes a CLX Prc group; Prm0
    /// encodes a single one-byte-operand modifier.
    pub fn piece_grpprl(&self, cp: usize) -> Vec<u8> {
        let Some(piece) = self
            .pieces
            .iter()
            .find(|p| p.start_cp <= cp && cp < p.end_cp)
        else {
            return Vec::new();
        };
        let prm = piece.prm;
        if prm & 1 == 1 {
            let wanted = (prm >> 1) as usize;
            let Some(clx) = self.pair(pair::CLX) else {
                return Vec::new();
            };
            let mut at = 0;
            let mut index = 0;
            while clx.get(at) == Some(&1) {
                let len = u16_le(clx, at + 1).unwrap_or(0) as usize;
                if index == wanted {
                    return clx.get(at + 3..at + 3 + len).unwrap_or(&[]).to_vec();
                }
                index += 1;
                at += 3 + len;
            }
            Vec::new()
        } else {
            let isprm = (prm >> 1) & 0x7f;
            let value = (prm >> 8) as u8;
            match prm0_sprm(isprm as u8) {
                Some(sprm) if prm != 0 => {
                    let mut out = sprm.to_le_bytes().to_vec();
                    out.push(value);
                    out
                }
                _ => Vec::new(),
            }
        }
    }
}

/// The Prm0 isprm table ([MS-DOC] 2.2.6.3), limited to one-byte-operand modifiers.
fn prm0_sprm(isprm: u8) -> Option<u16> {
    Some(match isprm {
        0x04 => 0x2403,
        0x05 => 0x2405,
        0x06 => 0x2406,
        0x07 => 0x2407,
        0x41 => 0x0835,
        0x42 => 0x0836,
        0x43 => 0x0837,
        0x44 => 0x0838,
        0x45 => 0x0839,
        0x46 => 0x083a,
        0x47 => 0x083b,
        0x48 => 0x083c,
        0x4c => 0x2a3e,
        0x5e => 0x2a42,
        _ => return None,
    })
}

// MS-DOC FcCompressed uses this mapping; 0x80 remains U+0080.
pub fn compressed_unit(b: u8) -> u16 {
    const SPECIAL: [u16; 32] = [
        0x80, 0x81, 0x201a, 0x192, 0x201e, 0x2026, 0x2020, 0x2021, 0x2c6, 0x2030, 0x160, 0x2039,
        0x152, 0x8d, 0x8e, 0x8f, 0x90, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014,
        0x2dc, 0x2122, 0x161, 0x203a, 0x153, 0x9d, 0x9e, 0x178,
    ];
    if (0x80..=0x9f).contains(&b) {
        SPECIAL[(b - 0x80) as usize]
    } else {
        b as u16
    }
}

/// Iterate (sprm, operand) pairs of a grpprl, stopping at the first truncated one.
pub fn sprms(grpprl: &[u8]) -> impl Iterator<Item = (u16, &[u8])> {
    let mut at = 0;
    std::iter::from_fn(move || {
        let sprm = u16_le(grpprl, at)?;
        let mut start = at + 2;
        let len = match sprm >> 13 {
            0 | 1 => 1,
            2 | 4 | 5 => 2,
            3 => 4,
            7 => 3,
            _ => {
                // Variable length. TDefTable and PChgTabs use their own size prefixes.
                if sprm == 0xd608 || sprm == 0xd606 {
                    start += 2;
                    u16_le(grpprl, at + 2)? as usize - 1
                } else if sprm == 0xc615 {
                    let cb = *grpprl.get(start)? as usize;
                    start += 1;
                    if cb == 255 {
                        let del = *grpprl.get(start)? as usize;
                        let add = *grpprl.get(start + 1 + del * 4)? as usize;
                        1 + del * 4 + 1 + add * 3
                    } else {
                        cb
                    }
                } else {
                    let cb = *grpprl.get(start)? as usize;
                    start += 1;
                    cb
                }
            }
        };
        let operand = grpprl.get(start..start + len)?;
        at = start + len;
        Some((sprm, operand))
    })
}
