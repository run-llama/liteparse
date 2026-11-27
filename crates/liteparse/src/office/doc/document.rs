use crate::office::doc::{Error, Result, bytes, compound::Compound, u16_at, u32_at};
use serde::Serialize;

const MAX_TEXT_UNITS: usize = 16 * 1024 * 1024;
const VERSIONS: &[u16] = &[0x00c1, 0x00d9, 0x0101, 0x010c, 0x0112];

#[derive(Debug, Serialize)]
pub struct Inspection {
    pub schema_version: u32,
    pub status: &'static str,
    pub binary_version: u16,
    pub trailing_container_bytes: usize,
    pub piece_count: usize,
    pub stories: Vec<Story>,
    pub trailing_utf16_units: Vec<u16>,
}

/// Character positions count UTF-16 code units, including source control markers.
#[derive(Debug, Serialize)]
pub struct Story {
    pub kind: &'static str,
    pub start_cp: usize,
    pub utf16_units: usize,
    pub text: String,
}

/// Inspects modern binary Word text; success does not imply rendered or accepted-revision text.
pub fn inspect(data: &[u8]) -> Result<Inspection> {
    let c = Compound::open(data)?;
    let word = c.stream("WordDocument")?;
    let (version, table_name, lengths, clx_off, clx_len) = fib(&word)?;
    let table = c.stream(table_name)?;
    let (units, piece_count) = pieces(&word, bytes(&table, clx_off, clx_len)?)?;
    let names = [
        "body",
        "footnotes",
        "headers",
        "reserved_macro",
        "comments",
        "endnotes",
        "textboxes",
        "header_textboxes",
    ];
    let mut stories = Vec::new();
    let mut cp = 0usize;
    for (kind, len) in names.into_iter().zip(lengths) {
        let end = cp
            .checked_add(len)
            .ok_or(Error::Invalid("story length overflow"))?;
        let raw = units
            .get(cp..end)
            .ok_or(Error::Invalid("story extends past piece table"))?;
        let text = String::from_utf16(raw).map_err(|_| Error::Invalid("story UTF-16"))?;
        stories.push(Story {
            kind,
            start_cp: cp,
            utf16_units: len,
            text,
        });
        cp = end;
    }
    // A terminal paragraph marker can follow the specialized document stories.
    // Preserve it separately so the inspection does not silently drop source units.
    let trailing = &units[cp..];
    if trailing.len() > 1 {
        return Err(Error::Invalid("unassigned story text"));
    }
    Ok(Inspection {
        schema_version: 1,
        status: "raw_stories_only",
        binary_version: version,
        trailing_container_bytes: c.trailing_bytes(),
        piece_count,
        stories,
        trailing_utf16_units: trailing.to_vec(),
    })
}

pub(crate) type Fib = (u16, &'static str, [usize; 8], usize, usize);
pub(crate) fn fib(word: &[u8]) -> Result<Fib> {
    if u16_at(word, 0)? != 0xa5ec {
        return Err(Error::Unsupported("Word signature"));
    }
    let base = u16_at(word, 2)?;
    if !VERSIONS.contains(&base) {
        return Err(Error::Unsupported("binary Word revision"));
    }
    let flags = u16_at(word, 10)?;
    if flags & 0x100 != 0 {
        return Err(Error::Unsupported("encrypted binary Word"));
    }
    if u16_at(word, 32)? != 14 || u16_at(word, 62)? != 22 {
        return Err(Error::Invalid("FIB array sizes"));
    }
    let n = u16_at(word, 152)? as usize;
    if n < 93 {
        return Err(Error::Invalid("FIB offset table"));
    }
    bytes(word, 154, n * 8)?;
    let end = 154 + n * 8;
    let extra = u16_at(word, end)? as usize;
    bytes(word, end + 2, extra * 2)?;
    let version = if extra == 0 {
        base
    } else {
        u16_at(word, end + 2)?
    };
    // Writers outside the spec (nFibNew 0xC3) keep a standard offset table; its size
    // names the revision whose layout it follows.
    let version = if VERSIONS.contains(&version) {
        version
    } else {
        match n {
            93 => 0x00c1,
            108 => 0x00d9,
            136 => 0x0101,
            164 => 0x010c,
            183 => 0x0112,
            _ => return Err(Error::Unsupported("extended binary Word revision")),
        }
    };
    let mut lengths = [0; 8];
    for (i, len) in lengths.iter_mut().enumerate() {
        *len = u32_at(word, 76 + i * 4)? as usize;
    }
    if lengths.iter().sum::<usize>() > MAX_TEXT_UNITS {
        return Err(Error::Limit("text length"));
    }
    if lengths[3] != 0 {
        return Err(Error::Unsupported("macro story"));
    }
    let table = if flags & 0x200 != 0 {
        "1Table"
    } else {
        "0Table"
    };
    Ok((
        version,
        table,
        lengths,
        u32_at(word, 154 + 33 * 8)? as usize,
        u32_at(word, 154 + 33 * 8 + 4)? as usize,
    ))
}

fn pieces(word: &[u8], clx: &[u8]) -> Result<(Vec<u16>, usize)> {
    let mut off = 0;
    // CLX stores optional property groups before its single character-position table.
    while clx.get(off) == Some(&1) {
        let len = u16_at(clx, off + 1)? as usize;
        bytes(clx, off + 3, len)?;
        off += 3 + len;
    }
    if clx.get(off) != Some(&2) {
        return Err(Error::Invalid("missing piece table"));
    }
    let len = u32_at(clx, off + 1)? as usize;
    let plc = bytes(clx, off + 5, len)?;
    if off + 5 + len != clx.len() || len < 4 || !(len - 4).is_multiple_of(12) {
        return Err(Error::Invalid("piece table size"));
    }
    let n = (len - 4) / 12;
    let total = u32_at(plc, n * 4)? as usize;
    if total > MAX_TEXT_UNITS {
        return Err(Error::Limit("piece table text length"));
    }
    if u32_at(plc, 0)? != 0 {
        return Err(Error::Invalid("initial character position"));
    }
    let mut units = Vec::with_capacity(total);
    for i in 0..n {
        let cp = u32_at(plc, i * 4)? as usize;
        let end = u32_at(plc, (i + 1) * 4)? as usize;
        if cp != units.len() || end <= cp || end > total {
            return Err(Error::Invalid("piece character positions"));
        }
        let fc = u32_at(plc, (n + 1) * 4 + i * 8 + 2)?;
        let compressed = fc & 0x4000_0000 != 0;
        let offset = (fc & 0x3fff_ffff) as usize;
        if compressed {
            if !offset.is_multiple_of(2) {
                return Err(Error::Invalid("compressed text offset"));
            }
            units.extend(
                bytes(word, offset / 2, end - cp)?
                    .iter()
                    .map(|&v| compressed_unit(v)),
            );
        } else {
            for v in bytes(word, offset, (end - cp) * 2)?.as_chunks::<2>().0 {
                units.push(u16::from_le_bytes([v[0], v[1]]));
            }
        }
    }
    Ok((units, n))
}

// MS-DOC FcCompressed uses this mapping; 0x80 remains U+0080.
fn compressed_unit(b: u8) -> u16 {
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

#[cfg(test)]
mod tests;
