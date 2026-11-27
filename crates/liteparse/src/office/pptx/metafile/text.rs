//! Text output (EXTTEXTOUT, TEXTOUT, SMALLTEXTOUT): font selection, glyph
//! positioning (honoring the per-character advance array when present),
//! alignment, background and decorations.
//!
//! Layout happens in pixmap space: the reference point is mapped, then glyphs
//! advance along the baseline direction (escapement plus any world rotation),
//! upright on the device whatever the y orientation of the mapping.

use rustybuzz::ttf_parser;
use tiny_skia::{FillRule, PathBuilder, Transform};

use super::MetafileText;
use super::dc::Rgb;
use super::player::{Player, map, solid, vec_len};
use crate::office::pptx::color::Rgba;
use crate::office::pptx::fonts::map_symbol_char;
use crate::office::pptx::render::anti_alias_fits;

pub const ETO_OPAQUE: u32 = 0x2;
pub const ETO_CLIPPED: u32 = 0x4;
pub const ETO_GLYPH_INDEX: u32 = 0x10;
pub const ETO_PDY: u32 = 0x2000;

const TA_UPDATECP: u32 = 1;
const TA_RIGHT: u32 = 2;
const TA_CENTER: u32 = 6;
const TA_BOTTOM: u32 = 8;
const TA_BASELINE: u32 = 24;

/// Characters drawn per metafile call; longer strings are cut.
const MAX_CHARS: usize = 16_384;

pub struct TextRun {
    /// Reference point, logical units.
    pub x: f64,
    pub y: f64,
    /// Unicode code points, or glyph indices with ETO_GLYPH_INDEX.
    pub text: Vec<u32>,
    pub options: u32,
    /// Advance after each character, logical units (empty: font advances).
    pub dx: Vec<f64>,
    /// Vertical advance after each character (ETO_PDY), logical units.
    pub dy: Vec<f64>,
    /// Opaque/clip rectangle (l, t, r, b), logical units.
    pub rect: Option<(f64, f64, f64, f64)>,
}

/// Decodes 8-bit text in the font's character set (LOGFONT lfCharSet): one
/// entry per character with the number of bytes it used, so per-byte advance
/// arrays can be merged. Symbol fonts keep raw byte values (their glyphs are
/// addressed by byte); unknown sets use Windows-1252.
pub fn decode_ansi_spans(bytes: &[u8], charset: u8) -> Vec<(u32, usize)> {
    if charset == 2 {
        return bytes.iter().map(|&b| (b as u32, 1)).collect();
    }
    let enc = encoding_for(charset);
    let double_byte = matches!(charset, 128 | 129 | 130 | 134 | 136);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let lead = bytes[i];
        let len = if double_byte && i + 1 < bytes.len() && is_lead(charset, lead) {
            2
        } else {
            1
        };
        let (text, _) = enc.decode_without_bom_handling(&bytes[i..i + len]);
        let ch = text.chars().next().map(|c| c as u32).unwrap_or(0xFFFD);
        out.push((ch, len));
        i += len;
    }
    out
}

/// [`decode_ansi_spans`] without the byte counts.
pub fn decode_ansi(bytes: &[u8], charset: u8) -> Vec<u32> {
    decode_ansi_spans(bytes, charset)
        .into_iter()
        .map(|(c, _)| c)
        .collect()
}

/// Sums a per-byte advance array into per-character advances.
pub fn merge_dx(spans: &[(u32, usize)], dx: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(spans.len());
    let mut i = 0;
    for &(_, n) in spans {
        if i >= dx.len() {
            break;
        }
        out.push(dx[i..(i + n).min(dx.len())].iter().sum());
        i += n;
    }
    out
}

fn is_lead(charset: u8, b: u8) -> bool {
    match charset {
        128 => (0x81..=0x9F).contains(&b) || (0xE0..=0xFC).contains(&b),
        _ => b >= 0x81 && b != 0xFF,
    }
}

fn encoding_for(charset: u8) -> &'static encoding_rs::Encoding {
    use encoding_rs::*;
    match charset {
        128 => SHIFT_JIS,
        129 | 130 => EUC_KR,
        134 => GBK,
        136 => BIG5,
        161 => WINDOWS_1253,
        162 => WINDOWS_1254,
        163 => WINDOWS_1258,
        177 => WINDOWS_1255,
        178 => WINDOWS_1256,
        186 => WINDOWS_1257,
        204 => WINDOWS_1251,
        222 => WINDOWS_874,
        238 => WINDOWS_1250,
        _ => WINDOWS_1252,
    }
}

/// Adobe Symbol encoding (the "Symbol" font, addressed by byte or as U+F0xx)
/// to Unicode, for 0x20..=0xFE.
pub fn symbol_to_unicode(code: u32) -> Option<char> {
    const LOW: &str = " !∀#∃%&∋()∗+,−./0123456789:;<=>?≅ΑΒΧΔΕΦΓΗΙϑΚΛΜΝΟΠΘΡΣΤΥςΩΞΨΖ[∴]⊥_‾αβχδεφγηιϕκλμνοπθρστυϖωξψζ{|}∼";
    const HIGH: &str = "€ϒ′≤⁄∞ƒ♣♦♥♠↔←↑→↓°±″≥×∝∂•÷≠≡≈…⏐⎯↵ℵℑℜ℘⊗⊕∅∩∪⊃⊇⊄⊂⊆∈∉∠∇®©™∏√⋅¬∧∨⇔⇐⇑⇒⇓◊〈®©™∑⎛⎜⎝⎡⎢⎣⎧⎨⎩⎪\u{F8FF}〉∫⌠⎮⌡⎞⎟⎠⎤⎥⎦⎫⎬⎭";
    let c = if (0xF020..=0xF0FE).contains(&code) {
        code - 0xF000
    } else {
        code
    };
    match c {
        0x20..=0x7E => LOW.chars().nth((c - 0x20) as usize),
        0xA0..=0xFE => HIGH.chars().nth((c - 0xA0) as usize),
        _ => None,
    }
}

/// Wide-character records may contain actual Unicode rather than font slots.
fn legacy_slot(code: u32) -> bool {
    code <= 0xFF || (0xF000..=0xF0FF).contains(&code)
}

/// Unambiguous slots shared by the historical reduced MT Extra encoding.
/// <https://www-archive.mozilla.org/projects/mathml/fonts/encoding/mtextra>
/// Multipart delimiter/arrow pieces have no single Unicode equivalent here.
fn mt_extra_to_unicode(code: u32) -> Option<char> {
    let code = match code {
        0x20..=0xFF => code,
        0xF020..=0xF0FF => code - 0xF000,
        _ => return None,
    };
    Some(match code {
        0x20 => ' ',
        0x23 => '\u{0300}',
        0x24 => '\u{0302}',
        0x25 => '\u{0303}',
        0x26 => '\u{0307}',
        0x3C => '\u{25C3}',
        0x3E => '\u{25B9}',
        0x43 => '\u{2210}',
        0x44 => '\u{019B}',
        0x49 => '\u{2229}',
        0x4B => '\u{2026}',
        0x4C => '\u{22EF}',
        0x4D => '\u{22EE}',
        0x4E => '\u{22F0}',
        0x4F => '\u{22F1}',
        0x51 => '\u{2235}',
        0x55 => '\u{222A}',
        0x60 => '\u{2035}',
        0x61 => '\u{21A6}',
        0x62 => '\u{2195}',
        0x63 => '\u{21D5}',
        0x66 => '\u{227B}',
        0x68 => '\u{210F}',
        0x6C => '\u{2113}',
        0x6D => '\u{2213}',
        0x6F => '\u{2218}',
        0x70 => '\u{227A}',
        _ => return None,
    })
}

/// Unmapped legacy slots remain explicit in extracted text, even when an
/// installed font can draw their source glyph. The source run retains its font.
fn readable_char(name: &str, symbol_map: bool, code: u32) -> Option<char> {
    let ch = char::from_u32(code)?;
    if name.eq_ignore_ascii_case("mt extra") && legacy_slot(code) {
        Some(mt_extra_to_unicode(code).unwrap_or('\u{FFFD}'))
    } else if name.eq_ignore_ascii_case("symbol") {
        symbol_to_unicode(code).or(Some(ch))
    } else if symbol_map {
        map_symbol_char(name, ch)
    } else {
        Some(ch)
    }
}

struct Placed {
    face: fontdb::ID,
    gid: u16,
    /// Pixels per font unit of this glyph's face.
    scale: f32,
    /// Offset along the baseline and perpendicular to it (down), pixels.
    along: f32,
    across: f32,
}

impl Player<'_> {
    pub fn text_out(&mut self, run: TextRun) {
        if run.text.is_empty() && run.options & ETO_OPAQUE == 0 {
            return;
        }
        if !self.spend(run.text.len()) {
            return;
        }
        let t = self.t();
        // Opaque rectangle, drawn even when there is no text.
        if let Some((l, tp, r, b)) = run.rect
            && run.options & ETO_OPAQUE != 0
        {
            let bk = self.dc.bk_color;
            self.fill_logical_quad(&t, [(l, tp), (r, tp), (r, b), (l, b)], bk);
        }
        if run.text.is_empty() {
            return;
        }
        let saved_clip = self.dc.clip.clone();
        if let (Some((l, tp, r, b)), true) = (run.rect, run.options & ETO_CLIPPED != 0) {
            self.clip_logical_rect(l, tp, r, b, false);
        }
        self.layout_and_draw(&run, &t);
        self.dc.clip = saved_clip;
    }

    fn layout_and_draw(&mut self, run: &TextRun, t: &Transform) {
        let fonts = self.fonts;
        let font = self.dc.font.clone();
        let bold = font.weight >= 600;
        let name = font.face.trim();
        let name = if name.is_empty() { "Arial" } else { name };
        let Some(primary) = fonts.resolve(name, bold, font.italic) else {
            return;
        };
        let Some(pfd) = fonts.face(primary) else {
            return;
        };
        let exact = pfd.family.eq_ignore_ascii_case(name);
        let symbol_map = map_symbol_char(name, 'a').is_some();
        let m = pfd.metrics;
        let upem = m.units_per_em.max(1.0);

        // Font size: negative height is the em, positive includes internal leading.
        let h = if font.height == 0 {
            12.0
        } else {
            font.height.unsigned_abs() as f32
        };
        let em_log = if font.height > 0 {
            h * upem / (m.ascent + m.descent).max(1.0)
        } else {
            h
        };
        let xscale = vec_len(t, 1.0, 0.0);
        let yscale = vec_len(t, 0.0, 1.0);
        let em_px = em_log * yscale;
        if !(em_px.is_finite() && em_px > 0.05 && em_px < 20_000.0) {
            return;
        }
        let s = em_px / upem;
        let mut wf = 1.0f32;
        if font.width != 0 {
            let avg = ttf_parser::Face::parse((*pfd.data).as_ref(), pfd.index)
                .ok()
                .and_then(|f| {
                    f.raw_face()
                        .table(ttf_parser::Tag::from_bytes(b"OS/2"))
                        .and_then(|t| t.get(2..4))
                        .map(|b| i16::from_be_bytes([b[0], b[1]]) as f32)
                })
                .filter(|v| *v > 0.0)
                .unwrap_or(upem * 0.5);
            wf = (font.width.unsigned_abs() as f32 / (avg * em_log / upem)).clamp(0.2, 5.0);
        }

        // Baseline direction in pixels: the mapped x axis, turned by the escapement.
        let ux = (t.sx, t.ky);
        let mut theta = ux.1.atan2(ux.0);
        if ux.0 < 0.0 && ux.1.abs() < 1e-6 * ux.0.abs() {
            theta = 0.0; // mirrored x: GDI still writes left to right
        }
        theta -= (font.escapement as f32 / 10.0).to_radians();
        let d = (theta.cos(), theta.sin());
        let n = (theta.sin(), -theta.cos()); // "up" on the page

        let mut placed: Vec<Placed> = Vec::with_capacity(run.text.len().min(MAX_CHARS));
        let (mut pen, mut drop) = (0.0f32, 0.0f32);
        for (i, &code) in run.text.iter().take(MAX_CHARS).enumerate() {
            let glyph = if run.options & ETO_GLYPH_INDEX != 0 {
                // Glyph ids only mean something in the exact font.
                exact.then_some((primary, code as u16))
            } else {
                self.pick_glyph(code, name, primary, exact, symbol_map, bold, font.italic)
            };
            let gscale = glyph.map(|(f, _)| face_scale(fonts, f, em_px)).unwrap_or(s);
            let font_adv = glyph
                .map(|(f, g)| advance(fonts, f, g) * gscale * wf)
                .unwrap_or(0.0);
            if let Some((face, gid)) = glyph
                && (code >= 0x20 || run.options & ETO_GLYPH_INDEX != 0)
            {
                placed.push(Placed {
                    face,
                    gid,
                    scale: gscale,
                    along: pen,
                    across: drop,
                });
            }
            match run.dx.get(i) {
                Some(&dx) => pen += dx as f32 * xscale,
                None => pen += font_adv,
            }
            if let Some(&dy) = run.dy.get(i) {
                drop -= dy as f32 * yscale;
            }
        }
        let total = pen;

        let ascent = m.ascent * s;
        let descent = m.descent * s;
        let align = self.dc.text_align;
        let use_cp = align & TA_UPDATECP != 0;
        let (rx, ry) = if use_cp { self.dc.cur } else { (run.x, run.y) };
        let refp = map(t, rx, ry);
        let shift_along = match align & TA_CENTER {
            TA_CENTER => -total / 2.0,
            TA_RIGHT => -total,
            _ => 0.0,
        };
        let shift_up = match align & TA_BASELINE {
            TA_BASELINE => 0.0,
            TA_BOTTOM => descent,
            _ => -ascent,
        };
        let origin = (
            refp.0 + d.0 * shift_along + n.0 * shift_up,
            refp.1 + d.1 * shift_along + n.1 * shift_up,
        );
        if use_cp {
            // Advance the current position by the run width, in logical units.
            let adv = if xscale > 0.0 {
                (total / xscale) as f64
            } else {
                0.0
            };
            let dir = if align & TA_CENTER == TA_RIGHT {
                -1.0
            } else {
                1.0
            };
            self.dc.cur = (self.dc.cur.0 + dir * adv, self.dc.cur.1);
        }

        let at = |along: f32, up: f32| {
            (
                origin.0 + d.0 * along + n.0 * up,
                origin.1 + d.1 * along + n.1 * up,
            )
        };
        if run.options & ETO_GLYPH_INDEX == 0 {
            let string: String = run
                .text
                .iter()
                .filter_map(|&c| readable_char(name, symbol_map, c))
                .filter(|c| *c >= ' ' && !('\u{E000}'..='\u{F8FF}').contains(c))
                .collect();
            if !string.trim().is_empty() {
                let quad = [
                    at(0.0, ascent),
                    at(total, ascent),
                    at(total, -descent),
                    at(0.0, -descent),
                ];
                self.record_text(string, quad, em_px, theta, name, bold, font.italic);
            }
        }
        if self.dc.bk_opaque && run.options & ETO_OPAQUE == 0 && total.abs() > 0.0 {
            let quad = [
                at(0.0, ascent),
                at(total, ascent),
                at(total, -descent),
                at(0.0, -descent),
            ];
            let bk = self.dc.bk_color;
            self.fill_px_quad(quad, bk);
        }

        let color = self.dc.text_color;
        let paint = solid(color);
        let mask = self.clip_mask();
        for g in &placed {
            let Some(outline) = fonts.outline(g.face, g.gid) else {
                continue;
            };
            let o = at(g.along, -g.across);
            let gs = g.scale;
            let gt =
                Transform::from_row(d.0 * gs * wf, d.1 * gs * wf, n.0 * gs, n.1 * gs, o.0, o.1);
            let mut paint = paint.clone();
            paint.anti_alias = anti_alias_fits(&outline, gt, 0.0);
            self.pm
                .fill_path(&outline, &paint, FillRule::Winding, gt, mask.as_deref());
        }

        let decorate = |pos: f32, thick: f32, this: &mut Self| {
            let half = (thick * s).max(this.dev_px.min(1.5)) / 2.0;
            let up = pos * s;
            let quad = [
                at(0.0, up + half),
                at(total, up + half),
                at(total, up - half),
                at(0.0, up - half),
            ];
            this.fill_px_quad(quad, color);
        };
        if font.underline {
            decorate(m.underline_pos, m.underline_thickness, self);
        }
        if font.strikeout {
            decorate(m.strikeout_pos, m.underline_thickness, self);
        }
    }

    /// Face and glyph for a character: the requested font (symbol fonts by
    /// private-use code, or mapped to Unicode when substituted), then fallback.
    #[expect(
        clippy::too_many_arguments,
        reason = "the lookup takes the code, the requested font, and the per-run cmap and style flags"
    )]
    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "exact and symbol_map pick the cmap lookup, bold and italic pick the fallback face; each is an independent per-run input"
    )]
    fn pick_glyph(
        &self,
        code: u32,
        name: &str,
        primary: fontdb::ID,
        exact: bool,
        symbol_map: bool,
        bold: bool,
        italic: bool,
    ) -> Option<(fontdb::ID, u16)> {
        let fonts = self.fonts;
        let ch = char::from_u32(code)?;
        if name.eq_ignore_ascii_case("mt extra") && legacy_slot(code) {
            if exact && let Some(g) = legacy_font_glyph(fonts, primary, code) {
                return Some((primary, g));
            }
            let unicode = mt_extra_to_unicode(code)?;
            let fallback = fonts.fallback_for(unicode, bold, italic)?;
            return glyph_of(fonts, fallback, unicode).map(|g| (fallback, g));
        }
        if name.eq_ignore_ascii_case("symbol") {
            // Installed "Symbol" faces differ in cmap (byte-addressed or Unicode),
            // so map to Unicode and let any face draw it.
            let u = symbol_to_unicode(code).unwrap_or(ch);
            if let Some(g) = glyph_of(fonts, primary, u) {
                return Some((primary, g));
            }
            let fb = fonts.fallback_for(u, bold, italic)?;
            return glyph_of(fonts, fb, u).map(|g| (fb, g));
        }
        if exact && symbol_map {
            for c in [code, code | 0xF000, code & 0xFF] {
                if let Some(g) = char::from_u32(c).and_then(|c| glyph_of(fonts, primary, c)) {
                    return Some((primary, g));
                }
            }
        }
        let ch = if !exact && symbol_map {
            map_symbol_char(name, ch).unwrap_or(ch)
        } else {
            ch
        };
        if (ch as u32) < 0x20 {
            return glyph_of(fonts, primary, ' ').map(|g| (primary, g));
        }
        if let Some(g) = glyph_of(fonts, primary, ch) {
            return Some((primary, g));
        }
        let fb = fonts.fallback_for(ch, bold, italic)?;
        glyph_of(fonts, fb, ch).map(|g| (fb, g))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "records one text run with its quad, size, angle and font style"
    )]
    fn record_text(
        &mut self,
        text: String,
        quad: [(f32, f32); 4],
        em_px: f32,
        theta: f32,
        family: &str,
        bold: bool,
        italic: bool,
    ) {
        let (w, h) = (self.pm.width() as f64, self.pm.height() as f64);
        let xs = quad.map(|p| p.0 as f64);
        let ys = quad.map(|p| p.1 as f64);
        let (x0, x1) = (
            xs.iter().copied().fold(f64::MAX, f64::min),
            xs.iter().copied().fold(f64::MIN, f64::max),
        );
        let (y0, y1) = (
            ys.iter().copied().fold(f64::MAX, f64::min),
            ys.iter().copied().fold(f64::MIN, f64::max),
        );
        if !(x0.is_finite() && y0.is_finite() && x1 > 0.0 && y1 > 0.0 && x0 < w && y0 < h) {
            return;
        }
        let c = self.dc.text_color;
        self.texts.push(MetafileText {
            text,
            x: x0 / w,
            y: y0 / h,
            w: (x1 - x0) / w,
            h: (y1 - y0) / h,
            size: em_px as f64 / h,
            angle: (theta as f64).to_degrees(),
            font_family: family.to_string(),
            bold,
            italic,
            color: Rgba {
                r: c.r as f32 / 255.0,
                g: c.g as f32 / 255.0,
                b: c.b as f32 / 255.0,
                a: 1.0,
            },
        });
    }

    fn fill_logical_quad(&mut self, t: &Transform, q: [(f64, f64); 4], color: Rgb) {
        let p = q.map(|(x, y)| map(t, x, y));
        self.fill_px_quad(p, color);
    }

    fn fill_px_quad(&mut self, q: [(f32, f32); 4], color: Rgb) {
        let mut pb = PathBuilder::new();
        pb.move_to(q[0].0, q[0].1);
        for p in &q[1..] {
            pb.line_to(p.0, p.1);
        }
        pb.close();
        let Some(path) = pb.finish() else { return };
        let mask = self.clip_mask();
        let mut paint = solid(color);
        paint.anti_alias = anti_alias_fits(&path, Transform::identity(), 0.0);
        self.pm.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            mask.as_deref(),
        );
    }
}

/// Pixels per font unit for a face at `em_px`.
fn face_scale(fonts: &crate::office::pptx::fonts::FontSystem, id: fontdb::ID, em_px: f32) -> f32 {
    fonts
        .face(id)
        .map(|f| em_px / f.metrics.units_per_em.max(1.0))
        .unwrap_or(0.0)
}

fn glyph_of(
    fonts: &crate::office::pptx::fonts::FontSystem,
    id: fontdb::ID,
    ch: char,
) -> Option<u16> {
    let fd = fonts.face(id)?;
    let face = ttf_parser::Face::parse((*fd.data).as_ref(), fd.index).ok()?;
    face.glyph_index(ch).map(|g| g.0).filter(|g| *g != 0)
}

/// Exact legacy faces can expose Windows Symbol or byte-addressed Unicode
/// cmaps. Probe source slots there before consulting mapped Unicode fallback.
fn legacy_font_glyph(
    fonts: &crate::office::pptx::fonts::FontSystem,
    id: fontdb::ID,
    code: u32,
) -> Option<u16> {
    let fd = fonts.face(id)?;
    let face = ttf_parser::Face::parse((*fd.data).as_ref(), fd.index).ok()?;
    legacy_cmap_glyph(face.tables().cmap?, code)
}

fn legacy_cmap_glyph(cmap: ttf_parser::cmap::Table<'_>, code: u32) -> Option<u16> {
    let byte = match code {
        0x20..=0xFF => code,
        0xF020..=0xF0FF => code - 0xF000,
        _ => return None,
    };
    for candidate in [byte | 0xF000, byte] {
        for table in cmap.subtables {
            let symbol =
                table.platform_id == ttf_parser::PlatformId::Windows && table.encoding_id == 0;
            if (symbol || table.is_unicode())
                && let Some(glyph) = table.glyph_index(candidate).filter(|g| g.0 != 0)
            {
                return Some(glyph.0);
            }
        }
    }
    None
}

/// Horizontal advance in font units.
fn advance(fonts: &crate::office::pptx::fonts::FontSystem, id: fontdb::ID, gid: u16) -> f32 {
    let Some(fd) = fonts.face(id) else { return 0.0 };
    ttf_parser::Face::parse((*fd.data).as_ref(), fd.index)
        .ok()
        .and_then(|f| f.glyph_hor_advance(ttf_parser::GlyphId(gid)))
        .unwrap_or(0) as f32
}

#[cfg(test)]
mod tests;
