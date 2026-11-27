//! Lenient picture recovery for pictures the strict inspector leaves unresolved: BLIPs
//! from the drawing's picture store (floating shapes, by `pib`) and from the OfficeArt
//! records that follow an inline picture's PICF in the Data stream.
use super::source::{u16_le, u32_le};
use flate2::{Decompress, FlushDecompress};

/// Largest expanded metafile accepted (bytes).
const MAX_EXPANDED: usize = 64 << 20;

/// (header, kind, body range) of the OfficeArt record at `at`, if it fits in `data`.
fn record(data: &[u8], at: usize) -> Option<(u16, u16, std::ops::Range<usize>)> {
    let head = u16_le(data, at)?;
    let kind = u16_le(data, at + 2)?;
    let len = u32_le(data, at + 4)? as usize;
    let end = (at + 8).checked_add(len)?;
    (end <= data.len()).then_some((head, kind, at + 8..end))
}

/// Decode one BLIP record into (format, bytes). The UID count follows the instance's
/// low bit; metafiles carry a 34-byte header and are zlib-compressed unless marked 0xFE.
fn blip(data: &[u8], at: usize) -> Option<(&'static str, Vec<u8>)> {
    let (head, kind, body) = record(data, at)?;
    let (format, metafile) = match kind {
        0xf01a => ("emf", true),
        0xf01b => ("wmf", true),
        0xf01d | 0xf02a => ("jpeg", false),
        0xf01e => ("png", false),
        0xf01f => ("dib", false),
        0xf029 => ("tiff", false),
        _ => return None,
    };
    let uids = if (head >> 4) & 1 == 1 { 32 } else { 16 };
    let header = uids + if metafile { 34 } else { 1 };
    let payload = data.get(body.start + header..body.end)?;
    if !metafile {
        return (!payload.is_empty()).then(|| (format, payload.to_vec()));
    }
    let meta = body.start + uids;
    let size = (u32_le(data, meta)? as usize).min(MAX_EXPANDED);
    if *data.get(meta + 32)? == 0xfe {
        return Some((format, payload.to_vec()));
    }
    let mut out = vec![0; size];
    let mut z = Decompress::new(true);
    z.decompress(payload, &mut out, FlushDecompress::Finish)
        .ok()?;
    out.truncate(z.total_out() as usize);
    (!out.is_empty()).then_some((format, out))
}

/// The BLIP of one FBSE record body: embedded after the name, or delayed in WordDocument.
fn fbse(data: &[u8], body: std::ops::Range<usize>, word: &[u8]) -> Option<(&'static str, Vec<u8>)> {
    let name_len = *data.get(body.start + 33)? as usize;
    let embedded = body.start + 36 + name_len;
    if embedded + 8 <= body.end {
        return blip(data, embedded);
    }
    let delay = u32_le(data, body.start + 28)? as usize;
    blip(word, delay)
}

/// Picture store of the drawing group: the BLIP of each FBSE, by 1-based `pib` order.
pub fn store(content: &[u8], word: &[u8]) -> Vec<Option<(&'static str, Vec<u8>)>> {
    let Some((_, 0xf000, dgg)) = record(content, 0) else {
        return Vec::new();
    };
    let mut at = dgg.start;
    while let Some((_, kind, body)) = record(content, at) {
        if kind == 0xf001 {
            let mut out = Vec::new();
            let mut entry = body.start;
            while let Some((_, kind, fbse_body)) = record(content, entry) {
                out.push(
                    (kind == 0xf007)
                        .then(|| fbse(content, fbse_body.clone(), word))
                        .flatten(),
                );
                entry = fbse_body.end;
            }
            return out;
        }
        at = body.end;
    }
    Vec::new()
}

/// Inline picture at `offset` in the Data stream: PICF (lcb, cbHeader, mapping mode and an
/// optional name), then an SpContainer and the FBSE records holding its BLIPs.
pub fn inline(data: &[u8], offset: usize, word: &[u8]) -> Option<(&'static str, Vec<u8>)> {
    let lcb = u32_le(data, offset)? as usize;
    let end = offset.checked_add(lcb)?.min(data.len());
    let mut at = offset + u16_le(data, offset + 4)? as usize;
    if u16_le(data, offset + 6)? == 0x66 {
        at += 1 + *data.get(at)? as usize;
    }
    let data = &data[..end];
    while let Some((_, kind, body)) = record(data, at) {
        if kind == 0xf007 {
            if let Some(found) = fbse(data, body.clone(), word) {
                return Some(found);
            }
        } else if let Some(found) = blip(data, at) {
            return Some(found);
        }
        at = body.end;
    }
    None
}

/// Crop of an inline picture from the OPT of the SpContainer after its PICF: top,
/// bottom, left, right as 16.16 fractions (Word 2000+ keeps crops here, not in PICF).
pub fn inline_crop(data: &[u8], offset: usize) -> [i32; 4] {
    let mut crop = [0; 4];
    let Some(lcb) = u32_le(data, offset) else {
        return crop;
    };
    let end = offset.saturating_add(lcb as usize).min(data.len());
    let Some(header) = u16_le(data, offset + 4) else {
        return crop;
    };
    let mut at = offset + header as usize;
    if u16_le(data, offset + 6) == Some(0x66) {
        at += 1 + data.get(at).copied().unwrap_or(0) as usize;
    }
    let data = &data[..end];
    let Some((_, 0xf004, body)) = record(data, at) else {
        return crop;
    };
    let mut inner = body.start;
    while let Some((head, kind, opt)) = record(data, inner) {
        if matches!(kind, 0xf00b | 0xf122) {
            for k in 0..(head >> 4) as usize {
                let (Some(op), Some(value)) = (
                    u16_le(data, opt.start + k * 6),
                    u32_le(data, opt.start + k * 6 + 2),
                ) else {
                    break;
                };
                if let 0x100..=0x103 = op & 0x3fff {
                    crop[(op & 0x3fff) as usize - 0x100] = value as i32;
                }
            }
        }
        inner = opt.end;
    }
    crop
}

/// Prefix a BITMAPFILEHEADER to a packed DIB so image decoders accept it.
pub fn dib_to_bmp(dib: &[u8]) -> Vec<u8> {
    let header = u32_le(dib, 0).unwrap_or(40) as usize;
    let bits = u16_le(dib, 14).unwrap_or(24) as usize;
    let used = u32_le(dib, 32).unwrap_or(0) as usize;
    let palette = if bits <= 8 {
        if used > 0 { used } else { 1 << bits }
    } else {
        0
    } * 4;
    let offset = 14 + header + palette;
    let mut out = Vec::with_capacity(dib.len() + 14);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((dib.len() + 14) as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(dib);
    out
}
