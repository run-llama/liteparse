//! Paragraph layout for one text body: font selection, shaping, line breaking,
//! bullets, spacing, alignment and anchoring. Emits glyph runs and text
//! decorations into the scene.
//!
//! Layout happens in "box space": origin at the top-left of the inset text box,
//! x along lines, y down. `TextBox::transform` maps box space to slide space, which
//! also covers vertical text and shape rotation.

use rustybuzz::{Feature, UnicodeBuffer};
use tiny_skia::Transform;

use crate::office::pptx::color::Rgba;
use crate::office::pptx::fonts::{FontSystem, map_symbol_char};
use crate::office::pptx::scene::{
    Glyph, GlyphRun, Item, LineCap, LineJoin, LinkInfo, Paint, PathItem, Rect, Scene, ShadowItem,
    ShadowSource, ShapeIdx, Stroke,
};
use crate::office::pptx::style::{Fill, StyleCtx, read_fill};
use crate::office::pptx::textprops::{BodyProps, BuSize, Bullet, ParaProps, RunProps, Spacing};
use crate::office::pptx::theme::Theme;

pub struct RunInput {
    pub text: String,
    pub props: RunProps,
    /// `a:br`: a line break inside the paragraph.
    pub is_break: bool,
}

pub struct ParaInput {
    pub level: u32,
    pub props: ParaProps,
    pub runs: Vec<RunInput>,
    /// `a:endParaRPr` merged over the level defaults; sizes empty paragraphs.
    pub end_props: RunProps,
}

pub struct TextBox {
    /// Text rectangle in shape-local points (before insets).
    pub rect: crate::office::pptx::geom::TextRect,
    /// Shape-local → slide transform.
    pub transform: Transform,
    pub body: BodyProps,
    pub paras: Vec<ParaInput>,
    /// Text color from the shape style's `fontRef`.
    pub default_color: Option<Rgba>,
    /// Font from the shape style's `fontRef` (`+mj-lt`/`+mn-lt`).
    pub default_font: Option<String>,
    pub shape: Option<ShapeIdx>,
    /// Set by layout when a run's font is missing with no known substitute.
    pub unknown_font: std::cell::Cell<bool>,
    /// Face that replaces such fonts instead of the generic fallback (see `layout`).
    pub narrow: std::cell::Cell<Option<&'static str>>,
}

/// Narrower stand-ins tried, in order, when generic fallbacks overflow a box that
/// PowerPoint sized to its text.
const NARROW_FALLBACKS: [&str; 2] = ["Barlow Semi Condensed", "Liberation Sans Narrow"];

/// PowerPoint for Mac draws a missing Thai-charset font with Browallia New.
const THAI_FALLBACK: &str = "Browallia New";

/// Whether `family` is installed or has a known or user substitute.
fn font_known(env: &LayoutEnv, family: &str) -> bool {
    let (base, _) = crate::office::pptx::fonts::normalize_family(family);
    env.fonts.has_family(&base)
        || env
            .fonts
            .has_family(family.split(',').next().unwrap_or(family).trim())
        || env.fonts.has_user_substitute(family)
        || env.fonts.has_user_substitute(&base)
        || crate::office::pptx::fonts::has_substitute(family)
        || crate::office::pptx::fonts::has_substitute(&base)
}

pub struct LayoutEnv<'a> {
    pub fonts: &'a FontSystem,
    pub theme: &'a Theme,
    pub style: &'a StyleCtx<'a>,
    /// 1-based slide number for `slidenum` fields.
    pub slide_number: u32,
}

/// One shaped cluster: the unit of line breaking.
#[derive(Debug, Clone)]
struct Atom {
    text: String,
    glyphs: Vec<(u16, f64, f64, f64)>, // gid, x_advance, x_offset, y_offset (points)
    advance: f64,
    face: fontdb::ID,
    font_name: String,
    size: f64,
    /// Glyphs are drawn at `size * em` (a scaled substitute font); line height
    /// still follows `size`.
    em: f64,
    run: usize,
    ascent: f64,
    descent: f64,
    kind: AtomKind,
    break_before: bool,
    /// Vertical shift for super/subscript (points, positive = up).
    rise: f64,
    /// Shaped right-to-left (odd bidi level): glyphs inside the atom are in
    /// visual order; atoms themselves are kept in logical order until emission.
    rtl: bool,
    /// Unicode bidi embedding level, for visual reordering of the line.
    level: u8,
    /// Line height follows this atom's font metrics (tall East Asian faces).
    tall: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AtomKind {
    Text,
    Space,
    Tab,
    Break,
    /// A run with no text: zero width, but it can size an otherwise empty line.
    Empty,
}

struct Line {
    atoms: Vec<Atom>,
    /// Advances after resolving tabs, same length as `atoms`.
    advances: Vec<f64>,
    x0: f64,
    avail: f64,
    ascent: f64,
    descent: f64,
    /// Single-spaced line height before `lnSpc` scaling.
    natural: f64,
    first: bool,
    last: bool,
    para: usize,
}

/// PowerPoint's single line height: 1.2 × the font size, except for fonts whose
/// own ascent+descent is much taller (CJK faces), which keep their metrics.
const SINGLE_LINE: f64 = 1.2;
const TALL_FONT_RATIO: f64 = 1.3;
/// Caps on Latin-script ascent/descent (em), above anything an Office font uses.
const MAX_LATIN_ASCENT: f64 = 1.05;
const MAX_LATIN_DESCENT: f64 = 0.3;

fn natural_height(size: f64, ascent: f64, descent: f64, east_asian: bool) -> f64 {
    let metrics = ascent + descent;
    if east_asian && size > 0.0 && metrics / size > TALL_FONT_RATIO {
        metrics
    } else {
        SINGLE_LINE * size
    }
}

const DEFAULT_SIZE: f64 = 18.0;
const SUPERSCRIPT_SCALE: f64 = 2.0 / 3.0;

pub struct LayoutResult {
    /// Per paragraph: level and bullet text.
    pub paragraphs: Vec<(u32, Option<String>)>,
    /// Height of the laid-out text (points).
    pub height: f64,
    /// Height available inside the insets (points).
    pub box_height: f64,
}

/// Lays out a text body at its stored autofit scale (`normAutofit fontScale` /
/// `lnSpcReduction`). PowerPoint applies the saved values when it opens a deck and
/// only recomputes them on edit, so overflowing text is not shrunk here.
///
/// A box with "resize shape to fit text" (`spAutoFit`) was saved at the height of its
/// text in the deck's own fonts. When missing fonts drawn with generic fallbacks make
/// the text taller than that, the fallbacks are too wide for this deck, so the text is
/// laid out again with narrower stand-ins: the first that fits, else the shortest.
/// Line height is left alone: shrinking it to fit made renders worse on average.
pub fn layout(tb: &TextBox, env: &LayoutEnv, scene: &mut Scene) -> LayoutResult {
    let fs = tb.body.font_scale.unwrap_or(1.0);
    let lr = tb.body.ln_spc_reduction.unwrap_or(0.0);
    if tb.body.autofit.as_deref() == Some("shape") {
        let fits = |r: &LayoutResult| r.height <= r.box_height + (0.02 * r.box_height).max(2.0);
        let first = layout_pass(tb, env, &mut Scene::default(), fs, lr, false);
        if !fits(&first) && tb.unknown_font.get() {
            let mut best = (first.height, None);
            for face in NARROW_FALLBACKS {
                tb.narrow.set(Some(face));
                let r = layout_pass(tb, env, &mut Scene::default(), fs, lr, false);
                if r.height < best.0 - 0.01 {
                    best = (r.height, Some(face));
                }
                if fits(&r) {
                    break;
                }
            }
            tb.narrow.set(best.1);
        }
    }
    layout_pass(tb, env, scene, fs, lr, true)
}

/// A run's point size under the stored autofit `fontScale`. PowerPoint rounds the scaled
/// size to a whole point (18 pt at 92.5 % draws at 17 pt, 27 pt at 90 % at 24 pt); line
/// breaks in its thumbnails match only that, not the exact product or a half-point grid.
fn scaled_size(size: Option<f64>, font_scale: f64) -> f64 {
    let size = size.unwrap_or(DEFAULT_SIZE);
    if (font_scale - 1.0).abs() < 1e-9 {
        size
    } else {
        (size * font_scale).round().max(1.0)
    }
}

/// The size a run is drawn at: super- and subscripts (`baseline`) shrink to 2/3.
/// That smaller size, not the run's own, sets the line height: PowerPoint does not
/// grow a line for a raised or lowered run.
fn drawn_size(props: &RunProps, font_scale: f64) -> f64 {
    let size = scaled_size(props.size, font_scale);
    if props.baseline.is_some_and(|b| b.abs() > 1e-6) {
        size * SUPERSCRIPT_SCALE
    } else {
        size
    }
}

fn layout_pass(
    tb: &TextBox,
    env: &LayoutEnv,
    scene: &mut Scene,
    font_scale: f64,
    ln_reduction: f64,
    emit: bool,
) -> LayoutResult {
    let body = &tb.body;
    let l_ins = body.l_ins.unwrap_or(7.2);
    let r_ins = body.r_ins.unwrap_or(7.2);
    let t_ins = body.t_ins.unwrap_or(3.6);
    let b_ins = body.b_ins.unwrap_or(3.6);
    let rect = Rect {
        x: tb.rect.l + l_ins,
        y: tb.rect.t + t_ins,
        w: (tb.rect.r - tb.rect.l - l_ins - r_ins).max(0.0),
        h: (tb.rect.b - tb.rect.t - t_ins - b_ins).max(0.0),
    };
    let vert = body.vert.as_deref().unwrap_or("horz");
    let vertical = matches!(
        vert,
        "vert" | "eaVert" | "vert270" | "wordArtVert" | "mongolianVert" | "wordArtVertRtl"
    );
    let (box_w, box_h) = if vertical {
        (rect.h, rect.w)
    } else {
        (rect.w, rect.h)
    };
    let wrap = body.wrap.as_deref() != Some("none");
    let num_col = body.num_col.unwrap_or(1).max(1) as usize;
    let spc_col = body.spc_col.unwrap_or(0.0);
    let col_w = if num_col > 1 {
        ((box_w - spc_col * (num_col as f64 - 1.0)) / num_col as f64).max(1.0)
    } else {
        box_w
    };

    // Box space → shape-local.
    let box_to_local = if vertical {
        if vert == "vert270" {
            Transform::from_row(0.0, -1.0, 1.0, 0.0, rect.x as f32, (rect.y + rect.h) as f32)
        } else {
            Transform::from_row(0.0, 1.0, -1.0, 0.0, (rect.x + rect.w) as f32, rect.y as f32)
        }
    } else {
        Transform::from_translate(rect.x as f32, rect.y as f32)
    };
    let mut local_tf = box_to_local;
    if let Some(rot) = body.rot.filter(|r| r.abs() > 1e-6) {
        let cx = (tb.rect.l + tb.rect.r) / 2.0;
        let cy = (tb.rect.t + tb.rect.b) / 2.0;
        local_tf =
            local_tf.post_concat(Transform::from_rotate_at(rot as f32, cx as f32, cy as f32));
    }
    let to_slide = local_tf.post_concat(tb.transform);

    let mut lines: Vec<Line> = Vec::new();
    let mut para_spacing: Vec<(f64, f64)> = Vec::new();
    let mut para_out = Vec::new();
    let mut bullets: Vec<Option<(Vec<Atom>, f64, RunProps)>> = Vec::new();
    let mut autonum: Vec<(u32, String, u32)> = Vec::new(); // (level, scheme, counter)

    for (pi, para) in tb.paras.iter().enumerate() {
        let p = &para.props;
        let mar_l = p.mar_l.unwrap_or(0.0);
        let indent = p.indent.unwrap_or(0.0);
        let def_tab = p.def_tab.unwrap_or(72.0).max(1.0);
        let atoms = shape_paragraph(para, tb, env, font_scale);
        let has_text = atoms.iter().any(|a| {
            a.kind == AtomKind::Text || a.kind == AtomKind::Space || a.kind == AtomKind::Tab
        });

        // Bullet.
        let bullet_spec = if has_text {
            p.bullet.clone().unwrap_or(Bullet::None)
        } else {
            Bullet::None
        };
        let first_run_props = para
            .runs
            .iter()
            .find(|r| !r.is_break)
            .map(|r| &r.props)
            .unwrap_or(&para.end_props);
        let bullet_text = match &bullet_spec {
            Bullet::None => {
                autonum.retain(|(lvl, _, _)| *lvl < para.level);
                None
            }
            Bullet::Char(c) => Some(c.clone()),
            Bullet::Blip(_) => Some("•".to_string()),
            Bullet::AutoNum { scheme, start } => {
                autonum.retain(|(lvl, _, _)| *lvl <= para.level);
                let n = match autonum
                    .iter_mut()
                    .find(|(lvl, s, _)| *lvl == para.level && s == scheme)
                {
                    Some(entry) => {
                        entry.2 += 1;
                        entry.2
                    }
                    None => {
                        autonum.retain(|(lvl, _, _)| *lvl != para.level);
                        autonum.push((para.level, scheme.clone(), *start));
                        *start
                    }
                };
                Some(format_autonum(scheme, n))
            }
        };
        let bullet_atoms = bullet_text.as_ref().map(|t| {
            let (atoms, props) = shape_bullet(t, p, first_run_props, tb, env, font_scale);
            let w: f64 = atoms.iter().map(|a| a.advance).sum();
            (atoms, w, props)
        });
        para_out.push((para.level, bullet_text.clone()));

        // Line breaking.
        // Without a bullet, an outdent past the left inset starts the first line at
        // the inset, as in PowerPoint; continuation lines stay at marL.
        let first_x = if bullet_atoms.is_some() {
            mar_l
        } else {
            (mar_l + indent).max(0.0)
        };
        let mut text_first_x = first_x;
        if let Some((_, bw, _)) = &bullet_atoms {
            let bx = mar_l + indent;
            if bx + bw > text_first_x - 0.01 {
                // Bullet wider than the hanging indent: text moves to the next tab stop.
                let after = bx + bw;
                text_first_x = ((after / def_tab).floor() + 1.0) * def_tab;
                if text_first_x - after > def_tab {
                    text_first_x = after;
                }
            }
        }
        let para_lines = break_lines(atoms, text_first_x, mar_l, col_w, wrap, p, def_tab);
        let n = para_lines.len();
        for (li, (atoms, advances, x0)) in para_lines.into_iter().enumerate() {
            let (mut asc, mut desc, mut natural) = (0.0f64, 0.0f64, 0.0f64);
            // Text sets the line height; a line without text takes it from the mark
            // that ends it (its a:br, or the paragraph's end). Empty runs never count.
            for a in atoms
                .iter()
                .filter(|a| a.kind != AtomKind::Break && a.kind != AtomKind::Empty)
            {
                asc = asc.max(a.ascent);
                desc = desc.max(a.descent);
                natural = natural.max(natural_height(a.size, a.ascent, a.descent, a.tall));
            }
            if let Some(b) = atoms
                .last()
                .filter(|a| natural == 0.0 && a.kind == AtomKind::Break)
            {
                asc = b.ascent;
                desc = b.descent;
                natural = natural_height(b.size, b.ascent, b.descent, false);
            }
            if natural == 0.0 {
                let (a, d) = empty_line_metrics(&para.end_props, tb, env, font_scale);
                let size = drawn_size(&para.end_props, font_scale);
                asc = a;
                desc = d;
                natural = natural_height(size, a, d, false);
            }
            lines.push(Line {
                atoms,
                advances,
                x0,
                avail: col_w - x0,
                ascent: asc,
                descent: desc,
                natural,
                first: li == 0,
                last: li + 1 == n,
                para: pi,
            });
        }
        let first_size = drawn_size(first_run_props, font_scale);
        let to_pts = |s: Option<Spacing>| match s {
            Some(Spacing::Pts(v)) => v,
            Some(Spacing::Pct(v)) => v * first_size * 1.2,
            None => 0.0,
        };
        para_spacing.push((to_pts(p.space_before), to_pts(p.space_after)));
        bullets.push(bullet_atoms);
    }

    // Vertical metrics: line pitch and baseline per line.
    let first_last = body.spc_first_last == Some(true);
    let mut ys = Vec::with_capacity(lines.len());
    let mut y = 0.0;
    let mut prev_para: Option<usize> = None;
    for line in &lines {
        let p = &tb.paras[line.para].props;
        if line.first {
            if let Some(pp) = prev_para {
                y += para_spacing[pp].1;
                y += para_spacing[line.para].0;
            } else if first_last {
                y += para_spacing[line.para].0;
            }
        }
        let natural = line.natural;
        let pitch = match p.line_spacing {
            Some(Spacing::Pts(v)) => v,
            Some(Spacing::Pct(v)) => natural * (v - ln_reduction).max(0.1),
            None => natural * (1.0 - ln_reduction).max(0.1),
        };
        // The box's first line scales only its ascent by a percentage above 100 %.
        let pitch = match p.line_spacing {
            Some(Spacing::Pct(v)) if ys.is_empty() && v > 1.0 => {
                ((v - ln_reduction) * line.ascent + line.descent).min(pitch)
            }
            _ => pitch,
        };
        let baseline = y + pitch - line.descent;
        ys.push(baseline);
        y += pitch;
        prev_para = Some(line.para);
    }
    let total_h = y + match prev_para {
        Some(pp) if first_last => para_spacing[pp].1,
        _ => 0.0,
    };

    // Columns: lines flow into the next column once a column is full.
    let mut col_of_line = vec![0usize; lines.len()];
    let mut col_shift = vec![0.0f64; lines.len()];
    if num_col > 1 {
        let mut col = 0usize;
        let mut col_top = 0.0;
        for (i, line) in lines.iter().enumerate() {
            let line_top = ys[i] - line.ascent;
            if ys[i] + line.descent - col_top > box_h && col + 1 < num_col && i > 0 {
                col += 1;
                col_top = line_top;
            }
            col_of_line[i] = col;
            col_shift[i] = col_top;
        }
    }
    let used_h = if num_col > 1 {
        lines
            .iter()
            .enumerate()
            .map(|(i, l)| ys[i] + l.descent - col_shift[i])
            .fold(0.0, f64::max)
    } else {
        total_h
    };
    if !emit {
        return LayoutResult {
            paragraphs: Vec::new(),
            height: used_h,
            box_height: box_h,
        };
    }
    let anchor = body.anchor.as_deref().unwrap_or("t");
    let y_off = match anchor {
        "ctr" => (box_h - used_h) / 2.0,
        "b" => box_h - used_h,
        _ => 0.0,
    };
    // Other WordArt warps (src/warp.rs). Text along a path ignores anchoring too.
    let warp = crate::office::pptx::warp::TextWarp::new(body, box_w, box_h)
        .filter(|_| !vertical && num_col == 1 && used_h > 0.0);
    let line_metrics: Vec<crate::office::pptx::warp::LineMetrics> = (lines.iter().enumerate())
        .map(|(i, l)| (ys[i], l.ascent, l.descent))
        .collect();

    for (i, line) in lines.iter().enumerate() {
        let p = &tb.paras[line.para].props;
        let col_x = col_of_line[i] as f64 * (col_w + spc_col);
        // WordArt "textPlain" stretches each line to the box width and the text
        // block to the box height (anchoring no longer applies).
        // Only legacy WordArt (`fromWordArt="1"`) stretches for textPlain; other
        // warps reshape any text box (a non-WordArt textDeflateTop thumbnail).
        let plain_warp = ((body.warp.as_deref() == Some("textPlain")
            && body.from_word_art == Some(true))
            || warp.as_ref().is_some_and(|w| w.fits()))
            && !vertical
            && used_h > 0.0;
        let baseline = ys[i] - col_shift[i]
            + if plain_warp || warp.is_some() {
                0.0
            } else {
                y_off
            };
        // Width without trailing spaces.
        let mut content_end = line.atoms.len();
        while content_end > 0
            && matches!(
                line.atoms[content_end - 1].kind,
                AtomKind::Space | AtomKind::Break | AtomKind::Empty
            )
        {
            content_end -= 1;
        }
        let w: f64 = line.advances[..content_end].iter().sum();
        let align = p.align.as_deref().unwrap_or("l");
        let slack = (line.avail - w).max(if wrap { 0.0 } else { f64::MIN });
        let mut x = line.x0 + col_x;
        let mut extra_per_space = 0.0;
        match align {
            "ctr" => x += slack / 2.0,
            "r" => x += slack,
            "just" | "dist" | "justLow" | "thaiDist" => {
                let justify = if align == "dist" { true } else { !line.last };
                let spaces = line.atoms[..content_end]
                    .iter()
                    .filter(|a| a.kind == AtomKind::Space)
                    .count();
                if justify && spaces > 0 && slack > 0.0 {
                    extra_per_space = slack / spaces as f64;
                }
            }
            _ => {}
        }
        let line_tf = match plain_warp.then(|| line_ink(&line.atoms, env)).flatten() {
            // Each line's glyph ink (not its line box) fills an equal band of the box.
            Some((ink_asc, ink_desc)) if w > 0.0 => {
                let band = box_h / lines.len() as f64;
                let sx = (box_w / w) as f32;
                let sy = (band / (ink_asc + ink_desc)) as f32;
                let ty = -((baseline - ink_asc) as f32) * sy + (i as f64 * band) as f32;
                Transform::from_row(sx, 0.0, 0.0, sy, -(x as f32) * sx, ty).post_concat(to_slide)
            }
            _ => to_slide,
        };
        let first_item = scene.items.len();
        let line_warp = (warp.as_ref())
            .filter(|wp| !wp.fits() || line_tf != to_slide)
            .and_then(|wp| {
                let algn = crate::office::pptx::warp::Align::from_algn(align);
                wp.line(i, &line_metrics, x, w, algn, to_slide)
            });
        let to_slide = &line_tf;
        if line.first
            && let Some((batoms, bw, bprops)) = &bullets[line.para]
        {
            let indent = p.indent.unwrap_or(0.0);
            // Centered/right text keeps its bullet glued to the text start.
            let bx = if align == "ctr" || align == "r" {
                x - bw
            } else {
                p.mar_l.unwrap_or(0.0) + indent + col_x
            };
            let advances: Vec<f64> = batoms.iter().map(|a| a.advance).collect();
            emit_atoms(
                batoms,
                &advances,
                bx,
                baseline,
                0.0,
                line.para,
                tb,
                env,
                to_slide,
                scene,
                Some(bprops),
            );
        }
        // Mixed-direction lines are drawn in visual order.
        let (vis_atoms, vis_adv): (Vec<Atom>, Vec<f64>) =
            if line.atoms[..content_end].iter().any(|a| a.level > 0) {
                let levels: Vec<u8> = line.atoms[..content_end].iter().map(|a| a.level).collect();
                visual_order(&levels)
                    .into_iter()
                    .map(|k| (line.atoms[k].clone(), line.advances[k]))
                    .unzip()
            } else {
                (
                    line.atoms[..content_end].to_vec(),
                    line.advances[..content_end].to_vec(),
                )
            };
        emit_atoms(
            &vis_atoms,
            &vis_adv,
            x,
            baseline,
            extra_per_space,
            line.para,
            tb,
            env,
            to_slide,
            scene,
            None,
        );
        if let Some(lw) = &line_warp {
            crate::office::pptx::warp::apply(&mut scene.items[first_item..], lw);
        }
    }

    LayoutResult {
        paragraphs: para_out,
        height: total_h,
        box_height: box_h,
    }
}

fn empty_line_metrics(
    props: &RunProps,
    tb: &TextBox,
    env: &LayoutEnv,
    font_scale: f64,
) -> (f64, f64) {
    let size = drawn_size(props, font_scale);
    let family = latin_family(props, tb, env);
    match env
        .fonts
        .resolve(&family, false, false)
        .and_then(|id| env.fonts.face(id))
    {
        Some(fd) => {
            let m = fd.metrics;
            (
                m.ascent / m.units_per_em * size as f32,
                m.descent / m.units_per_em * size as f32,
            )
        }
        None => (size as f32 * 0.9, size as f32 * 0.25),
    }
    .into_f64()
}

trait IntoF64 {
    fn into_f64(self) -> (f64, f64);
}

impl IntoF64 for (f32, f32) {
    fn into_f64(self) -> (f64, f64) {
        (self.0 as f64, self.1 as f64)
    }
}

fn latin_family(props: &RunProps, tb: &TextBox, env: &LayoutEnv) -> String {
    let raw = props
        .latin
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| tb.default_font.clone())
        .unwrap_or_else(|| "+mn-lt".to_string());
    let resolved = env.theme.font_ref(&raw).to_string();
    if resolved.is_empty() {
        "Calibri".to_string()
    } else {
        resolved
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Script {
    Latin,
    EastAsian,
    Complex,
}

fn script_of(ch: char) -> Script {
    let c = ch as u32;
    match c {
        0x1100..=0x11FF
        | 0x2E80..=0x2FFF
        | 0x3000..=0x303F
        | 0x3040..=0x30FF
        | 0x3100..=0x31FF
        | 0x3200..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA960..=0xA97F
        | 0xAC00..=0xD7FF
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFFEF
        | 0x20000..=0x2FFFF => Script::EastAsian,
        0x0590..=0x08FF | 0x0900..=0x0DFF | 0x0E00..=0x0EFF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF => {
            Script::Complex
        }
        _ => Script::Latin,
    }
}

/// Shapes a paragraph into atoms.
fn shape_paragraph(para: &ParaInput, tb: &TextBox, env: &LayoutEnv, font_scale: f64) -> Vec<Atom> {
    // Paragraph text, for line-break opportunities.
    let mut full = String::new();
    let mut run_starts = Vec::new();
    for r in &para.runs {
        run_starts.push(full.len());
        if r.is_break {
            full.push('\n');
        } else {
            full.push_str(&apply_caps(&r.text, r.props.cap.as_deref()));
        }
    }
    // Thai, Lao, Khmer and Burmese write words without spaces: their break
    // opportunities need a dictionary, which ICU's segmenter carries.
    let needs_dictionary = full
        .chars()
        .any(|c| matches!(c as u32, 0x0E00..=0x0EFF | 0x1000..=0x109F | 0x1780..=0x17FF));
    let breaks: std::collections::HashSet<usize> = if needs_dictionary {
        icu_segmenter::LineSegmenter::new_dictionary(Default::default())
            .segment_str(&full)
            .collect()
    } else {
        // PowerPoint breaks at the space before a slash, and not after a slash
        // inside a word (URLs, paths, dates).
        unicode_linebreak::linebreaks(&full)
            .map(|(i, _)| i)
            .filter(|&i| !full[..i].ends_with('/'))
            .chain(full.match_indices(" /").map(|(i, _)| i + 1))
            .collect()
    };
    // Bidi levels per byte of the paragraph; `rtl="1"` paragraphs start right-to-left.
    let base_level = para
        .props
        .rtl
        .filter(|r| *r)
        .map(|_| unicode_bidi::Level::rtl());
    let bidi = unicode_bidi::BidiInfo::new(&full, base_level);

    let mut atoms = Vec::new();
    for (ri, r) in para.runs.iter().enumerate() {
        let base = run_starts[ri];
        if r.is_break {
            let (asc, desc) = empty_line_metrics(&r.props, tb, env, font_scale);
            atoms.push(Atom {
                text: "\n".into(),
                glyphs: vec![],
                advance: 0.0,
                face: fontdb::ID::dummy(),
                font_name: String::new(),
                size: drawn_size(&r.props, font_scale),
                em: 1.0,
                run: ri,
                ascent: asc,
                descent: desc,
                kind: AtomKind::Break,
                break_before: true,
                rise: 0.0,
                rtl: false,
                level: 0,
                tall: false,
            });
            continue;
        }
        let text = apply_caps(&r.text, r.props.cap.as_deref());
        let small = small_caps_flags(&r.text, r.props.cap.as_deref());
        if text.is_empty() {
            let (asc, desc) = empty_line_metrics(&r.props, tb, env, font_scale);
            atoms.push(Atom {
                text: String::new(),
                glyphs: vec![],
                advance: 0.0,
                face: fontdb::ID::dummy(),
                font_name: String::new(),
                size: drawn_size(&r.props, font_scale),
                em: 1.0,
                run: ri,
                ascent: asc,
                descent: desc,
                kind: AtomKind::Empty,
                break_before: false,
                rise: 0.0,
                rtl: false,
                level: 0,
                tall: false,
            });
            continue;
        }
        let mut size = scaled_size(r.props.size, font_scale);
        let mut rise = 0.0;
        if let Some(b) = r.props.baseline.filter(|b| b.abs() > 1e-6) {
            rise = b * size;
            size *= SUPERSCRIPT_SCALE;
        }
        let mut bold = r.props.bold.unwrap_or(false);
        let italic = r.props.italic.unwrap_or(false);
        let mut latin = latin_family(&r.props, tb, env);
        // A missing font with no known substitute: honour its PANOSE weight and serif
        // style. (Weight words in the name are handled by font resolution itself.)
        let (_, name_weight) = crate::office::pptx::fonts::normalize_family(&latin);
        if !font_known(env, &latin) {
            tb.unknown_font.set(true);
            let panose = r.props.latin_panose.as_deref();
            let (heavy, serif) = panose
                .map(crate::office::pptx::fonts::panose_hints)
                .unwrap_or_default();
            bold |= heavy && name_weight.is_none();
            if let Some(face) = tb.narrow.get() {
                latin = face.to_string();
            } else if r.props.latin_thai == Some(true) {
                latin = THAI_FALLBACK.to_string();
            } else if serif {
                latin = "Times New Roman".to_string();
            }
        }
        let ea = r
            .props
            .ea
            .clone()
            .map(|f| env.theme.font_ref(&f).to_string())
            .filter(|s| !s.is_empty());
        let cs = r
            .props
            .cs
            .clone()
            .map(|f| env.theme.font_ref(&f).to_string())
            .filter(|s| !s.is_empty())
            .map(|f| {
                if r.props.cs_thai == Some(true) && !font_known(env, &f) {
                    THAI_FALLBACK.to_string()
                } else {
                    f
                }
            });
        let latin_symbol = map_symbol_char(&latin, 'a').is_some();
        // PowerPoint draws private-use U+F0xx characters with the run's `a:sym` font.
        let sym = r
            .props
            .sym
            .as_deref()
            .map(|f| env.theme.font_ref(f).to_string())
            .filter(|f| map_symbol_char(f, 'a').is_some());

        // Split the run into segments of one face.
        let mut segments: Vec<(usize, String, fontdb::ID, u8, f64, bool)> = Vec::new();
        // Real symbol-font widths (em), keyed by segment start + offset in its text.
        let mut sym_widths: std::collections::HashMap<usize, f64> = Default::default();
        for (k, (off, ch0)) in text.char_indices().enumerate() {
            let small_ch = small.get(k).copied().unwrap_or(false);
            let sym_font = match &sym {
                _ if latin_symbol => Some(latin.as_str()),
                Some(f) if ('\u{F000}'..='\u{F0FF}').contains(&ch0) => Some(f.as_str()),
                _ => None,
            };
            let symbol_font = sym_font.is_some();
            let ch = sym_font
                .and_then(|f| map_symbol_char(f, ch0))
                .unwrap_or(ch0);
            let family = match script_of(ch) {
                Script::EastAsian => ea
                    .clone()
                    .unwrap_or_else(|| env.theme.font_ref("+mn-ea").to_string()),
                Script::Complex => cs
                    .clone()
                    .unwrap_or_else(|| env.theme.font_ref("+mn-cs").to_string()),
                Script::Latin => latin.clone(),
            };
            let family = if family.is_empty() || symbol_font {
                latin.clone()
            } else {
                family
            };
            let primary = if symbol_font {
                None
            } else {
                env.fonts.resolve(&family, bold, italic)
            };
            let id = match primary {
                Some(id) if ch.is_whitespace() || env.fonts.has_glyph(id, ch) => Some(id),
                _ => env
                    .fonts
                    .fallback_weighted(
                        ch,
                        crate::office::pptx::fonts::requested_weight(&family, bold),
                        italic,
                    )
                    .or(primary)
                    .or_else(|| env.fonts.resolve("Carlito", bold, italic)),
            };
            let Some(id) = id else { continue };
            let em = if primary == Some(id) {
                env.fonts.em_scale(&family, id)
            } else {
                1.0
            };
            let level = bidi.levels.get(base + off).map(|l| l.number()).unwrap_or(0);
            let sym_w = sym_font
                .and_then(crate::office::pptx::symbols::SymbolFont::from_name)
                .and_then(|f| f.advance_em(ch0));
            match segments.last_mut() {
                Some((so, s, sid, sl, se, ss))
                    if *sid == id && *sl == level && *se == em && *ss == small_ch =>
                {
                    if let Some(w) = sym_w {
                        sym_widths.insert(*so + s.len(), w);
                    }
                    s.push(ch)
                }
                _ => {
                    if let Some(w) = sym_w {
                        sym_widths.insert(off, w);
                    }
                    segments.push((off, ch.to_string(), id, level, em, small_ch))
                }
            }
        }
        for (seg_off, seg_text, id, level, em, small_seg) in segments {
            let Some(fd) = env.fonts.face(id) else {
                continue;
            };
            let size = if small_seg {
                size * SMALL_CAPS_SCALE
            } else {
                size
            };
            let Some(face) = fd.shaping_face() else {
                continue;
            };
            let scale = size * em / fd.metrics.units_per_em as f64;
            // Substitute fonts may declare inflated Windows metrics (TeX Gyre: 1.7 em);
            // Latin text is placed with typical Office proportions instead.
            let east_asian = seg_text.chars().any(|c| script_of(c) == Script::EastAsian);
            // East Asian lines keep a tall font's own metrics only when that font is
            // the one requested, not a substitute. Even decks set in Meiryo (1.5 em)
            // match PowerPoint renders best at the 1.2 × size default.
            let requested_ea = ea
                .clone()
                .unwrap_or_else(|| env.theme.font_ref("+mn-ea").to_string());
            let tall_ok =
                east_asian && !fd.embedded && fd.family.eq_ignore_ascii_case(requested_ea.trim());
            let (ascent, descent) = if tall_ok {
                (
                    fd.metrics.ascent as f64 * scale,
                    fd.metrics.descent as f64 * scale,
                )
            } else {
                (
                    (fd.metrics.ascent as f64 * scale).min(MAX_LATIN_ASCENT * size),
                    (fd.metrics.descent as f64 * scale).min(MAX_LATIN_DESCENT * size),
                )
            };
            let mut buf = UnicodeBuffer::new();
            buf.push_str(&seg_text);
            buf.guess_segment_properties();
            let kern_on = r.props.kern.map(|k| k > 0.0 && size >= k).unwrap_or(false);
            // A proportional Japanese font (MS PGothic, Meiryo...) replaced by a
            // full-width face: its proportional alternates (palt) restore the widths.
            let east_asian_seg = seg_text.chars().any(|c| script_of(c) == Script::EastAsian);
            let palt = east_asian_seg
                && ea
                    .as_deref()
                    .or(Some(latin.as_str()))
                    .map(is_proportional_cjk)
                    .unwrap_or(false)
                && !env.fonts.has_family(ea.as_deref().unwrap_or(&latin));
            let features = [
                Feature::new(
                    rustybuzz::ttf_parser::Tag::from_bytes(b"kern"),
                    kern_on as u32,
                    ..,
                ),
                Feature::new(rustybuzz::ttf_parser::Tag::from_bytes(b"liga"), 0, ..),
                Feature::new(rustybuzz::ttf_parser::Tag::from_bytes(b"clig"), 0, ..),
                Feature::new(
                    rustybuzz::ttf_parser::Tag::from_bytes(b"palt"),
                    palt as u32,
                    ..,
                ),
            ];
            let rtl = level % 2 == 1;
            buf.set_direction(if rtl {
                rustybuzz::Direction::RightToLeft
            } else {
                rustybuzz::Direction::LeftToRight
            });
            let out = rustybuzz::shape(&face, &features, buf);
            let seg_start = atoms.len();
            let mut seg_clusters: Vec<usize> = Vec::new();
            let infos = out.glyph_infos();
            let poss = out.glyph_positions();
            // Group glyphs by cluster.
            let mut i = 0;
            while i < infos.len() {
                let cluster = infos[i].cluster as usize;
                let mut j = i;
                let mut glyphs = Vec::new();
                let mut adv = 0.0;
                while j < infos.len() && infos[j].cluster as usize == cluster {
                    let g = &poss[j];
                    glyphs.push((
                        infos[j].glyph_id as u16,
                        g.x_advance as f64 * scale,
                        g.x_offset as f64 * scale,
                        g.y_offset as f64 * scale,
                    ));
                    adv += g.x_advance as f64 * scale;
                    j += 1;
                }
                // The cluster's text runs to the next larger cluster offset anywhere in
                // the segment: right-to-left output lists clusters in descending order.
                let next = infos
                    .iter()
                    .map(|g| g.cluster as usize)
                    .filter(|c| *c > cluster)
                    .min()
                    .unwrap_or(seg_text.len());
                let end = next.max(cluster).min(seg_text.len());
                let piece = seg_text.get(cluster..end).unwrap_or("").to_string();
                let first_ch = piece.chars().next().unwrap_or(' ');
                let kind = if first_ch == '\t' {
                    AtomKind::Tab
                } else if first_ch.is_whitespace() {
                    AtomKind::Space
                } else {
                    AtomKind::Text
                };
                // A remapped symbol keeps its original font's width, glyph centred in it.
                if let Some(w) = sym_widths.get(&(seg_off + cluster)) {
                    let target = w * size;
                    let shift = (target - adv) / 2.0;
                    for g in glyphs.iter_mut() {
                        g.2 += shift;
                    }
                    adv = target;
                }
                let spacing = r.props.spacing.unwrap_or(0.0) * font_scale;
                let byte = base + seg_off + cluster;
                atoms.push(Atom {
                    text: piece,
                    glyphs,
                    advance: adv + spacing,
                    face: id,
                    font_name: fd.family.clone(),
                    size,
                    em,
                    run: ri,
                    ascent,
                    descent,
                    kind,
                    break_before: breaks.contains(&byte),
                    rise,
                    rtl,
                    level,
                    tall: tall_ok,
                });
                seg_clusters.push(cluster);
                i = j;
            }
            // Store atoms in logical order. Right-to-left output is usually visual,
            // but not for every face (Arial returns logical order), so sort by the
            // cluster's byte offset instead of assuming either.
            if rtl {
                let mut seg: Vec<(usize, Atom)> = seg_clusters
                    .drain(..)
                    .zip(atoms.drain(seg_start..))
                    .collect();
                seg.sort_by_key(|(c, _)| *c);
                atoms.extend(seg.into_iter().map(|(_, a)| a));
            }
        }
    }
    atoms
}

/// Height of a line's glyph ink above and below the baseline (points), from the
/// glyph outlines; `None` when the line draws nothing.
fn line_ink(atoms: &[Atom], env: &LayoutEnv) -> Option<(f64, f64)> {
    let (mut asc, mut desc) = (f64::MIN, f64::MIN);
    for a in atoms {
        let Some(fd) = env.fonts.face(a.face) else {
            continue;
        };
        let scale = a.size * a.em / fd.metrics.units_per_em as f64;
        for g in &a.glyphs {
            if let Some(b) = env.fonts.outline(a.face, g.0).map(|p| p.bounds()) {
                asc = asc.max(b.bottom() as f64 * scale + a.rise);
                desc = desc.max(-(b.top() as f64) * scale - a.rise);
            }
        }
    }
    (asc + desc > 0.0).then_some((asc, desc))
}

/// Visual order of a line's atoms from their bidi levels (UAX #9 rule L2):
/// from the highest level down to the lowest odd level, reverse every run of
/// atoms at that level or higher.
fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let Some(&max) = levels.iter().max() else {
        return order;
    };
    let min_odd = levels
        .iter()
        .copied()
        .filter(|l| l % 2 == 1)
        .min()
        .unwrap_or(max + 1);
    let mut level = max;
    while level >= min_odd && level > 0 {
        let mut i = 0;
        while i < order.len() {
            if levels[order[i]] >= level {
                let start = i;
                while i < order.len() && levels[order[i]] >= level {
                    i += 1;
                }
                order[start..i].reverse();
            } else {
                i += 1;
            }
        }
        level -= 1;
    }
    order
}

/// Japanese families with proportional kana and punctuation: the "P" variants
/// (ＭＳ Ｐゴシック, MS PMincho, HGP..., BIZ UDP...) and Meiryo.
fn is_proportional_cjk(family: &str) -> bool {
    let f = family.trim();
    let lower = f.to_lowercase();
    f.contains('Ｐ')
        || lower.starts_with("ms p")
        || lower.starts_with("hgp")
        || lower.contains("udp")
        || lower.starts_with("meiryo")
        || f.starts_with("メイリオ")
}

fn apply_caps(text: &str, cap: Option<&str>) -> String {
    match cap {
        Some("all") | Some("small") => text.to_uppercase(),
        _ => text.to_string(),
    }
}

/// Size of small capitals relative to the run's capitals (`cap="small"`):
/// 0.8 matched PowerPoint thumbnails best (0.7 to 0.9 tried).
const SMALL_CAPS_SCALE: f64 = 0.8;

/// For each char of `apply_caps(text, cap)`: whether it is a small capital,
/// i.e. came from a lowercase letter of a `cap="small"` run.
fn small_caps_flags(text: &str, cap: Option<&str>) -> Vec<bool> {
    if cap != Some("small") {
        return Vec::new();
    }
    text.chars()
        .flat_map(|c| {
            let n = c.to_uppercase().count();
            std::iter::repeat_n(c.is_lowercase(), n)
        })
        .collect()
}

/// The bullet font draws the bullet character whatever its script. The text run's
/// PANOSE and Thai flags describe the text's font: kept, they would swap a missing
/// bullet font (Wingdings) for the text's fallback and leave U+F06C unmapped.
fn set_bullet_font(props: &mut RunProps, face: &str) {
    for slot in [
        &mut props.latin,
        &mut props.ea,
        &mut props.cs,
        &mut props.sym,
    ] {
        *slot = Some(face.to_string());
    }
    props.latin_panose = None;
    props.latin_thai = None;
    props.cs_thai = None;
}

fn shape_bullet(
    text: &str,
    p: &ParaProps,
    first: &RunProps,
    tb: &TextBox,
    env: &LayoutEnv,
    font_scale: f64,
) -> (Vec<Atom>, RunProps) {
    let mut props = first.clone();
    let text_size = first.size.unwrap_or(DEFAULT_SIZE);
    props.size = Some(match &p.bu_size {
        Some(BuSize::Pct(v)) => text_size * v,
        Some(BuSize::Pts(v)) => *v / font_scale.max(1e-6),
        _ => text_size,
    });
    if let Some(Some(face)) = &p.bu_font {
        set_bullet_font(&mut props, face);
    }
    if let Some(Some(clr)) = &p.bu_color {
        // `a:buClr` holds a bare colour, like `a:solidFill`; the fill reader only
        // knows fill elements, so pass it as one (else the bullet is transparent).
        let mut fill = (**clr).clone();
        fill.name = "a:solidFill".into();
        props.fill = Some(std::rc::Rc::new(fill));
    }
    props.underline = None;
    props.strike = None;
    props.baseline = None;
    props.hlink_rid = None;
    let para = ParaInput {
        level: 0,
        props: ParaProps::default(),
        runs: vec![RunInput {
            text: text.to_string(),
            props,
            is_break: false,
        }],
        end_props: RunProps::default(),
    };
    let mut atoms = shape_paragraph(&para, tb, env, font_scale);
    for a in &mut atoms {
        a.run = usize::MAX;
    }
    let props = para
        .runs
        .into_iter()
        .next()
        .map(|r| r.props)
        .unwrap_or_default();
    (atoms, props)
}

type LineParts = (Vec<Atom>, Vec<f64>, f64);

fn break_lines(
    atoms: Vec<Atom>,
    first_x: f64,
    mar_l: f64,
    width: f64,
    wrap: bool,
    p: &ParaProps,
    def_tab: f64,
) -> Vec<LineParts> {
    let mut lines: Vec<LineParts> = Vec::new();
    let mut i = 0;
    let n = atoms.len();
    let mut first = true;
    if n == 0 {
        return vec![(Vec::new(), Vec::new(), first_x)];
    }
    while i < n {
        let x0 = if first { first_x } else { mar_l };
        let avail = width - x0;
        let start = i;
        let mut x = 0.0;
        let mut advances = Vec::new();
        let mut last_break: Option<usize> = None;
        let mut end = n;
        let mut consumed_break = false;
        let mut j = i;
        while j < n {
            let a = &atoms[j];
            if a.kind == AtomKind::Break {
                end = j;
                consumed_break = true;
                break;
            }
            if a.break_before && j > start {
                last_break = Some(j);
            }
            let adv = if a.kind == AtomKind::Tab {
                next_tab(x0 + x, p, def_tab) - (x0 + x)
            } else {
                a.advance
            };
            if wrap && a.kind != AtomKind::Space && x + adv > avail + 0.01 && j > start {
                end = match last_break {
                    Some(b) if b > start => b,
                    _ => j,
                };
                break;
            }
            x += adv;
            advances.push(adv);
            j += 1;
        }
        // A hard break stays on the line it ends, with zero width.
        let stop = if consumed_break { end + 1 } else { end };
        let line_atoms: Vec<Atom> = atoms[start..stop].to_vec();
        advances.truncate(end - start);
        if consumed_break {
            advances.push(0.0);
        }
        lines.push((line_atoms, advances, x0));
        i = if consumed_break { end + 1 } else { end };
        // A break at the very end still starts an (empty) new line.
        if consumed_break && i >= n {
            lines.push((Vec::new(), Vec::new(), mar_l));
        }
        first = false;
    }
    lines
}

fn next_tab(pos: f64, p: &ParaProps, def_tab: f64) -> f64 {
    let next = p
        .tabs
        .as_ref()
        .and_then(|tabs| tabs.iter().find(|(t, _)| *t > pos + 0.01).map(|(t, _)| *t))
        .unwrap_or(((pos / def_tab).floor() + 1.0) * def_tab);
    // A hanging indent's margin is also a stop: "\t1. text" in an outdented first
    // line puts "1." at marL, in line with the wrapped lines (PowerPoint for Mac).
    let mar_l = p.mar_l.unwrap_or(0.0);
    if p.indent.is_some_and(|i| i < 0.0) && pos < mar_l - 0.01 && mar_l < next {
        return mar_l;
    }
    next
}

#[expect(
    clippy::too_many_arguments,
    reason = "places one line's atoms and needs its metrics, frame, transform and scene"
)]
fn emit_atoms(
    atoms: &[Atom],
    advances: &[f64],
    x_start: f64,
    baseline: f64,
    extra_per_space: f64,
    para: usize,
    tb: &TextBox,
    env: &LayoutEnv,
    to_slide: &Transform,
    scene: &mut Scene,
    bullet_props: Option<&RunProps>,
) {
    let mut x = x_start;
    let mut i = 0;
    let mut prev_space = false;
    while i < atoms.len() {
        let a = &atoms[i];
        if a.kind == AtomKind::Empty {
            i += 1;
            continue;
        }
        if matches!(a.kind, AtomKind::Space | AtomKind::Tab) {
            x += advances[i]
                + if a.kind == AtomKind::Space {
                    extra_per_space
                } else {
                    0.0
                };
            prev_space = true;
            i += 1;
            continue;
        }
        // Group following atoms of the same face/size/run (inner spaces included) into one run.
        let mut j = i;
        let mut text = String::new();
        let mut gx = 0.0;
        let mut width = 0.0;
        while j < atoms.len() {
            let b = &atoms[j];
            let same = |c: &Atom| {
                c.face == a.face && c.size == a.size && c.run == a.run && c.rise == a.rise
            };
            if matches!(b.kind, AtomKind::Tab | AtomKind::Empty)
                || (b.kind == AtomKind::Text && !same(b))
            {
                break;
            }
            if b.kind == AtomKind::Space {
                // A space only stays inside the run when the run continues after it.
                match atoms.get(j + 1) {
                    Some(nb) if nb.kind == AtomKind::Text && same(nb) => {}
                    _ => break,
                }
            }
            // Visual order is reversed for right-to-left atoms; keep the text logical.
            if b.rtl {
                text.insert_str(0, &b.text);
            } else {
                text.push_str(&b.text);
            }
            gx += advances[j]
                + if b.kind == AtomKind::Space {
                    extra_per_space
                } else {
                    0.0
                };
            if b.kind == AtomKind::Text {
                width = gx;
            }
            j += 1;
        }
        let glyphs = position_glyphs(&atoms[i..j], &advances[i..j], extra_per_space);
        let run_props = if let Some(bp) = bullet_props {
            bp.clone()
        } else {
            tb.paras[para]
                .runs
                .get(a.run)
                .map(|r| r.props.clone())
                .unwrap_or_default()
        };
        let link = run_props
            .hlink_rid
            .as_deref()
            .and_then(|rid| env.style.pkg.rel_by_id(env.style.part, rid))
            .map(|r| r.target);
        let color = run_color(&run_props, tb, env, link.is_some());
        let local = Transform::from_translate(x as f32, (baseline - a.rise) as f32);
        let transform = local.post_concat(*to_slide);
        let text = text.trim_end().to_string();
        // Highlight behind the run.
        if let Some(h) = &run_props.highlight
            && let Some(c) = env.style.color(h, None)
            && let Some(path) = rect_path(0.0, -a.ascent + a.rise, width, a.ascent + a.descent)
            && let Some(path) = path.transform(transform)
        {
            scene.items.push(Item::Path(PathItem {
                path,
                fill: Some(Paint::Solid(c)),
                stroke: None,
                has_curve: false,
                shape: tb.shape,
                from_text: true,
            }));
        }
        let underline = run_props
            .underline
            .as_deref()
            .filter(|u| *u != "none")
            .is_some()
            || link.is_some();
        let strike = run_props
            .strike
            .as_deref()
            .filter(|s| *s != "noStrike")
            .is_some();
        if let Some(uri) = &link
            && let Some(r) =
                transformed_bbox(&transform, 0.0, -a.ascent, width, a.ascent + a.descent)
        {
            scene.links.push(LinkInfo {
                uri: uri.clone(),
                rect: r,
            });
        }
        // Text outline: only an `a:ln` with its own fill draws (a width alone does not).
        let outline = run_props
            .outline
            .as_deref()
            .filter(|ln| ln.child("a:solidFill").is_some() || ln.child("a:gradFill").is_some())
            .and_then(|ln| {
                let unit = Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                };
                crate::office::pptx::style::read_line(Some(ln), None, env.style, unit)
            })
            .and_then(|s| Some((s.paint.representative()?, s.width)));
        let run = GlyphRun {
            bookmarks: None,
            source: None,
            preserve_whitespace: false,
            outline,
            face: a.face,
            font_name: a.font_name.clone(),
            size: a.size * a.em,
            color: color.unwrap_or(Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }),
            glyphs,
            transform,
            text,
            width,
            ascent: a.ascent - a.rise.max(0.0),
            descent: a.descent - (-a.rise).max(0.0),
            fake_bold: run_props.bold == Some(true)
                && (env.fonts.embedded_style(a.face).is_some_and(|(b, _)| !b)
                    || crate::office::pptx::fonts::bold_from_regular(&latin_family(
                        &run_props, tb, env,
                    ))),
            // PowerPoint slants the upright face when the family has no italic.
            fake_italic: run_props.italic == Some(true) && !env.fonts.is_italic(a.face),
            shape: tb.shape,
            paragraph: para,
            hyperlink: link,
            space_before: prev_space,
            drawn: true,
            warp: None,
        };
        let mut text_shadows = Vec::new();
        if let Some(effects) = run_props.effects.as_deref() {
            let shadow = crate::office::pptx::style::read_shadow(Some(effects), None, env.style);
            let glow = crate::office::pptx::effects::read_glow(Some(effects), None, env.style);
            for shadow in [shadow, glow].into_iter().flatten() {
                scene.items.push(Item::Shadow(ShadowItem {
                    source: ShadowSource::Glyphs(run.clone()),
                    shadow,
                }));
                text_shadows.push(shadow);
            }
        }
        scene.items.push(Item::Text(run));
        if (underline || strike)
            && let Some(fd) = env.fonts.face(a.face)
        {
            let scale = a.size * a.em / fd.metrics.units_per_em as f64;
            let thick = (fd.metrics.underline_thickness as f64 * scale).max(0.5);
            let mut lines = Vec::new();
            if underline {
                let (fill, stroke) = underline_paint(&run_props, color, outline, tb, env);
                lines.push((-(fd.metrics.underline_pos as f64) * scale, fill, stroke));
            }
            if strike {
                lines.push((-(fd.metrics.strikeout_pos as f64) * scale, color, None));
            }
            for (ly, fill, stroke) in lines {
                if fill.is_none() && stroke.is_none() {
                    continue;
                }
                let Some(path) = rect_path(0.0, ly - thick / 2.0, width, thick)
                    .and_then(|p| p.transform(transform))
                else {
                    continue;
                };
                for shadow in &text_shadows {
                    scene.items.push(Item::Shadow(ShadowItem {
                        source: ShadowSource::Path(path.clone()),
                        shadow: *shadow,
                    }));
                }
                scene.items.push(Item::Path(PathItem {
                    path,
                    fill: fill.map(Paint::Solid),
                    stroke,
                    has_curve: false,
                    shape: tb.shape,
                    from_text: true,
                }));
            }
        }
        x += gx;
        prev_space = false;
        i = j;
    }
}

/// Fill and outline of a run's underline: `a:uFill` / `a:uLn` when set, else the
/// text's own fill and glyph outline (PowerPoint strokes an outlined run's
/// underline too).
fn underline_paint(
    props: &RunProps,
    text_color: Option<Rgba>,
    text_outline: Option<(Rgba, f64)>,
    tb: &TextBox,
    env: &LayoutEnv,
) -> (Option<Rgba>, Option<Stroke>) {
    let fill = match &props.u_fill {
        Some(f) => {
            let mut p = props.clone();
            p.fill = Some(f.clone());
            run_color(&p, tb, env, false)
        }
        None => text_color,
    };
    let outline = match props.u_ln.as_deref() {
        Some(ln) => {
            let unit = Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            };
            crate::office::pptx::style::read_line(Some(ln), None, env.style, unit)
                .filter(|_| ln.child("a:solidFill").is_some() || ln.child("a:gradFill").is_some())
                .and_then(|s| Some((s.paint.representative()?, s.width)))
        }
        None => text_outline,
    };
    let stroke = outline.map(|(c, w)| Stroke {
        paint: Paint::Solid(c),
        width: w,
        dash: Vec::new(),
        cap: LineCap::Flat,
        join: LineJoin::Miter,
        head: None,
        tail: None,
    });
    (fill.filter(|c| c.a > 0.0), stroke)
}

fn position_glyphs(atoms: &[Atom], advances: &[f64], extra_per_space: f64) -> Vec<Glyph> {
    let mut out = Vec::new();
    let mut x = 0.0;
    for (k, a) in atoms.iter().enumerate() {
        let mut gx = x;
        for (gid, adv, xo, yo) in &a.glyphs {
            if a.kind == AtomKind::Text {
                out.push(Glyph {
                    id: *gid,
                    x: gx + xo,
                    y: -yo,
                });
            }
            gx += adv;
        }
        x += advances[k]
            + if a.kind == AtomKind::Space {
                extra_per_space
            } else {
                0.0
            };
    }
    out
}

fn run_color(props: &RunProps, tb: &TextBox, env: &LayoutEnv, is_link: bool) -> Option<Rgba> {
    if is_link && let Some(c) = env.style.colors.scheme("hlink") {
        return Some(c);
    }
    match &props.fill {
        Some(f) => match read_fill(
            f,
            env.style,
            None,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
        ) {
            Fill::Paint(p) => p.representative(),
            Fill::None => None,
            Fill::Group => tb.default_color,
        },
        None => Some(
            tb.default_color
                .unwrap_or_else(|| env.theme.default_text_color_mapped(env.style.colors)),
        ),
    }
}

pub fn rect_path(x: f64, y: f64, w: f64, h: f64) -> Option<tiny_skia::Path> {
    let r = tiny_skia::Rect::from_xywh(x as f32, y as f32, w.max(0.01) as f32, h.max(0.01) as f32)?;
    Some(tiny_skia::PathBuilder::from_rect(r))
}

pub fn transformed_bbox(t: &Transform, x: f64, y: f64, w: f64, h: f64) -> Option<Rect> {
    let mut pts = [
        tiny_skia::Point::from_xy(x as f32, y as f32),
        tiny_skia::Point::from_xy((x + w) as f32, y as f32),
        tiny_skia::Point::from_xy((x + w) as f32, (y + h) as f32),
        tiny_skia::Point::from_xy(x as f32, (y + h) as f32),
    ];
    t.map_points(&mut pts);
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in pts {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    if !x0.is_finite() {
        return None;
    }
    Some(Rect {
        x: x0 as f64,
        y: y0 as f64,
        w: (x1 - x0) as f64,
        h: (y1 - y0) as f64,
    })
}

pub fn format_autonum(scheme: &str, n: u32) -> String {
    let (body, style) = if let Some(rest) = scheme.strip_prefix("arabic") {
        (n.to_string(), rest)
    } else if let Some(rest) = scheme.strip_prefix("romanUc") {
        (roman(n).to_uppercase(), rest)
    } else if let Some(rest) = scheme.strip_prefix("romanLc") {
        (roman(n), rest)
    } else if let Some(rest) = scheme.strip_prefix("alphaUc") {
        (alpha(n).to_uppercase(), rest)
    } else if let Some(rest) = scheme.strip_prefix("alphaLc") {
        (alpha(n), rest)
    } else if scheme.starts_with("circleNum") {
        (
            char::from_u32(0x2460 + n.saturating_sub(1).min(19))
                .map(|c| c.to_string())
                .unwrap_or_default(),
            "Plain",
        )
    } else {
        (n.to_string(), "Period")
    };
    match style {
        "Period" => format!("{body}."),
        "ParenR" => format!("{body})"),
        "ParenBoth" => format!("({body})"),
        "Minus" => format!("- {body} -"),
        _ => body,
    }
}

fn roman(mut n: u32) -> String {
    const T: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for (v, r) in T {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

fn alpha(n: u32) -> String {
    // a..z, then aa, bb, ... as Office does.
    let n = n.max(1) - 1;
    let ch = (b'a' + (n % 26) as u8) as char;
    std::iter::repeat_n(ch, (n / 26 + 1) as usize).collect()
}

#[cfg(test)]
mod tests;
