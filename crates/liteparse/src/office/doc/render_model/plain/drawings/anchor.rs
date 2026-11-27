//! Resolve source shape coordinates without choosing a paginated anchor position.
use crate::office::doc::{Error, Result};
use serde::Serialize;

#[derive(Debug, PartialEq, Serialize)]
pub(super) struct OverlayAnchor {
    pub rectangle_twips: [i32; 4],
    pub horizontal_origin: HorizontalOrigin,
    pub vertical_origin: VerticalOrigin,
    pub behind_text: bool,
    pub locked: bool,
}

#[derive(Debug, PartialEq, Serialize)]
pub enum HorizontalOrigin {
    Margin,
    Page,
    Column,
}

#[derive(Debug, PartialEq, Serialize)]
pub enum VerticalOrigin {
    Margin,
    Page,
    Paragraph,
}

/// Rectangular text flow is selected separately from overlay paint order.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum WrapSide {
    Both,
    Left,
    Right,
    Largest,
}

// Spa ignores fBelowText for square wrapping. Preserve its raw flags at the
// source owner while resolving the effective foreground wrapping behavior here.
pub(super) fn square(rectangle: [i32; 4], flags: u16) -> Result<(OverlayAnchor, WrapSide)> {
    if (flags >> 5) & 15 != 2 {
        return Err(Error::Unsupported("render textbox square wrapping"));
    }
    let side = match (flags >> 9) & 15 {
        0 => WrapSide::Both,
        1 => WrapSide::Left,
        2 => WrapSide::Right,
        3 => WrapSide::Largest,
        _ => return Err(Error::Unsupported("render textbox wrap side")),
    };
    let effective = (flags & !0x41e0) | 0x60;
    Ok((overlay(rectangle, effective)?, side))
}

// Top/bottom wrapping ignores both side preference and the behind-text bit.
// Its rectangle and independent coordinate origins retain the usual Spa bounds.
pub(super) fn top_bottom(rectangle: [i32; 4], flags: u16) -> Result<OverlayAnchor> {
    if (flags >> 5) & 15 != 1 {
        return Err(Error::Unsupported("render textbox top/bottom wrapping"));
    }
    overlay(rectangle, (flags & !0x41e0) | 0x60)
}

// MS-DOC Spa stores signed rectangle coordinates relative to independently selected
// horizontal and vertical origins. Overlay shapes leave paragraph wrapping unchanged.
pub(super) fn overlay(rectangle: [i32; 4], flags: u16) -> Result<OverlayAnchor> {
    let invalid = || Error::Unsupported("render textbox anchor geometry");
    if flags & 0x2000 != 0 || (flags >> 5) & 15 != 3 {
        return Err(invalid());
    }
    let horizontal_origin = match (flags >> 1) & 3 {
        0 => HorizontalOrigin::Margin,
        1 => HorizontalOrigin::Page,
        2 => HorizontalOrigin::Column,
        _ => return Err(invalid()),
    };
    let vertical_origin = match (flags >> 3) & 3 {
        0 => VerticalOrigin::Margin,
        1 => VerticalOrigin::Page,
        2 => VerticalOrigin::Paragraph,
        _ => return Err(invalid()),
    };
    let width = i64::from(rectangle[2]) - i64::from(rectangle[0]);
    let height = i64::from(rectangle[3]) - i64::from(rectangle[1]);
    if width <= 0 || height <= 0 || width > i64::from(i32::MAX) || height > i64::from(i32::MAX) {
        return Err(invalid());
    }
    Ok(OverlayAnchor {
        rectangle_twips: rectangle,
        horizontal_origin,
        vertical_origin,
        behind_text: flags & 0x4000 != 0,
        locked: flags & 0x8000 != 0,
    })
}

#[cfg(test)]
#[path = "anchor/tests.rs"]
mod tests;
