//! Ordered raster store entries for source-qualified drawing consumers.
use super::{ImageAsset, officeart};
use crate::office::doc::{Error, Result, bytes, compound::Compound, document::fib, u32_at};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct StoreImage {
    pub one_based_index: usize,
    pub store_record_range: [usize; 2],
    pub asset: ImageAsset,
}

/// Preserve OfficeArt store indices while reusing the bounded BLIP decoder.
/// Shape visibility, crop and header activation require separate qualification.
pub fn inspect(data: &[u8]) -> Result<Vec<StoreImage>> {
    let compound = Compound::open(data)?;
    let word = compound.stream("WordDocument")?;
    let (_, table_name, _, _, _) = fib(&word)?;
    let table = compound.stream(table_name)?;
    let start = u32_at(&word, 154 + 50 * 8)? as usize;
    let size = u32_at(&word, 158 + 50 * 8)? as usize;
    if size == 0 {
        return Ok(Vec::new());
    }
    bytes(&table, start, size)?;
    decode(&table, start, start + size, &word)
}

fn decode(table: &[u8], start: usize, end: usize, word: &[u8]) -> Result<Vec<StoreImage>> {
    let mut budget = officeart::Budget::default();
    let root = budget.record(table, start, end)?;
    if root.kind != 0xf000 || root.version != 15 || root.instance != 0 {
        return Err(Error::Invalid("raster drawing root"));
    }
    let mut at = root.start;
    let mut store_seen = false;
    let mut result = Vec::new();
    while at < root.end {
        let record = budget.record(table, at, root.end)?;
        if record.kind == 0xf001 {
            if store_seen || record.version != 15 || record.instance > 4096 {
                return Err(Error::Invalid("raster picture store"));
            }
            store_seen = true;
            let mut next = record.start;
            while next < record.end {
                let block = budget.record(table, next, record.end)?;
                if block.kind != 0xf007 || result.len() >= record.instance as usize {
                    return Err(Error::Invalid("raster store entry"));
                }
                let asset = officeart::file_block(table, block, "Table", word, &mut budget)?;
                if !matches!(asset.format, "jpeg" | "png") {
                    return Err(Error::Unsupported("render raster store format"));
                }
                result.push(StoreImage {
                    one_based_index: result.len() + 1,
                    store_record_range: [block.offset, block.end],
                    asset,
                });
                next = block.end;
            }
            if result.len() != record.instance as usize {
                return Err(Error::Invalid("raster store cardinality"));
            }
        }
        at = record.end;
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
