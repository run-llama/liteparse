//! OfficeArt shape properties (OPT, secondary and tertiary OPT) and their colours.
//!
//! A property table is a list of 6-byte entries: a u16 id (complex flag 0x8000, low
//! 14 bits the property number) and a u32 value. A complex property's value is the
//! byte length of its data, which follows the table in property order.
//!
//! Colours ([MS-ODRAW] OfficeArtCOLORREF) are 0x00BBGGRR plus flags in the high byte:
//! 0x08 picks the slide's colour scheme entry `red`, 0x10 a colour relative to the
//! shape's own fill or line (`red` 0xF0..), adjusted by the operation in `green`
//! with `blue` as its parameter.

use crate::office::ppt::records::{self as r, Rec};
use std::collections::HashMap;

pub mod id {
    pub const ROTATION: u16 = 0x004;
    pub const TEXT_LEFT: u16 = 0x081;
    pub const TEXT_TOP: u16 = 0x082;
    pub const TEXT_RIGHT: u16 = 0x083;
    pub const TEXT_BOTTOM: u16 = 0x084;
    pub const WRAP_TEXT: u16 = 0x085;
    pub const ANCHOR_TEXT: u16 = 0x087;
    pub const TEXT_FLOW: u16 = 0x088;
    pub const CROP_TOP: u16 = 0x100;
    pub const PIB: u16 = 0x104;
    pub const GEO_LEFT: u16 = 0x140;
    pub const VERTICES: u16 = 0x145;
    pub const SEGMENTS: u16 = 0x146;
    pub const ADJUST: u16 = 0x147;
    pub const FILL_TYPE: u16 = 0x180;
    pub const FILL_COLOR: u16 = 0x181;
    pub const FILL_OPACITY: u16 = 0x182;
    pub const PICTURE_TRANSPARENT: u16 = 0x107;
    pub const PICTURE_CONTRAST: u16 = 0x108;
    pub const PICTURE_BRIGHTNESS: u16 = 0x109;
    pub const FILL_BACK_COLOR: u16 = 0x183;
    pub const FILL_BACK_OPACITY: u16 = 0x184;
    pub const FILL_BLIP: u16 = 0x186;
    pub const FILL_ANGLE: u16 = 0x18B;
    pub const FILL_FOCUS: u16 = 0x18C;
    pub const FILL_BOOLS: u16 = 0x1BF;
    pub const LINE_COLOR: u16 = 0x1C0;
    pub const LINE_OPACITY: u16 = 0x1C1;
    pub const LINE_WIDTH: u16 = 0x1CB;
    pub const LINE_DASHING: u16 = 0x1CE;
    pub const LINE_START_ARROW: u16 = 0x1D0;
    pub const LINE_END_ARROW: u16 = 0x1D1;
    pub const LINE_START_WIDTH: u16 = 0x1D2;
    pub const LINE_START_LENGTH: u16 = 0x1D3;
    pub const LINE_END_WIDTH: u16 = 0x1D4;
    pub const LINE_END_LENGTH: u16 = 0x1D5;
    pub const LINE_BOOLS: u16 = 0x1FF;
    pub const SHADOW_COLOR: u16 = 0x201;
    pub const SHADOW_OPACITY: u16 = 0x204;
    pub const SHADOW_OFFSET_X: u16 = 0x205;
    pub const SHADOW_OFFSET_Y: u16 = 0x206;
    pub const SHADOW_BOOLS: u16 = 0x23F;
}

/// Merged property values of one shape. Later tables override earlier ones.
#[derive(Clone, Default)]
pub struct Props<'a> {
    simple: HashMap<u16, u32>,
    complex: HashMap<u16, &'a [u8]>,
}

impl<'a> Props<'a> {
    /// Adds the entries of one property table record.
    pub fn add(&mut self, data: &'a [u8], opt: Rec) {
        let count = opt.inst as usize;
        let Some(mut extra) = opt.start.checked_add(6 * count) else {
            return;
        };
        for i in 0..count {
            let (Some(pid), Some(value)) = (
                r::u16_at(data, opt.start + 6 * i),
                r::u32_at(data, opt.start + 6 * i + 2),
            ) else {
                return;
            };
            let num = pid & 0x3FFF;
            if pid & 0x8000 != 0 {
                let mut len = value as usize;
                // IMsoArray lengths sometimes leave out the 6-byte array header.
                if matches!(num, id::VERTICES | id::SEGMENTS) {
                    let n = r::u16_at(data, extra).unwrap_or(0) as usize;
                    let size = match r::u16_at(data, extra + 4).unwrap_or(0) {
                        0xFFF0 => 4,
                        c => c as usize,
                    };
                    if len + 6 == 6 + n * size {
                        len += 6;
                    }
                }
                let end = extra.saturating_add(len).min(opt.end);
                if let Some(bytes) = data.get(extra..end) {
                    self.complex.insert(num, bytes);
                }
                extra = end;
            } else {
                // Boolean groups merge bit by bit: a table sets only the bits whose
                // "use" flag (16 bits higher) is on.
                if is_bool_group(num) {
                    let used = value >> 16;
                    let old = self.simple.get(&num).copied().unwrap_or(0);
                    let merged = (old & !(used | used << 16)) | (value & (used | used << 16));
                    self.simple.insert(num, merged);
                } else {
                    self.simple.insert(num, value);
                }
            }
        }
    }

    /// Sets boolean `bit` of group `id`, with its use flag.
    pub fn add_flag(&mut self, id: u16, bit: u32, on: bool) {
        let old = self.simple.get(&id).copied().unwrap_or(0) & !(1 << bit);
        self.simple
            .insert(id, old | (1 << (bit + 16)) | (u32::from(on) << bit));
    }

    pub fn set(&mut self, id: u16, value: u32) {
        self.simple.insert(id, value);
    }

    pub fn get(&self, id: u16) -> Option<u32> {
        self.simple.get(&id).copied()
    }

    pub fn complex(&self, id: u16) -> Option<&'a [u8]> {
        self.complex.get(&id).copied()
    }

    /// Boolean `bit` of group `id` when its use flag is set.
    pub fn flag(&self, id: u16, bit: u32) -> Option<bool> {
        let v = self.get(id)?;
        (v & (1 << (bit + 16)) != 0).then_some(v & (1 << bit) != 0)
    }
}

fn is_bool_group(num: u16) -> bool {
    num & 0x3F == 0x3F
}

/// The colours a shape's paint resolves against.
#[derive(Clone, Copy)]
pub struct Scheme(pub [u32; 8]);

impl Scheme {
    /// ColorSchemeAtom: eight 0x00BBGGRR colours (background, text and lines,
    /// shadows, title text, fills, accent, accent and hyperlink, accent and followed
    /// hyperlink).
    pub fn read(data: &[u8], atom: Rec) -> Option<Scheme> {
        let mut out = [0; 8];
        for (i, c) in out.iter_mut().enumerate() {
            *c = bgr(r::u32_at(data, atom.start + 4 * i)?);
        }
        Some(Scheme(out))
    }

    pub const DEFAULT: Scheme = Scheme([
        0xFFFFFF, 0x000000, 0x808080, 0x000000, 0x00CC99, 0x3333CC, 0xCCCCFF, 0xB2B2B2,
    ]);
}

/// 0x00BBGGRR to 0xRRGGBB.
pub fn bgr(v: u32) -> u32 {
    ((v & 0xFF) << 16) | (v & 0xFF00) | ((v >> 16) & 0xFF)
}

/// A shape colour as 0xRRGGBB. `fill` and `line` are the shape's own resolved
/// colours, which relative colours (flag 0x10) refer to.
pub fn color(v: u32, scheme: &Scheme, fill: u32, line: u32) -> Option<u32> {
    let flags = v >> 24;
    let index = (v & 0xFF) as usize;
    if flags & 0x08 != 0 {
        return scheme.0.get(index).copied();
    }
    if flags & 0x10 != 0 {
        let base = match index {
            0xF0 | 0xF5 | 0xF7 | 0xF4 => fill,
            0xF1 | 0xF2 | 0xF6 => line,
            0xF3 => scheme.0[2],
            i if i < 0xF0 => system(i)?,
            _ => fill,
        };
        // `green` holds the operation (low nibble) and its flags, `blue` the parameter
        // (every relative colour in the .ppt samples: green 1 or 2, blue varied).
        let op = (v >> 8) & 0x0F;
        let param = (v >> 16) & 0xFF;
        let map = |c: u32| -> u32 {
            match op {
                1 => c * param / 255,
                2 => 255 - (255 - c) * param / 255,
                3 => (c + param).min(255),
                4 => c.saturating_sub(param),
                5 => param.saturating_sub(c),
                _ => c,
            }
        };
        let mut out = 0;
        for shift in [16, 8, 0] {
            out |= map((base >> shift) & 0xFF) << shift;
        }
        if v & 0x0000_2000 != 0 {
            out ^= 0xFFFFFF;
        }
        return Some(out);
    }
    if flags & 0x01 != 0 && flags & 0x02 == 0 {
        return None;
    }
    Some(bgr(v & 0xFF_FFFF))
}

/// Windows system colours by index (COLOR_* constants) with their default values.
fn system(i: usize) -> Option<u32> {
    Some(match i {
        0 => 0xC0C0C0,     // scroll bar
        1 => 0x008080,     // desktop
        5 => 0xFFFFFF,     // window
        6 => 0x000000,     // window frame
        7 | 8 => 0x000000, // menu text, window text
        9 => 0xFFFFFF,     // caption text
        15 => 0xC0C0C0,    // button face
        16 => 0x808080,    // button shadow
        17 => 0x808080,    // gray text
        18 => 0x000000,    // button text
        20 => 0xFFFFFF,    // button highlight
        21 => 0x404040,    // 3-D dark shadow
        22 => 0xE0E0E0,    // 3-D light
        23 => 0x000000,    // tooltip text
        24 => 0xFFFFE1,    // tooltip background
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
