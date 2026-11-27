//! Device-context state of a metafile replay: selected objects, colors, modes,
//! the clip region, and the logical-to-pixel mapping.
//!
//! GDI maps a logical point in three steps: the world transform (EMF only),
//! the page transform (window origin/extent to viewport origin/extent, shaped by
//! the map mode), then device units to our pixmap. [`Space`] holds the last,
//! fixed step; [`Dc::transform`] composes all three.

use std::rc::Rc;

use tiny_skia::{Mask, Pixmap, Transform};

/// A GDI COLORREF (0x00bbggrr).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };
    pub const WHITE: Rgb = Rgb {
        r: 255,
        g: 255,
        b: 255,
    };

    pub fn from_colorref(v: u32) -> Rgb {
        Rgb {
            r: v as u8,
            g: (v >> 8) as u8,
            b: (v >> 16) as u8,
        }
    }

    pub fn to_color(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.r, self.g, self.b, 255)
    }
}

pub const PS_NULL: u32 = 5;
pub const PS_STYLE_MASK: u32 = 0xF;
pub const PS_USERSTYLE: u32 = 7;

#[derive(Debug, Clone)]
pub struct Pen {
    /// PS_* style bits: line style (low nibble), end cap (0xF00), join (0xF000).
    pub style: u32,
    /// Width in logical units; 0 or a cosmetic pen means one device pixel.
    pub width: f32,
    pub color: Rgb,
    pub cosmetic: bool,
    /// PS_USERSTYLE dash/gap lengths, in logical units.
    pub user_dashes: Vec<f32>,
}

impl Pen {
    pub fn solid(color: Rgb) -> Pen {
        Pen {
            style: 0,
            width: 0.0,
            color,
            cosmetic: true,
            user_dashes: Vec::new(),
        }
    }
    pub fn null() -> Pen {
        Pen {
            style: PS_NULL,
            ..Pen::solid(Rgb::BLACK)
        }
    }
    pub fn is_null(&self) -> bool {
        self.style & PS_STYLE_MASK == PS_NULL
    }
}

#[derive(Clone)]
pub enum Brush {
    Null,
    Solid(Rgb),
    /// HS_* hatch style drawn in `color` over the background color.
    Hatched {
        color: Rgb,
        style: u32,
    },
    /// Tiled bitmap; one bitmap pixel is one device pixel.
    Pattern(Rc<Pixmap>),
}

#[derive(Debug, Clone)]
pub struct Font {
    /// LOGFONT lfHeight: negative = em height, positive = cell height (em +
    /// internal leading), 0 = default size. Logical units.
    pub height: i32,
    /// Average character width, logical units; 0 = natural width.
    pub width: i32,
    /// Baseline angle, tenths of a degree counterclockwise.
    pub escapement: i32,
    pub weight: i32,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
    pub charset: u8,
    pub face: String,
}

impl Default for Font {
    fn default() -> Font {
        Font {
            height: 0,
            width: 0,
            escapement: 0,
            weight: 400,
            italic: false,
            underline: false,
            strikeout: false,
            charset: 0,
            face: "Arial".to_string(),
        }
    }
}

#[derive(Clone)]
pub enum Object {
    Pen(Pen),
    Brush(Brush),
    Font(Font),
    /// Palettes, regions and other objects that occupy a slot but are not drawn.
    Other,
}

/// Clip region, in pixmap pixels. Rectangles stay rectangles (the common case)
/// so that no mask is allocated until something is drawn.
#[derive(Clone)]
pub enum Clip {
    None,
    Rect(tiny_skia::Rect),
    Mask(Rc<Mask>),
}

/// How device space relates to the output pixmap.
#[derive(Debug, Clone, Copy)]
pub enum Space {
    /// EMF: the page transform maps to reference-device pixels, which
    /// `dev_to_px` maps onto the pixmap. `dev_per_mm` sizes metric map modes.
    Device {
        dev_to_px: Transform,
        dev_per_mm: (f64, f64),
    },
    /// WMF: the window rectangle maps onto the whole pixmap (the player of a
    /// placeable WMF sets the viewport to the picture frame).
    Window { px_w: f64, px_h: f64 },
}

pub const MM_TEXT: u32 = 1;
pub const MM_ISOTROPIC: u32 = 7;
pub const MM_ANISOTROPIC: u32 = 8;

#[derive(Clone)]
pub struct Dc {
    pub pen: Pen,
    pub brush: Brush,
    pub font: Font,
    pub text_color: Rgb,
    pub bk_color: Rgb,
    pub bk_opaque: bool,
    pub text_align: u32,
    /// WINDING fill (true) or ALTERNATE / even-odd (false).
    pub winding: bool,
    pub map_mode: u32,
    pub window_org: (f64, f64),
    pub window_ext: (f64, f64),
    pub viewport_org: (f64, f64),
    pub viewport_ext: (f64, f64),
    pub world: Transform,
    /// Current position, logical units.
    pub cur: (f64, f64),
    pub clip: Clip,
    pub arc_clockwise: bool,
    pub rop2: u32,
    pub miter_limit: f32,
}

impl Default for Dc {
    fn default() -> Dc {
        Dc {
            pen: Pen::solid(Rgb::BLACK),
            brush: Brush::Solid(Rgb::WHITE),
            font: Font::default(),
            text_color: Rgb::BLACK,
            bk_color: Rgb::WHITE,
            bk_opaque: true,
            text_align: 0,
            winding: false,
            map_mode: MM_TEXT,
            window_org: (0.0, 0.0),
            window_ext: (1.0, 1.0),
            viewport_org: (0.0, 0.0),
            viewport_ext: (1.0, 1.0),
            world: Transform::identity(),
            cur: (0.0, 0.0),
            clip: Clip::None,
            arc_clockwise: false,
            rop2: 13,
            miter_limit: 10.0,
        }
    }
}

/// Logical units per millimetre is `1 / MM_PER_UNIT` for the fixed map modes
/// (MM_LOMETRIC .. MM_TWIPS, values 2..=6).
fn mm_per_unit(map_mode: u32) -> Option<f64> {
    match map_mode {
        2 => Some(0.1),
        3 => Some(0.01),
        4 => Some(0.254),
        5 => Some(0.0254),
        6 => Some(25.4 / 1440.0),
        _ => None,
    }
}

fn ratio(num: f64, den: f64) -> f64 {
    if den.abs() < 1e-12 || !num.is_finite() || !den.is_finite() {
        1.0
    } else {
        num / den
    }
}

impl Dc {
    /// Page transform: logical (post-world) coordinates to device units.
    pub fn page_transform(&self, space: &Space) -> Transform {
        let (wox, woy) = self.window_org;
        match space {
            Space::Window { px_w, px_h } => {
                let sx = ratio(*px_w, self.window_ext.0);
                let sy = ratio(*px_h, self.window_ext.1);
                Transform::from_row(
                    sx as f32,
                    0.0,
                    0.0,
                    sy as f32,
                    (-wox * sx) as f32,
                    (-woy * sy) as f32,
                )
            }
            Space::Device { dev_per_mm, .. } => {
                let (sx, sy) = if let Some(mm) = mm_per_unit(self.map_mode) {
                    // Fixed metric modes are y-up.
                    (dev_per_mm.0 * mm, -dev_per_mm.1 * mm)
                } else if self.map_mode == MM_ISOTROPIC || self.map_mode == MM_ANISOTROPIC {
                    let sx = ratio(self.viewport_ext.0, self.window_ext.0);
                    let sy = ratio(self.viewport_ext.1, self.window_ext.1);
                    if self.map_mode == MM_ISOTROPIC {
                        let m = sx.abs().min(sy.abs());
                        (m.copysign(sx), m.copysign(sy))
                    } else {
                        (sx, sy)
                    }
                } else {
                    (1.0, 1.0)
                };
                let (vox, voy) = self.viewport_org;
                Transform::from_row(
                    sx as f32,
                    0.0,
                    0.0,
                    sy as f32,
                    (vox - wox * sx) as f32,
                    (voy - woy * sy) as f32,
                )
            }
        }
    }

    /// Full logical-to-pixmap transform.
    pub fn transform(&self, space: &Space) -> Transform {
        let page = self.page_transform(space);
        let dev = match space {
            Space::Device { dev_to_px, .. } => *dev_to_px,
            Space::Window { .. } => Transform::identity(),
        };
        dev.pre_concat(page).pre_concat(self.world)
    }
}

/// Applies an XFORM-style combination (EMR_MODIFYWORLDTRANSFORM modes) to `world`.
/// GDI uses row vectors, so "left multiply" applies `x` before the current
/// transform.
pub fn modify_world(world: Transform, x: Transform, mode: u32) -> Transform {
    match mode {
        1 => Transform::identity(),
        2 => world.pre_concat(x),
        3 => world.post_concat(x),
        4 => x,
        _ => world,
    }
}

#[cfg(test)]
mod tests;
