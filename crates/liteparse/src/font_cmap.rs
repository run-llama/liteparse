//! Reverse-cmap recovery: parse an embedded sfnt (TrueType/OpenType) font
//! program's `cmap` table and invert it to a glyph-index → unicode map.
//!
//! Used when a font's /ToUnicode is missing or garbage AND its glyph names
//! are unavailable (typical of CID TrueType subsets). The embedded font's own
//! character map is the last structural source of truth tying glyphs back to
//! unicode. Pure Rust, no deps; only formats 4, 12, 6 and 0 are parsed (these
//! cover essentially all real-world fonts).

use std::collections::HashMap;

/// Build glyph_index → unicode from an sfnt font program. Returns None when
/// the data is not sfnt (e.g. bare CFF) or has no usable cmap subtable.
pub fn reverse_cmap(data: &[u8]) -> Option<HashMap<u32, u32>> {
    let font = sfnt_slice(data)?;
    let cmap = find_table(font, b"cmap")?;

    let num_subtables = read_u16(cmap, 2)? as usize;
    // Pick the best unicode subtable: full-repertoire (fmt 12) beats BMP (fmt 4).
    let mut best: Option<(u32, u32)> = None; // (score, offset)
    for i in 0..num_subtables {
        let rec = 4 + i * 8;
        let platform = read_u16(cmap, rec)?;
        let encoding = read_u16(cmap, rec + 2)?;
        let offset = read_u32(cmap, rec + 4)?;
        // Only true unicode subtables. Mac Roman (1,0) and symbol (3,0) cmaps
        // encode charcodes, not unicode — reversing them echoes garbage (e.g.
        // Wingdings (1,0) maps the checkmark glyph back to 'ü').
        let score = match (platform, encoding) {
            (3, 10) | (0, 4) | (0, 6) => 4, // UCS-4
            (3, 1) | (0, 0..=3) => 3,       // BMP
            _ => 0,
        };
        if score > 0 && best.is_none_or(|(s, _)| score > s) {
            best = Some((score, offset));
        }
    }
    let (_, offset) = best?;
    let sub = cmap.get(offset as usize..)?;

    let mut map: HashMap<u32, u32> = HashMap::new();
    let mut add = |glyph: u32, unicode: u32| {
        if glyph == 0 || unicode == 0 {
            return;
        }
        // On collision prefer non-PUA, then non-radical, then the smaller
        // codepoint (ASCII / canonical forms over compatibility duplicates).
        // CJK fonts (Noto/Source Han, Yu Gothic) map the CJK radicals blocks
        // to the same glyph as the unified ideograph, and the radical has the
        // smaller codepoint: U+2F47 KANGXI RADICAL SUN vs U+65E5 '日'.
        let rank = |u: u32| match u {
            0xE000..=0xF8FF => 2,
            0x2E80..=0x2FDF => 1,
            _ => 0,
        };
        match map.entry(glyph) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(unicode);
            }
            std::collections::hash_map::Entry::Occupied(mut e) => {
                let cur = *e.get();
                if (rank(unicode), unicode) < (rank(cur), cur) {
                    e.insert(unicode);
                }
            }
        }
    };

    match read_u16(sub, 0)? {
        0 => {
            // Byte encoding table: 256 glyph ids at offset 6
            for code in 0..256u32 {
                let g = *sub.get(6 + code as usize)? as u32;
                add(g, code);
            }
        }
        4 => {
            let seg_count = (read_u16(sub, 6)? / 2) as usize;
            let end_codes = 14;
            let start_codes = end_codes + seg_count * 2 + 2;
            let id_deltas = start_codes + seg_count * 2;
            let id_range_offsets = id_deltas + seg_count * 2;
            for seg in 0..seg_count {
                let end = read_u16(sub, end_codes + seg * 2)? as u32;
                let start = read_u16(sub, start_codes + seg * 2)? as u32;
                let delta = read_u16(sub, id_deltas + seg * 2)? as u32;
                let range_offset = read_u16(sub, id_range_offsets + seg * 2)? as usize;
                if start == 0xFFFF && end == 0xFFFF {
                    continue;
                }
                for code in start..=end.min(0xFFFE) {
                    let glyph = if range_offset == 0 {
                        (code + delta) & 0xFFFF
                    } else {
                        // glyphIdArray indexing relative to this rangeOffset slot
                        let slot =
                            id_range_offsets + seg * 2 + range_offset + (code - start) as usize * 2;
                        let g = read_u16(sub, slot)? as u32;
                        if g == 0 { 0 } else { (g + delta) & 0xFFFF }
                    };
                    add(glyph, code);
                }
            }
        }
        6 => {
            let first = read_u16(sub, 6)? as u32;
            let count = read_u16(sub, 8)? as usize;
            for i in 0..count {
                let g = read_u16(sub, 10 + i * 2)? as u32;
                add(g, first + i as u32);
            }
        }
        12 => {
            let n_groups = read_u32(sub, 12)? as usize;
            for i in 0..n_groups.min(100_000) {
                let rec = 16 + i * 12;
                let start = read_u32(sub, rec)?;
                let end = read_u32(sub, rec + 4)?;
                let start_glyph = read_u32(sub, rec + 8)?;
                if end < start || end - start > 0x10000 {
                    continue;
                }
                for off in 0..=(end - start) {
                    add(start_glyph + off, start + off);
                }
            }
        }
        _ => return None,
    }

    if map.is_empty() { None } else { Some(map) }
}

/// Resolve TTC wrappers and validate the sfnt magic.
fn sfnt_slice(data: &[u8]) -> Option<&[u8]> {
    let tag = data.get(..4)?;
    if tag == b"ttcf" {
        let first = read_u32(data, 12)? as usize;
        let sub = data.get(first..)?;
        let m = sub.get(..4)?;
        return (m == [0, 1, 0, 0] || m == b"OTTO" || m == b"true").then_some(sub);
    }
    (tag == [0, 1, 0, 0] || tag == b"OTTO" || tag == b"true").then_some(data)
}

fn find_table<'a>(font: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    let num_tables = read_u16(font, 4)? as usize;
    for i in 0..num_tables {
        let rec = 12 + i * 16;
        if font.get(rec..rec + 4)? == tag {
            let offset = read_u32(font, rec + 8)? as usize;
            let length = read_u32(font, rec + 12)? as usize;
            return font.get(offset..offset.checked_add(length)?);
        }
    }
    None
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let b = data.get(offset..offset + 2)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let b = data.get(offset..offset + 4)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal sfnt with a single format-4 cmap subtable mapping
    /// 'A'..='C' (0x41..0x43) to glyphs 10..12.
    fn minimal_format4_font() -> Vec<u8> {
        // format 4 subtable: one real segment + terminator segment
        let mut sub: Vec<u8> = Vec::new();
        sub.extend([0, 4]); // format
        sub.extend([0, 0]); // length (unchecked)
        sub.extend([0, 0]); // language
        sub.extend([0, 4]); // segCountX2 = 4 (2 segments)
        sub.extend([0, 0, 0, 0, 0, 0]); // searchRange/entrySelector/rangeShift
        sub.extend([0x00, 0x43, 0xFF, 0xFF]); // endCodes
        sub.extend([0, 0]); // reservedPad
        sub.extend([0x00, 0x41, 0xFF, 0xFF]); // startCodes
        // idDelta: glyph = code + delta mod 65536; 10 - 0x41 = -55 = 0xFFC9
        sub.extend([0xFF, 0xC9, 0x00, 0x01]); // idDeltas
        sub.extend([0, 0, 0, 0]); // idRangeOffsets

        let mut cmap: Vec<u8> = Vec::new();
        cmap.extend([0, 0]); // version
        cmap.extend([0, 1]); // numTables
        cmap.extend([0, 3, 0, 1]); // platform 3, encoding 1
        cmap.extend(12u32.to_be_bytes()); // subtable offset
        cmap.extend(&sub);

        let mut font: Vec<u8> = Vec::new();
        font.extend([0, 1, 0, 0]); // sfnt version
        font.extend([0, 1]); // numTables
        font.extend([0, 0, 0, 0, 0, 0]); // search fields
        font.extend(b"cmap");
        font.extend([0, 0, 0, 0]); // checksum
        font.extend(28u32.to_be_bytes()); // offset (12 + 16)
        font.extend((cmap.len() as u32).to_be_bytes());
        font.extend(&cmap);
        font
    }

    #[test]
    fn parses_format4_and_inverts() {
        let font = minimal_format4_font();
        let map = reverse_cmap(&font).unwrap();
        assert_eq!(map.get(&10), Some(&0x41)); // A
        assert_eq!(map.get(&11), Some(&0x42)); // B
        assert_eq!(map.get(&12), Some(&0x43)); // C
        assert!(!map.contains_key(&0));
    }

    #[test]
    fn prefers_unified_ideograph_over_kangxi_radical() {
        // Format 12 cmap where U+2F00 (KANGXI RADICAL ONE) and U+4E00 '一'
        // share glyph 7, as in Noto Serif CJK and Yu Gothic.
        let mut sub: Vec<u8> = Vec::new();
        sub.extend([0, 12, 0, 0]); // format 12, reserved
        sub.extend(40u32.to_be_bytes()); // length
        sub.extend(0u32.to_be_bytes()); // language
        sub.extend(2u32.to_be_bytes()); // numGroups
        for start in [0x2F00u32, 0x4E00] {
            sub.extend(start.to_be_bytes()); // startCharCode
            sub.extend(start.to_be_bytes()); // endCharCode
            sub.extend(7u32.to_be_bytes()); // startGlyphID
        }

        let mut cmap: Vec<u8> = Vec::new();
        cmap.extend([0, 0]); // version
        cmap.extend([0, 1]); // numTables
        cmap.extend([0, 3, 0, 10]); // platform 3, encoding 10
        cmap.extend(12u32.to_be_bytes()); // subtable offset
        cmap.extend(&sub);

        let mut font: Vec<u8> = Vec::new();
        font.extend([0, 1, 0, 0]); // sfnt version
        font.extend([0, 1]); // numTables
        font.extend([0, 0, 0, 0, 0, 0]); // search fields
        font.extend(b"cmap");
        font.extend([0, 0, 0, 0]); // checksum
        font.extend(28u32.to_be_bytes()); // offset (12 + 16)
        font.extend((cmap.len() as u32).to_be_bytes());
        font.extend(&cmap);

        let map = reverse_cmap(&font).unwrap();
        assert_eq!(map.get(&7), Some(&0x4E00));
    }

    #[test]
    fn rejects_non_sfnt() {
        assert!(reverse_cmap(b"%!PS-AdobeFont").is_none());
        assert!(reverse_cmap(&[1, 0, 0, 0]).is_none()); // bare CFF header
        assert!(reverse_cmap(&[]).is_none());
    }
}
