//! EMF/WMF pictures (Windows metafiles), rasterized natively.
//!
//! A metafile is a recorded sequence of GDI calls. `render` replays those calls
//! onto a transparent pixmap that covers the metafile's frame (EMF `rclFrame`,
//! WMF placeable bounding box or window), with the long edge at about
//! [`RASTER_EDGE`] pixels. The caller stretches that pixmap into the picture
//! frame, exactly as PowerPoint stretches the metafile frame.
//!
//! Layout:
//! - `emf` / `wmf` parse records and translate them into calls on [`player::Player`];
//! - `player` owns the pixmap, the device-context stack and the object table, and
//!   draws shapes, bitmaps and clipping; `text` draws text runs;
//! - `dc` holds device-context state and the logical-to-pixel mapping;
//! - `dib` decodes device-independent bitmaps.
//!
//! EMF+ records (inside GDI comments) are ignored: PowerPoint writes "EMF+ dual"
//! files whose plain EMF records draw the same picture.

mod dc;
mod dib;
mod emf;
mod emfplus;
mod player;
mod text;
mod wmf;

use tiny_skia::Pixmap;

use crate::office::pptx::color::Rgba;
use crate::office::pptx::fonts::FontSystem;

/// Output raster size: long edge in pixels.
pub const RASTER_EDGE: f64 = 2048.0;
/// Output raster size cap, pixels.
const MAX_PIXELS: f64 = 8.0e6;
/// Records played per metafile before giving up on the rest.
const MAX_RECORDS: usize = 2_000_000;
/// Total path points drawn per metafile before giving up on the rest. Bounds
/// the work a hostile file with huge polylines can cause.
const MAX_POINTS: u64 = 40_000_000;

/// A rendered metafile plus the text it draws, for text extraction.
pub struct MetafileOutput {
    pub pixmap: Pixmap,
    pub text: Vec<MetafileText>,
}

/// One text-drawing record (EXTTEXTOUT, TEXTOUT, SMALLTEXTOUT). Geometry is in
/// normalized frame coordinates: 0..1 over the area the pixmap covers, origin
/// top-left.
#[derive(Debug, Clone)]
pub struct MetafileText {
    pub text: String,
    /// Axis-aligned bounding box of the run (cell height, ascent to descent).
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Font em size as a fraction of the frame height.
    pub size: f64,
    /// Baseline angle, degrees clockwise.
    pub angle: f64,
    /// Requested family (before substitution).
    pub font_family: String,
    pub bold: bool,
    pub italic: bool,
    pub color: Rgba,
}

/// Renders an EMF or WMF picture to a premultiplied, transparent-background
/// pixmap. `None` only when the data cannot be parsed as a metafile.
pub fn render(bytes: &[u8], fonts: &FontSystem) -> Option<Pixmap> {
    render_with_text(bytes, fonts).map(|o| o.pixmap)
}

/// [`render`], also returning the text runs the metafile draws.
pub fn render_with_text(bytes: &[u8], fonts: &FontSystem) -> Option<MetafileOutput> {
    if emf::is_emf(bytes) {
        emf::render(bytes, fonts)
    } else {
        wmf::render(bytes, fonts)
    }
}

/// Renders a metafile embedded in another one (EMF+ image objects), with a
/// nesting depth so self-referencing data cannot recurse forever.
fn render_nested(bytes: &[u8], fonts: &FontSystem, depth: u32) -> Option<Pixmap> {
    if emf::is_emf(bytes) {
        emf::render_depth(bytes, fonts, depth).map(|o| o.pixmap)
    } else {
        wmf::render(bytes, fonts).map(|o| o.pixmap)
    }
}

/// Pixel size of the output raster for a frame of `w` x `h` (any unit): long
/// edge [`RASTER_EDGE`], total pixels capped, each side at least 1.
pub(crate) fn raster_size(w: f64, h: f64) -> Option<(u32, u32)> {
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let mut scale = RASTER_EDGE / w.max(h);
    if w * h * scale * scale > MAX_PIXELS {
        scale = (MAX_PIXELS / (w * h)).sqrt();
    }
    let pw = (w * scale).round().clamp(1.0, RASTER_EDGE) as u32;
    let ph = (h * scale).round().clamp(1.0, RASTER_EDGE) as u32;
    Some((pw, ph))
}

/// Bounds-checked little-endian reads over one record. Every accessor returns
/// `None` past the end, so record handlers can use `?` and simply skip a
/// truncated record.
#[derive(Clone, Copy)]
pub(crate) struct Rd<'a>(pub &'a [u8]);

impl<'a> Rd<'a> {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn u8(&self, o: usize) -> Option<u8> {
        self.0.get(o).copied()
    }
    pub fn u16(&self, o: usize) -> Option<u16> {
        let b = self.0.get(o..o.checked_add(2)?)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }
    pub fn i16(&self, o: usize) -> Option<i16> {
        self.u16(o).map(|v| v as i16)
    }
    pub fn u32(&self, o: usize) -> Option<u32> {
        let b = self.0.get(o..o.checked_add(4)?)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn i32(&self, o: usize) -> Option<i32> {
        self.u32(o).map(|v| v as i32)
    }
    pub fn f32(&self, o: usize) -> Option<f32> {
        self.u32(o).map(f32::from_bits).filter(|v| v.is_finite())
    }
    pub fn bytes(&self, o: usize, len: usize) -> Option<&'a [u8]> {
        self.0.get(o..o.checked_add(len)?)
    }
}

#[cfg(test)]
mod tests;
