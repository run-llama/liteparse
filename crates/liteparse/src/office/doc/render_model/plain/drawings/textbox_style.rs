//! Bounded rectangular textbox appearance after document defaults are qualified.
use crate::office::doc::{Error, Result};

#[derive(Debug, PartialEq)]
pub(super) struct TextboxStyle {
    pub text_id: u32,
    pub record_index: usize,
    /// Source inset order is left, top, right, bottom, in EMUs.
    pub insets_emu: [u32; 4],
    pub fill_rgb: Option<u32>,
    pub default_line: bool,
}

pub(super) fn resolve(properties: &[(u16, u32)], client_text_id: u32) -> Result<TextboxStyle> {
    let invalid = || Error::Unsupported("render textbox shape properties");
    if client_text_id & 0xffff != 0 || client_text_id >> 16 == 0 {
        return Err(invalid());
    }
    let mut text_id = None;
    let mut insets_emu = [91_440, 45_720, 91_440, 45_720];
    let mut fill = Some(0xffffff);
    let mut filled = Some(true);
    let mut line_disabled = false;
    let mut previous = None;
    for &(key, value) in properties {
        if previous.is_some_and(|old| old >= key) {
            return Err(invalid());
        }
        previous = Some(key);
        match key {
            0x80 => text_id = Some(value),
            0x81..=0x84 if value <= 0x0132_f540 => insets_emu[(key - 0x81) as usize] = value,
            0x87 if value == 0 => {}
            0x181 if value >> 24 == 0 => {
                fill = Some(((value & 255) << 16) | (value & 0xff00) | ((value >> 16) & 255));
            }
            0x1bf if value == 0x100010 => filled = Some(true),
            0x1bf if value == 0x100000 => filled = Some(false),
            0x1ff if value == 0x80000 => line_disabled = true,
            _ => return Err(invalid()),
        }
    }
    if text_id != Some(client_text_id) {
        return Err(invalid());
    }
    let fill_rgb = match filled {
        Some(false) => None,
        Some(true) => Some(fill.ok_or_else(invalid)?),
        None => return Err(invalid()),
    };
    Ok(TextboxStyle {
        text_id: client_text_id,
        record_index: (client_text_id >> 16) as usize - 1,
        insets_emu,
        fill_rgb,
        default_line: !line_disabled,
    })
}

#[cfg(test)]
#[path = "textbox_style/tests.rs"]
mod tests;
