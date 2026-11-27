//! Bounded OfficeArt record and encoded-image decoding shared by inline and floating pictures.
use super::{ImageAsset, MAX_ASSET_BYTES, PayloadSource};
use crate::office::doc::{Error, Result, bytes, u16_at, u32_at};
use flate2::{Decompress, FlushDecompress, Status};

#[derive(Clone, Copy, Debug)]
pub(super) struct Record {
    pub offset: usize,
    pub version: u16,
    pub instance: u16,
    pub kind: u16,
    pub start: usize,
    pub end: usize,
}

pub(super) struct Budget {
    records: usize,
    input_bytes: usize,
    pub remaining_bytes: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            records: 262_144,
            input_bytes: 256 * 1024 * 1024,
            remaining_bytes: 64 * 1024 * 1024,
        }
    }
}
impl Budget {
    pub fn record(&mut self, input: &[u8], offset: usize, end: usize) -> Result<Record> {
        self.records = self
            .records
            .checked_sub(1)
            .ok_or(Error::Limit("picture records"))?;
        if end > input.len() || offset > end || end - offset < 8 {
            return Err(Error::Invalid("OfficeArt record header"));
        }
        let bits = u16_at(input, offset)?;
        let start = offset + 8;
        let length = u32_at(input, offset + 4)? as usize;
        if length > end - start {
            return Err(Error::Invalid("OfficeArt record length"));
        }
        self.input_bytes = self
            .input_bytes
            .checked_sub(length + 8)
            .ok_or(Error::Limit("picture record work"))?;
        Ok(Record {
            offset,
            version: bits & 15,
            instance: bits >> 4,
            kind: u16_at(input, offset + 2)?,
            start,
            end: start + length,
        })
    }
    fn charge(&mut self, count: usize) -> Result<()> {
        if count > MAX_ASSET_BYTES {
            return Err(Error::Limit("picture asset bytes"));
        }
        self.remaining_bytes = self
            .remaining_bytes
            .checked_sub(count)
            .ok_or(Error::Limit("aggregate picture bytes"))?;
        Ok(())
    }
}

/// FBSE's embedded BLIP takes precedence over its delay offset. MS-DOC identifies
/// WordDocument as the delay stream for floating drawings; callers pass that stream.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-odraw/2f2d7f5e-d5c4-4cb7-b230-59b3fe8f10d6>
pub(super) fn file_block(
    input: &[u8],
    record: Record,
    stream: &'static str,
    delay: &[u8],
    budget: &mut Budget,
) -> Result<ImageAsset> {
    if record.kind != 0xf007 {
        return blip(input, record, stream, budget);
    }
    if record.version != 2 || record.end - record.start < 36 {
        return Err(Error::Invalid("OfficeArt FBSE header"));
    }
    let base = record.start;
    let name_length = input[base + 33] as usize;
    if !name_length.is_multiple_of(2) || name_length > 254 {
        return Err(Error::Invalid("OfficeArt FBSE name length"));
    }
    let name = bytes(input, base + 36, name_length)?;
    if name_length > record.end - base - 36 || (!name.is_empty() && !name.ends_with(&[0, 0])) {
        return Err(Error::Invalid("OfficeArt FBSE name"));
    }
    if record.instance != u16::from(input[base]) && record.instance != u16::from(input[base + 1]) {
        return Err(Error::Invalid("OfficeArt FBSE type"));
    }
    let embedded = base + 36 + name_length;
    if embedded < record.end {
        let child = budget.record(input, embedded, record.end)?;
        if child.end != record.end {
            return Err(Error::Unsupported("multiple embedded BLIPs"));
        }
        return blip(input, child, stream, budget);
    }
    let offset = u32_at(input, base + 28)? as usize;
    if offset == u32::MAX as usize || u32_at(input, base + 24)? == 0 {
        return Err(Error::Unsupported("empty picture store slot"));
    }
    let child = budget.record(delay, offset, delay.len())?;
    if child.end - child.offset != u32_at(input, base + 20)? as usize {
        return Err(Error::Invalid("delayed BLIP declared size"));
    }
    blip(delay, child, "WordDocument", budget)
}

/// BLIP headers describe one or two 16-byte UIDs followed by raster or metafile data.
/// Metafile compression is RFC1950 zlib, with exact compressed and expanded lengths.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-odraw/ffdcc2b8-98fd-46b2-921e-6bb6693d05e5>
fn blip(
    input: &[u8],
    record: Record,
    stream: &'static str,
    budget: &mut Budget,
) -> Result<ImageAsset> {
    if record.version != 0 {
        return Err(Error::Invalid("BLIP record version"));
    }
    let (format, first, second, metafile) = match record.kind {
        0xf01a => ("emf", 0x3d4, 0x3d5, true),
        0xf01b => ("wmf", 0x216, 0x217, true),
        0xf01d | 0xf02a => ("jpeg", 0x46a, 0x46b, false),
        0xf01e => ("png", 0x6e0, 0x6e1, false),
        0xf01c => return Err(Error::Unsupported("PICT picture payload")),
        0xf01f => return Err(Error::Unsupported("DIB picture payload")),
        0xf029 => return Err(Error::Unsupported("TIFF picture payload")),
        _ => return Err(Error::Unsupported("OfficeArt BLIP record")),
    };
    let uids = if record.instance == first || (format == "jpeg" && record.instance == 0x6e2) {
        16
    } else if record.instance == second || (format == "jpeg" && record.instance == 0x6e3) {
        32
    } else {
        return Err(Error::Invalid("BLIP record instance"));
    };
    let header = uids + if metafile { 34 } else { 1 };
    if record.end - record.start < header {
        return Err(Error::Invalid("BLIP data header"));
    }
    let start = record.start + header;
    let payload = &input[start..record.end];
    let (decoded, compression) = if metafile {
        let meta = record.start + uids;
        let size = u32_at(input, meta)? as usize;
        let saved = u32_at(input, meta + 28)? as usize;
        if saved != payload.len() || input[meta + 33] != 0xfe {
            return Err(Error::Invalid("metafile length or filter"));
        }
        budget.charge(size)?;
        match input[meta + 32] {
            0 => (inflate(payload, size)?, "zlib"),
            0xfe if size == payload.len() => (payload.to_vec(), "none"),
            0xfe => return Err(Error::Invalid("uncompressed metafile length")),
            _ => return Err(Error::Unsupported("metafile compression")),
        }
    } else {
        budget.charge(payload.len())?;
        if (format == "png" && !payload.starts_with(b"\x89PNG\r\n\x1a\n"))
            || (format == "jpeg" && !payload.starts_with(&[0xff, 0xd8, 0xff]))
        {
            return Err(Error::Invalid("BLIP raster signature"));
        }
        (payload.to_vec(), "none")
    };
    Ok(ImageAsset {
        format,
        byte_length: decoded.len(),
        source: PayloadSource {
            stream,
            record_range: [record.offset, record.end],
            payload_range: [start, record.end],
            compression,
        },
        bytes: decoded,
    })
}

fn inflate(input: &[u8], size: usize) -> Result<Vec<u8>> {
    if size > MAX_ASSET_BYTES {
        return Err(Error::Limit("metafile expanded bytes"));
    }
    let mut output = vec![0; size + 1];
    let mut decoder = Decompress::new(true);
    let status = decoder
        .decompress(input, &mut output, FlushDecompress::Finish)
        .map_err(|_| Error::Invalid("metafile zlib payload"))?;
    if status != Status::StreamEnd
        || decoder.total_in() != input.len() as u64
        || decoder.total_out() != size as u64
    {
        return Err(Error::Invalid("metafile zlib exact length"));
    }
    output.truncate(size);
    Ok(output)
}

#[cfg(test)]
mod tests;
