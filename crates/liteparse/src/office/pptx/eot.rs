//! Embedded OpenType (EOT) font containers, the format PowerPoint uses for the
//! fonts it embeds under `ppt/fonts/*.fntdata`.
//!
//! An EOT file is a little-endian header (sizes, flags, PANOSE, OS/2 copies and
//! four UTF-16 names, plus a root string and EUDC block from version 2.1 on)
//! followed by the font data. The font data is a plain big-endian sfnt, possibly
//! XOR-obfuscated with key 0x50 and/or compressed with MicroType Express
//! ([`mtx`]). Decoding undoes both (XOR first) and returns the sfnt.
//! Format: W3C Member Submission "Embedded OpenType (EOT) File Format", 2008.

pub mod ctf;
pub mod mtx;

/// Font data is MicroType Express compressed (`TTEMBED_TTCOMPRESSED`).
pub const FLAG_COMPRESSED: u32 = 0x0000_0004;
/// Font data is XORed with [`XOR_KEY`] (`TTEMBED_XORENCRYPTDATA`).
pub const FLAG_XOR: u32 = 0x1000_0000;
pub const XOR_KEY: u8 = 0x50;
const MAGIC: u16 = 0x504C;

/// The parsed header of an EOT file, borrowing its font data.
#[derive(Debug)]
pub struct Eot<'a> {
    pub version: u32,
    pub flags: u32,
    pub weight: u32,
    pub italic: bool,
    pub family_name: String,
    pub style_name: String,
    pub full_name: String,
    /// The (possibly compressed / obfuscated) font data.
    pub font_data: &'a [u8],
}

/// Little-endian cursor over the header.
struct Le<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Le<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).filter(|e| *e <= self.b.len());
        let end = end.ok_or_else(|| format!("EOT header truncated at byte {}", self.pos))?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, String> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    /// A `Padding`, `Size`, `bytes[Size]` group: one of the UTF-16LE name fields.
    fn name(&mut self) -> Result<String, String> {
        self.u16()?;
        let n = self.u16()? as usize;
        let s = self.take(n)?;
        let units: Vec<u16> = s
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Ok(String::from_utf16_lossy(&units)
            .trim_end_matches('\0')
            .to_string())
    }
}

/// Parses the EOT header (versions 0x00010000, 0x00020001 and 0x00020002).
pub fn parse(b: &[u8]) -> Result<Eot<'_>, String> {
    let mut r = Le { b, pos: 0 };
    let eot_size = r.u32()? as usize;
    let font_data_size = r.u32()? as usize;
    let version = r.u32()?;
    let flags = r.u32()?;
    r.take(10)?; // PANOSE
    r.take(1)?; // charset
    let italic = r.take(1)?[0] == 1;
    let weight = r.u32()?;
    r.u16()?; // fsType
    if r.u16()? != MAGIC {
        return Err("not an EOT file (bad magic)".into());
    }
    if !matches!(version, 0x0001_0000 | 0x0002_0001 | 0x0002_0002) {
        return Err(format!("unknown EOT version {version:#010x}"));
    }
    // Unicode ranges (16), code page ranges (8), checksum adjustment, 4 reserved.
    r.take(16 + 8 + 4 + 16)?;
    let family_name = r.name()?;
    let style_name = r.name()?;
    r.name()?; // version string
    let full_name = r.name()?;
    if version >= 0x0002_0001 {
        r.name()?; // root string (UTF-16 URLs; irrelevant for documents)
    }
    if version == 0x0002_0002 {
        r.u32()?; // root string checksum
        r.u32()?; // EUDC code page
        r.u16()?; // padding
        let sig = r.u16()? as usize;
        r.take(sig)?;
        r.u32()?; // EUDC flags
        let eudc = r.u32()? as usize;
        r.take(eudc)?;
    }
    // FontData follows the header; fall back to "the last FontDataSize bytes of
    // EOTSize" for writers that pad the header differently.
    let font_data = match r.take(font_data_size) {
        Ok(d) => d,
        Err(_) => eot_size
            .checked_sub(font_data_size)
            .filter(|_| eot_size <= b.len())
            .map(|start| &b[start..eot_size])
            .ok_or("EOT font data out of bounds")?,
    };
    Ok(Eot {
        version,
        flags,
        weight,
        italic,
        family_name,
        style_name,
        full_name,
        font_data,
    })
}

/// Decodes an EOT file to a plain sfnt (TrueType/OpenType) font. A part that is
/// already an sfnt is returned unchanged.
pub fn decode(b: &[u8]) -> Result<Vec<u8>, String> {
    if is_sfnt(b) {
        return Ok(b.to_vec());
    }
    let eot = parse(b)?;
    let mut data = eot.font_data.to_vec();
    if eot.flags & FLAG_XOR != 0 {
        data.iter_mut().for_each(|x| *x ^= XOR_KEY);
    }
    if eot.flags & FLAG_COMPRESSED != 0 {
        data = mtx::decompress(&data)?;
    }
    if !is_sfnt(&data) {
        return Err("EOT font data is not an sfnt".into());
    }
    Ok(data)
}

/// True for TrueType (`00 01 00 00`, `true`) and CFF OpenType (`OTTO`) data.
fn is_sfnt(b: &[u8]) -> bool {
    matches!(
        b.get(..4),
        Some([0, 1, 0, 0]) | Some(b"true") | Some(b"OTTO")
    )
}

#[cfg(test)]
mod tests;
