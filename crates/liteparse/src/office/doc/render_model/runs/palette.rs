//! MS-DOC Ico maps source palette indices to RGB or automatic text color.
use crate::office::doc::{Error, Result};

pub(super) fn resolve(operand: &[u8]) -> Result<Option<[u8; 3]>> {
    let [index] = operand else {
        return Err(Error::Invalid("render palette color operand"));
    };
    let rgb = match index {
        0 => return Ok(None),
        1 => [0, 0, 0],
        2 => [0, 0, 255],
        3 => [0, 255, 255],
        4 => [0, 255, 0],
        5 => [255, 0, 255],
        6 => [255, 0, 0],
        7 => [255, 255, 0],
        8 => [255, 255, 255],
        9 => [0, 0, 128],
        10 => [0, 128, 128],
        11 => [0, 128, 0],
        12 => [128, 0, 128],
        // The published Ico table's index 13 needs independent Word qualification.
        13 => return Err(Error::Unsupported("render palette index 13")),
        14 => [128, 128, 0],
        15 => [128, 128, 128],
        16 => [192, 192, 192],
        _ => return Err(Error::Invalid("render palette color index")),
    };
    Ok(Some(rgb))
}
