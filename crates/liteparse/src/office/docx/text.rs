//! Paragraph line layout the way Word does it: shaping, line breaking at
//! UAX #14 opportunities, tab stops, numbering labels, alignment, and line
//! heights from the requested fonts' Windows metrics. Produces lines whose
//! items are in line-local coordinates: x from the column's left edge, y from
//! the line's top.

use std::rc::Rc;

use crate::office::pptx::color::Rgba;
use crate::office::pptx::fonts::FontSystem;
use crate::office::pptx::scene::{Glyph, GlyphRun, ImageItem, ImageRef, Item, Paint, PathItem};
use rustybuzz::{Feature, UnicodeBuffer};
use tiny_skia::Transform;

use crate::office::docx::model::{
    Anchored, BreakKind, FieldKind, Inline, Paragraph, Picture, Suffix,
};
use crate::office::docx::props::{LineRule, TabKind, TabStop};
use crate::office::docx::styles::{Jc, RunStyle, VertAlign};

mod bidi;

/// Superscript and subscript size, and their baseline shifts, as fractions of
/// the run's size (unconfirmed against Word).
const SCRIPT_SCALE: f64 = 2.0 / 3.0;
const SUPER_RISE: f64 = 0.33;
const SUB_DROP: f64 = 0.08;
const SMALL_CAPS_SCALE: f64 = 0.8;

thread_local! {
    /// Text of each STYLEREF field (see `FieldKind::StyleRef`) on the page being drawn.
    static STYLE_REF_TEXT: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Sets the text each style reference shows on the next header or footer laid out.
pub fn set_style_ref_texts(texts: Vec<String>) {
    STYLE_REF_TEXT.with(|t| *t.borrow_mut() = texts);
}

pub(crate) fn style_ref_text(index: usize) -> String {
    STYLE_REF_TEXT.with(|t| t.borrow().get(index).cloned().unwrap_or_default())
}

/// Page numbers for fields: (page number text, total pages, section pages).
#[derive(Debug, Clone)]
pub struct FieldValues {
    pub page: String,
    /// The page number before the section's format is applied.
    pub page_no: i64,
    pub num_pages: u32,
    pub section_pages: u32,
}

impl Default for FieldValues {
    fn default() -> Self {
        FieldValues {
            page: "1".into(),
            page_no: 1,
            num_pages: 1,
            section_pages: 1,
        }
    }
}

pub struct TextEnv<'a> {
    pub fonts: &'a FontSystem,
    pub default_tab: f64,
    pub fields: FieldValues,
    /// Document grid line pitch (points) when lines snap to it.
    pub grid: Option<f64>,
    /// Active section character tracking, refreshed before section layout.
    pub char_grid: std::cell::Cell<Option<f64>>,
    pub expand_shift_return: bool,
    /// The paragraph mark's font takes part in the last line's height.
    pub mark_in_last_line: bool,
    /// Space between paragraphs is the larger of after and before (Word's
    /// HTML paragraph spacing, off with `doNotUseHTMLParagraphAutoSpacing`).
    pub html_spacing: bool,
    /// Source compatibility permits AutoFit content growth beyond story width.
    pub grow_autofit: bool,
    /// Set while a table cell's paragraphs are laid out.
    pub in_cell: std::cell::Cell<bool>,
    /// Set while header and footer paragraphs are laid out.
    pub in_header: std::cell::Cell<bool>,
    /// `w:adjustLineHeightInTable`: lines in cells snap to the grid.
    pub grid_in_table: bool,
    /// `w:noLeading`: line heights leave out the fonts' external leading.
    pub no_leading: bool,
    /// Word compatibility mode (15 = Word 2013 and later).
    pub compat_mode: u32,
}

/// The resolved horizontal interval and vertical clearance used to break one line.
#[derive(Debug, Clone, Copy)]
pub struct LineMeasure {
    /// Logical paragraph byte boundary, including non-text source atoms.
    pub source_end: usize,
    pub left: f64,
    pub right: f64,
    pub skip: f64,
}

#[derive(Debug, Clone)]
pub struct InlineTextBox {
    pub spec: Rc<crate::office::docx::shapes::ShapeSpec>,
    pub x: f64,
    pub top: f64,
    pub source_slot: usize,
    /// Number of original line text items preceding this inline shape.
    pub text_before: usize,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub height: f64,
    /// Original breaking interval for deterministic continuation replay.
    pub replay: Option<LineMeasure>,
    /// The extra leading an auto line spacing above single adds below the line, in
    /// points. It may run past the bottom of the page (Word documents keep such a
    /// last line on its page).
    pub trailing_leading: f64,
    /// Baseline offset from the line top.
    pub baseline: f64,
    pub items: Vec<Item>,
    /// A page or column break ends this line.
    pub break_after: Option<BreakKind>,
    /// Floating pictures anchored in this line.
    pub anchors: Vec<Anchored>,
    /// Footnotes referenced in this line (ids).
    pub notes: Vec<i64>,
    /// Text of the line, for diagnostics.
    pub text: String,
    /// Room above the line left to clear floating objects.
    pub skip: f64,
    /// Inline shapes with text boxes, at (x, top) in line coordinates: their
    /// text is laid out with the paragraph.
    pub boxes: Vec<InlineTextBox>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Text,
    Space,
    Tab,
    Break(BreakKind),
    Pic,
}

/// One shaped glyph of an atom: (glyph id, advance, x offset, y offset).
type AtomGlyph = (u16, f64, f64, f64);

/// An atom's glyphs; the usual single glyph is stored without a heap allocation.
#[derive(Debug, Clone, Default)]
enum Glyphs {
    #[default]
    Empty,
    One(AtomGlyph),
    Many(Vec<AtomGlyph>),
}

impl Glyphs {
    fn from_vec(mut v: Vec<AtomGlyph>) -> Glyphs {
        match v.len() {
            0 => Glyphs::Empty,
            1 => Glyphs::One(v.pop().unwrap()),
            _ => Glyphs::Many(v),
        }
    }
    fn as_slice(&self) -> &[AtomGlyph] {
        match self {
            Glyphs::Empty => &[],
            Glyphs::One(g) => std::slice::from_ref(g),
            Glyphs::Many(v) => v,
        }
    }
    fn last_mut(&mut self) -> Option<&mut AtomGlyph> {
        match self {
            Glyphs::Empty => None,
            Glyphs::One(g) => Some(g),
            Glyphs::Many(v) => v.last_mut(),
        }
    }
    fn clear(&mut self) {
        *self = Glyphs::Empty;
    }
}

/// An atom's text: up to 22 bytes (nearly every cluster) inline, longer on the heap.
#[derive(Clone)]
enum AtomText {
    Inline(u8, [u8; 22]),
    Heap(Box<str>),
}

impl AtomText {
    fn new(s: &str) -> AtomText {
        if s.len() <= 22 {
            let mut b = [0u8; 22];
            b[..s.len()].copy_from_slice(s.as_bytes());
            AtomText::Inline(s.len() as u8, b)
        } else {
            AtomText::Heap(s.into())
        }
    }
}

impl std::ops::Deref for AtomText {
    type Target = str;
    fn deref(&self) -> &str {
        match self {
            // Built from a `&str` cut at a character boundary.
            AtomText::Inline(n, b) => std::str::from_utf8(&b[..*n as usize]).unwrap_or(""),
            AtomText::Heap(s) => s,
        }
    }
}

impl std::fmt::Debug for AtomText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}

impl PartialEq<&str> for AtomText {
    fn eq(&self, o: &&str) -> bool {
        &**self == *o
    }
}

#[derive(Debug, Clone)]
struct Atom {
    kind: Kind,
    text: AtomText,
    glyphs: Glyphs,
    width: f64,
    /// Drawn face and size, shared by the atoms of one shaped segment.
    m: Rc<AtomFace>,
    style: Rc<RunStyle>,
    ascent: f64,
    descent: f64,
    break_before: bool,
    /// Byte range in the logical paragraph, used by line bidi placement.
    bidi_range: Option<std::ops::Range<usize>>,
    bidi_level: u8,
    /// Picture, anchor, note or tab leader: rare, so kept out of line.
    x: Option<Box<AtomExtra>>,
}

#[derive(Debug, Clone, PartialEq)]
struct AtomFace {
    face: Option<fontdb::ID>,
    /// Size the glyphs are drawn at.
    draw_size: f64,
    /// Drawn-face ascent/descent (text items, decorations).
    face_ascent: f64,
    face_descent: f64,
    rise: f64,
    /// Height the East Asian 1.3 line rule added to this run's ascent (points).
    ea_extra: f64,
}

#[derive(Debug, Clone, Default)]
struct AtomExtra {
    ownership: Vec<(
        std::ops::Range<usize>,
        Option<std::sync::Arc<crate::office::pptx::scene::TextSource>>,
    )>,
    bookmarks: Vec<(
        usize,
        std::sync::Arc<crate::office::pptx::scene::SourceBookmark>,
    )>,
    /// Source interval retained through automatic hyphen reshaping.
    source_range: Option<std::ops::Range<usize>>,
    /// Ruby items are relative to the base baseline and occupy one line atom.
    ruby: Option<Vec<Item>>,
    pic: Option<Picture>,
    anchor: Option<Box<Anchored>>,
    note: Option<i64>,
    /// Width assigned at line finalisation (tabs).
    tab_leader: Option<char>,
    /// Part of a list label or its suffix.
    label: bool,
}

impl Atom {
    fn source_range(&self) -> Option<&std::ops::Range<usize>> {
        self.x
            .as_ref()
            .and_then(|x| x.source_range.as_ref())
            .or(self.bidi_range.as_ref())
    }
    fn face(&self) -> Option<fontdb::ID> {
        self.m.face
    }
    fn draw_size(&self) -> f64 {
        self.m.draw_size
    }
    fn face_ascent(&self) -> f64 {
        self.m.face_ascent
    }
    fn face_descent(&self) -> f64 {
        self.m.face_descent
    }
    fn rise(&self) -> f64 {
        self.m.rise
    }
    fn pic(&self) -> Option<&Picture> {
        self.x.as_ref().and_then(|x| x.pic.as_ref())
    }
    fn ruby(&self) -> Option<&[Item]> {
        self.x.as_ref().and_then(|x| x.ruby.as_deref())
    }
    fn anchor(&self) -> Option<&Anchored> {
        self.x.as_ref().and_then(|x| x.anchor.as_deref())
    }
    fn note(&self) -> Option<i64> {
        self.x.as_ref().and_then(|x| x.note)
    }
    fn tab_leader(&self) -> Option<char> {
        self.x.as_ref().and_then(|x| x.tab_leader)
    }
    fn is_label(&self) -> bool {
        self.x.as_ref().is_some_and(|x| x.label)
    }
    fn extra(&mut self) -> &mut AtomExtra {
        self.x.get_or_insert_with(Default::default)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Script {
    Latin,
    EastAsian,
    Complex,
}

fn script_of(ch: char) -> Script {
    let c = ch as u32;
    match c {
        0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x0E00..=0x0E7F => Script::Complex,
        0x1100..=0x11FF
        | 0x2E80..=0x2FDF
        | 0x3000..=0x30FF
        | 0x3100..=0x31FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xAC00..=0xD7AF
        | 0xF900..=0xFAFF
        | 0xFF00..=0xFFEF
        | 0x20000..=0x2FFFF => Script::EastAsian,
        _ => Script::Latin,
    }
}

/// The family Word uses for a character of a run.
fn family_for(style: &RunStyle, ch: char) -> &str {
    // Symbol encodings retain their explicit family for Unicode translation.
    if style.rtl == Some(true)
        && crate::office::pptx::fonts::map_symbol_char(&style.font, ch).is_none()
    {
        return &style.font_cs;
    }
    match script_of(ch) {
        Script::EastAsian => &style.font_ea,
        Script::Complex => &style.font_cs,
        // Shared punctuation follows the east asian font when the run hints so.
        Script::Latin if style.hint_ea && !ch.is_ascii() => &style.font_ea,
        Script::Latin => &style.font,
    }
}

fn draw_size(style: &RunStyle) -> (f64, f64) {
    let size = style.size;
    match style.vert_align {
        VertAlign::Super => (size * SCRIPT_SCALE, size * SUPER_RISE + style.position),
        VertAlign::Sub => (size * SCRIPT_SCALE, -size * SUB_DROP + style.position),
        VertAlign::Baseline => (size, style.position),
    }
}

/// Lays out one paragraph at `width` (the column or cell text width).
pub fn layout_paragraph(p: &Paragraph, width: f64, env: &TextEnv) -> Vec<Line> {
    layout_paragraph_wrapped(p, width, env, &WrapIn::default())
}

/// Floating objects a paragraph's lines wrap around: rectangles
/// `(top, bottom, left, right)`, with y from the paragraph's first line top
/// and x from the column's left edge.
#[derive(Debug, Clone, Default)]
pub struct WrapIn {
    pub rects: Vec<(f64, f64, f64, f64)>,
    /// Per rect: whether it comes from a tight or through (contour) wrap. Missing
    /// entries count as square.
    pub contour: Vec<bool>,
    /// Height assumed for a line before it is laid out.
    pub est_h: f64,
    /// Narrowest gap beside an object that takes text.
    pub min_gap: f64,
}

/// Per-line placement beside floating objects while breaking lines.
struct WrapFlow<'w> {
    wrap: &'w WrapIn,
    prefix: &'w [Line],
    line_no: usize,
    source_end: usize,
    left: f64,
    /// Top of the next line, from the paragraph's first line top.
    y: f64,
    last_h: f64,
    /// Space the line being built skips to clear objects.
    skip: f64,
    /// Right edge of the line being built.
    edge: f64,
    /// The span the line being built asked for, when objects narrowed it.
    narrowed: Option<(f64, f64)>,
    /// Whether a tight or through wrapped object narrowed the line being built.
    beside_contour: bool,
}

impl WrapFlow<'_> {
    /// Starts a line whose text would span `lo..hi`: moves it into the widest
    /// gap beside the objects it meets, or below them when no gap is wide enough.
    fn start(&mut self, lo: f64, hi: f64) -> LineBuild {
        self.start_needing(lo, hi, 0.0)
    }

    /// Restarts the line being built in a gap at least `need` wide. Word does not
    /// break a word beside an object: a first word wider than the gap moves the
    /// line below it (Word for Mac probes: 27 to 33 pt gaps beside a picture).
    /// None when objects did not narrow the line.
    fn restart_for(&mut self, need: f64) -> Option<LineBuild> {
        let (lo, hi) = self.narrowed?;
        self.y -= self.skip;
        Some(self.start_needing(lo, hi, need))
    }

    fn start_needing(&mut self, lo: f64, hi: f64, need: f64) -> LineBuild {
        if let Some(measure) = self.prefix.get(self.line_no).and_then(|l| l.replay) {
            self.left = measure.left;
            self.edge = measure.right;
            self.skip = measure.skip;
            self.y += measure.skip;
            self.narrowed = None;
            self.beside_contour = false;
            return LineBuild::new(measure.left);
        }
        // Remaining lines start at the current column's wrapping origin.
        if self.line_no == self.prefix.len() && need == 0.0 {
            self.y = 0.0;
            self.last_h = self.wrap.est_h;
        }
        let h = self.last_h.max(1.0);
        let mut y = self.y;
        let mut span = (lo, hi);
        let mut beside_contour = false;
        let attempts = self.wrap.rects.len().saturating_add(1).max(32);
        for _ in 0..attempts {
            let hit_ids: Vec<usize> = (0..self.wrap.rects.len())
                .filter(|&k| {
                    let r = &self.wrap.rects[k];
                    y < r.1 - 0.01 && y + h > r.0 + 0.01 && r.2 < hi && r.3 > lo
                })
                .collect();
            let hits: Vec<&(f64, f64, f64, f64)> =
                hit_ids.iter().map(|&k| &self.wrap.rects[k]).collect();
            if hits.is_empty() {
                span = (lo, hi);
                break;
            }
            beside_contour = hit_ids
                .iter()
                .any(|&k| self.wrap.contour.get(k).copied().unwrap_or(false));
            let mut segs = vec![(lo, hi)];
            for r in &hits {
                segs = segs
                    .into_iter()
                    .flat_map(|(a, b)| [(a, r.2.min(b)), (r.3.max(a), b)])
                    .filter(|(a, b)| b - a > 0.5)
                    .collect();
            }
            // A gap lying wholly left of the column (a hanging indent's outdent left
            // of a float at the column edge) takes no text.
            segs.retain(|(_, b)| *b > 0.5);
            let best = segs
                .iter()
                .copied()
                .max_by(|x, y| (x.1 - x.0).total_cmp(&(y.1 - y.0)));
            if let Some(b) = best.filter(|b| b.1 - b.0 >= self.wrap.min_gap.max(need)) {
                span = b;
                break;
            }
            y = hits.iter().map(|r| r.1).fold(f64::INFINITY, f64::min);
            span = (lo, hi);
        }
        self.skip = y - self.y;
        self.y = y;
        self.edge = span.1;
        self.left = span.0;
        self.narrowed = (span != (lo, hi)).then_some((lo, hi));
        self.beside_contour = self.narrowed.is_some() && beside_contour;
        LineBuild::new(span.0)
    }

    /// Records a finished line's skip and advances below it.
    fn finish(&mut self, mut line: Line) -> Line {
        line.skip = self.skip;
        let source_end = line.replay.map_or(0, |m| m.source_end).max(self.source_end);
        self.source_end = source_end;
        line.replay = Some(LineMeasure {
            source_end,
            left: self.left,
            right: self.edge,
            skip: self.skip,
        });
        self.line_no += 1;
        self.y += line.height;
        self.last_h = line.height;
        line
    }
}

/// [`layout_paragraph`] with lines wrapped around floating objects.
pub fn layout_paragraph_wrapped(
    p: &Paragraph,
    width: f64,
    env: &TextEnv,
    wrap: &WrapIn,
) -> Vec<Line> {
    layout_paragraph_resuming(p, width, env, &[], wrap)
}

/// Width of a word's final atom when it is a comma or full stop Word compresses at
/// a line end: the ideographic and full-width forms (and their half-width forms).
/// Whether a character is a space that may hang past the line's edge. En and em
/// spaces take their width there like text (Word for Mac: a form field of five en
/// spaces that does not fit wraps to the next line).
fn hanging_space(ch: char) -> bool {
    ch.is_whitespace() && !matches!(ch, '\u{a0}' | '\u{2002}' | '\u{2003}')
}

fn hanging_punct_width(word: &[Atom]) -> f64 {
    match word.last() {
        Some(a)
            if a.kind == Kind::Text
                && !a.text.is_empty()
                && a.text
                    .chars()
                    .all(|c| matches!(c, '、' | '。' | '，' | '．' | '､' | '｡' | '：')) =>
        {
            a.width
        }
        _ => 0.0,
    }
}

/// Replays consumed line intervals with the full paragraph's shaping context,
/// then breaks the remaining atoms at the current column width and wrap origin.
pub fn layout_paragraph_resuming(
    p: &Paragraph,
    width: f64,
    env: &TextEnv,
    prefix: &[Line],
    tail_wrap: &WrapIn,
) -> Vec<Line> {
    with_paragraph_atoms(p, width, env, |atoms, label, context, empty_bookmarks| {
        let mut lines = break_lines(
            p,
            atoms,
            label,
            width,
            env,
            WrapResume {
                wrap: tail_wrap,
                prefix,
            },
            Some(context),
        );
        if let Some(line) = lines.first_mut() {
            let x = match p.style.jc {
                Jc::Center => (p.style.left + width - p.style.right) / 2.0,
                Jc::Right => width - p.style.right,
                _ => p.style.left + p.style.first,
            };
            for marker in empty_bookmarks {
                line.items
                    .push(Item::Bookmark(crate::office::pptx::scene::BookmarkItem {
                        source: marker,
                        point: tiny_skia::Point::from_xy(x as f32, line.baseline as f32),
                        reading_key: 0,
                    }));
            }
        }
        lines
    })
}

type LabelAtoms = Option<(Vec<Atom>, Option<Atom>)>;

type Bookmark = std::sync::Arc<crate::office::pptx::scene::SourceBookmark>;

/// Coalesce the same text runs as parsing does, while retaining source byte boundaries.
type InlineBookmarkMap = std::collections::HashMap<usize, Vec<(usize, Bookmark)>>;
type InlineOwners = std::collections::HashMap<usize, Vec<(usize, Rc<RunStyle>)>>;
type BookmarkInlines<'a> = (
    std::borrow::Cow<'a, [Inline]>,
    InlineBookmarkMap,
    Vec<Bookmark>,
    InlineOwners,
);

fn bookmark_inlines(p: &Paragraph) -> BookmarkInlines<'_> {
    if !p.inlines.iter().any(|inline| {
        matches!(inline, Inline::Bookmark(_))
            || matches!(inline, Inline::Text(_, style) if internal_source(style))
    }) {
        return (
            std::borrow::Cow::Borrowed(&p.inlines),
            Default::default(),
            Vec::new(),
            Default::default(),
        );
    }
    let mut inlines = Vec::new();
    let mut owners: InlineOwners = Default::default();
    let mut boundaries: std::collections::HashMap<usize, Vec<(usize, Bookmark)>> =
        Default::default();
    let mut pending = Vec::new();
    for inline in &p.inlines {
        if let Inline::Bookmark(marker) = inline {
            pending.push(marker.clone());
            continue;
        }
        let last_index = inlines.len().saturating_sub(1);
        if let (Some(Inline::Text(previous, previous_style)), Inline::Text(text, style)) =
            (inlines.last_mut(), inline)
            && ((!pending.is_empty() && Rc::ptr_eq(previous_style, style))
                || internal_shaping_equivalent(previous_style, style))
        {
            if previous_style.source != style.source || owners.contains_key(&last_index) {
                let spans = owners
                    .entry(last_index)
                    .or_insert_with(|| vec![(previous.len(), previous_style.clone())]);
                if let Some((end, _)) = spans
                    .last_mut()
                    .filter(|(_, owner)| owner.source == style.source)
                {
                    *end = previous.len() + text.len();
                } else {
                    spans.push((previous.len() + text.len(), style.clone()));
                }
            }
            boundaries
                .entry(last_index)
                .or_default()
                .extend(pending.drain(..).map(|marker| (previous.len(), marker)));
            previous.push_str(text);
            continue;
        }
        boundaries
            .entry(inlines.len())
            .or_default()
            .extend(pending.drain(..).map(|marker| (0, marker)));
        inlines.push(inline.clone());
    }
    if let Some(last) = inlines.last() {
        let offset = if let Inline::Text(text, _) = last {
            text.len()
        } else {
            usize::MAX
        };
        boundaries
            .entry(inlines.len() - 1)
            .or_default()
            .extend(pending.drain(..).map(|marker| (offset, marker)));
    }
    (
        std::borrow::Cow::Owned(inlines),
        boundaries,
        pending,
        owners,
    )
}

fn with_paragraph_atoms<T>(
    p: &Paragraph,
    width: f64,
    env: &TextEnv,
    consume: impl FnOnce(
        Vec<Atom>,
        LabelAtoms,
        &bidi::Context<'_>,
        Vec<std::sync::Arc<crate::office::pptx::scene::SourceBookmark>>,
    ) -> T,
) -> T {
    // About one atom per character: reserved up front, so a long paragraph's
    // vector does not overshoot by doubling.
    let chars: usize = p
        .inlines
        .iter()
        .map(|i| match i {
            Inline::Text(t, _) => t.chars().count(),
            _ => 1,
        })
        .sum();
    let analysis = bidi::Analysis::new(p, env);
    let context = analysis.resolve(p.style.bidi);
    let mut atoms = Vec::with_capacity(chars);
    let mut full = String::new();
    let mut offsets = Vec::with_capacity(chars);
    let label_atoms = p.num.as_ref().map(|n| {
        let mut a = Vec::new();
        let mut scratch = String::new();
        let mut offs = Vec::new();
        if let Some(picture) = &n.picture {
            // Picture bullets follow the resolved label size. The 0.8-em height
            // is an approximation calibrated against public Word references;
            // the source dimensions retain the image's aspect ratio.
            let mut picture = (**picture).clone();
            let height = n.style.size * 0.8;
            let scale = height / picture.h;
            picture.w *= scale;
            picture.h = height;
            if let Some(spec) = &mut picture.shape {
                let transform = tiny_skia::Transform::from_scale(scale as f32, scale as f32);
                for item in &mut Rc::make_mut(spec).items {
                    crate::office::docx::layout::transform_item(item, transform);
                    if let Item::Path(path) = item
                        && let Some(stroke) = &mut path.stroke
                    {
                        stroke.width *= scale;
                    }
                }
            }
            let (_, descent) = font_line(env, &n.style, &n.style.font);
            for _ in 0..n.picture_count {
                a.push(Atom {
                    width: picture.w,
                    ascent: picture.h,
                    descent,
                    x: Some(Box::new(AtomExtra {
                        pic: Some(picture.clone()),
                        ..Default::default()
                    })),
                    ..blank(Kind::Pic, &n.style)
                });
            }
        } else if p.style.bidi {
            let analysis = bidi::Analysis::label(&n.text, &n.style);
            let context = analysis.resolve(true);
            bidi::shape(
                &n.text,
                &n.style,
                env,
                &mut a,
                &mut scratch,
                &mut offs,
                Some(&context),
            );
            let mut label = LineBuild::new(0.0);
            for (i, mut atom) in a.into_iter().enumerate() {
                atom.bidi_range = Some(offs[i]..offs.get(i + 1).copied().unwrap_or(scratch.len()));
                atom.bidi_level = context.level(offs[i]).number();
                label.push(atom);
            }
            context.place(&mut label);
            a = label.atoms;
            // Resolved label levels remain separate from the body's byte offsets.
            for atom in &mut a {
                atom.bidi_range = None;
            }
        } else {
            shape_text(&n.text, &n.style, env, &mut a, &mut scratch, &mut offs);
        }
        let mut suffix = match n.suffix {
            Suffix::Tab => Some(tab_atom(&n.style, env)),
            Suffix::Space => {
                let mut sp = Vec::new();
                shape_text(" ", &n.style, env, &mut sp, &mut scratch, &mut offs);
                sp.into_iter().next()
            }
            Suffix::Nothing => None,
        };
        for x in a.iter_mut().chain(suffix.iter_mut()) {
            x.extra().label = true;
        }
        (a, suffix)
    });
    let (inlines, mut boundaries, mut empty_bookmarks, mut owners) = bookmark_inlines(p);
    for (inline_index, inline) in inlines.iter().enumerate() {
        let first_atom = atoms.len();
        let text_start = full.len();
        match inline {
            Inline::Bookmark(_) => unreachable!(),
            Inline::Text(t, s) => bidi::shape(
                t,
                s,
                env,
                &mut atoms,
                &mut full,
                &mut offsets,
                Some(&context),
            ),
            Inline::Ruby(r) => {
                let a = ruby_atom(r, env);
                offsets.push(full.len());
                full.push_str(&a.text);
                atoms.push(a);
            }
            Inline::Field(k, s) => {
                let text = match k {
                    FieldKind::Page(None) => env.fields.page.clone(),
                    FieldKind::Page(Some(f)) => {
                        crate::office::docx::numbering::format_number(f, env.fields.page_no)
                    }
                    FieldKind::NumPages(f) => crate::office::docx::numbering::format_number(
                        f.unwrap_or("decimal"),
                        env.fields.num_pages as i64,
                    ),
                    FieldKind::SectionPages(f) => crate::office::docx::numbering::format_number(
                        f.unwrap_or("decimal"),
                        env.fields.section_pages as i64,
                    ),
                    FieldKind::StyleRef(i) => style_ref_text(*i),
                };
                bidi::shape(
                    &text,
                    s,
                    env,
                    &mut atoms,
                    &mut full,
                    &mut offsets,
                    Some(&context),
                )
            }
            Inline::Tab(s) => {
                offsets.push(full.len());
                full.push('\t');
                atoms.push(tab_atom(s, env));
            }
            Inline::Break(k, s) => {
                offsets.push(full.len());
                full.push('\n');
                let (asc, desc) = font_line(env, s, &s.font);
                atoms.push(Atom {
                    ascent: asc,
                    descent: desc,
                    ..blank(Kind::Break(*k), s)
                });
            }
            Inline::Picture(pic, s) => {
                offsets.push(full.len());
                full.push('\u{FFFC}');
                let (_, desc) = font_line(env, s, &s.font);
                // A raised or lowered run (`w:position`) moves its picture off the
                // baseline: Equation Editor pictures sit lowered so their text lines up.
                let rise = s.position;
                let mut atom = Atom {
                    width: pic.w,
                    ascent: (pic.h + rise).max(0.0),
                    descent: desc.max(-rise),
                    x: Some(Box::new(AtomExtra {
                        pic: Some(pic.clone()),
                        ..Default::default()
                    })),
                    ..blank(Kind::Pic, s)
                };
                if rise != 0.0 {
                    atom.m = Rc::new(AtomFace {
                        rise,
                        ..(*atom.m).clone()
                    });
                }
                atoms.push(atom);
            }
            Inline::ContinuationSeparator(s) => {
                let Inline::Picture(pic, _) = crate::office::docx::parse::note_separator(s, width)
                else {
                    unreachable!()
                };
                offsets.push(full.len());
                full.push('\u{FFFC}');
                let (_, desc) = font_line(env, s, &s.font);
                atoms.push(Atom {
                    width: pic.w,
                    ascent: pic.h,
                    descent: desc,
                    x: Some(Box::new(AtomExtra {
                        pic: Some(pic.clone()),
                        ..Default::default()
                    })),
                    ..blank(Kind::Pic, s)
                });
            }
            Inline::Anchor(a) => {
                offsets.push(full.len());
                full.push('\u{200B}');
                atoms.push(Atom {
                    x: Some(Box::new(AtomExtra {
                        anchor: Some(a.clone()),
                        ..Default::default()
                    })),
                    ..blank(Kind::Text, &p.mark)
                });
            }
            Inline::Note(id) => {
                offsets.push(full.len());
                full.push('\u{200B}');
                atoms.push(Atom {
                    x: Some(Box::new(AtomExtra {
                        note: Some(*id),
                        ..Default::default()
                    })),
                    ..blank(Kind::Text, &p.mark)
                });
            }
        }
        if let (Some(ownership), Inline::Text(text, style)) = (owners.remove(&inline_index), inline)
        {
            let (mut start, mut source_start) = (0, 0);
            let ranges: Vec<_> = ownership
                .into_iter()
                .map(|(end, owner)| {
                    let mapped = if style.caps && style.symbol.is_none() {
                        start + text[source_start..end].to_uppercase().len()
                    } else {
                        end
                    };
                    let range = start..mapped;
                    start = mapped;
                    source_start = end;
                    (range, owner)
                })
                .collect();
            let mut owner_index = 0;
            for k in first_atom..atoms.len() {
                let lo = offsets[k] - text_start;
                let hi = offsets.get(k + 1).copied().unwrap_or(full.len()) - text_start;
                while owner_index + 1 < ranges.len() && ranges[owner_index].0.end <= lo {
                    owner_index += 1;
                }
                atoms[k].style = ranges[owner_index].1.clone();
                let mut fragments = Vec::new();
                for (range, owner) in &ranges[owner_index..] {
                    if range.start >= hi {
                        break;
                    }
                    if range.end > lo {
                        fragments.push((
                            range.start.max(lo) - lo..range.end.min(hi) - lo,
                            owner.source.clone(),
                        ));
                    }
                }
                if fragments.len() > 1 {
                    atoms[k].extra().ownership = fragments;
                }
            }
        }
        for (offset, marker) in boundaries.remove(&inline_index).unwrap_or_default() {
            if atoms.len() == first_atom {
                empty_bookmarks.push(marker);
                continue;
            }
            let byte = match inline {
                Inline::Text(text, style) => {
                    let prefix = &text[..offset.min(text.len())];
                    text_start
                        + if style.caps && style.symbol.is_none() {
                            prefix.to_uppercase().len()
                        } else {
                            prefix.len()
                        }
                }
                _ if offset == usize::MAX => full.len(),
                _ => text_start,
            }
            .min(full.len());
            let index = offsets[first_atom..]
                .partition_point(|start| *start <= byte)
                .saturating_sub(1)
                + first_atom;
            let utf16 = full[offsets[index]..byte].encode_utf16().count();
            atoms[index].extra().bookmarks.push((utf16, marker));
        }
    }
    // Break opportunities over the whole paragraph text.
    let mut breaks: std::collections::HashSet<usize> = unicode_linebreak::linebreaks(&full)
        .map(|(i, _)| i)
        .filter(|&i| word_break_ok(&full, i))
        .collect();
    // A hyphen-minus after a word may end a line before a digit (Word: "9月-" then
    // "2017年"); UAX #14 keeps it with the number as a minus sign.
    let mut prev: Option<char> = None;
    for (i, c) in full.char_indices() {
        if c == '-' && prev.is_some_and(|p| !p.is_whitespace()) {
            let after = i + 1;
            if full[after..]
                .chars()
                .next()
                .is_some_and(|n| n.is_ascii_digit())
            {
                breaks.insert(after);
            }
        }
        prev = Some(c);
    }
    for (a, off) in atoms.iter_mut().zip(&offsets) {
        a.break_before = breaks.contains(off) && *off > 0;
    }
    for (i, a) in atoms.iter_mut().enumerate() {
        a.bidi_range = Some(offsets[i]..offsets.get(i + 1).copied().unwrap_or(full.len()));
        a.bidi_level = context.level(offsets[i]).number();
    }
    auto_space(&p.style, &mut atoms);
    if crate::office::docx::metrics::balance_spaces() {
        balance_spaces(&mut atoms, env.char_grid.get().unwrap_or(0.0));
    }
    fit_text(&mut atoms);
    consume(atoms, label_atoms, &context, empty_bookmarks)
}

/// Width constraints from the same shaping and break opportunities used for paint.
#[derive(Clone, Copy, Default)]
pub(crate) struct ContentWidths {
    pub minimum: f64,
    pub maximum: f64,
    pub atomic: f64,
    /// Minimum extent retained when inline raster paint can be clipped.
    pub unclippable: f64,
}

pub(crate) fn content_widths(p: &Paragraph, width: f64, env: &TextEnv) -> ContentWidths {
    with_paragraph_atoms(p, width, env, |atoms, label, _, _| {
        let st = &p.style;
        let hanging = (st.first < 0.0).then_some(st.left);
        let mut line = LineBuild::new(st.left + st.first);
        if let Some((label, suffix)) = label {
            push_label(p, &mut line, label, suffix, hanging, env);
        }
        let mut word = line.x;
        let mut trimmed_word = word;
        let mut result = ContentWidths::default();
        let mut first_visible = true;
        for mut atom in atoms {
            if let Kind::Break(kind) = atom.kind {
                if env.in_cell.get() && kind != BreakKind::Line {
                    continue;
                }
                line.settle();
                result.minimum = result.minimum.max(trimmed_word + st.right);
                result.maximum = result.maximum.max(line.x + st.right);
                line = LineBuild::new(st.left);
                first_visible = false;
                word = st.left;
                trimmed_word = word;
                continue;
            }
            if (atom.break_before && !first_visible) || atom.kind == Kind::Tab {
                result.minimum = result.minimum.max(trimmed_word + st.right);
                word = st.left;
                trimmed_word = word;
            }
            if atom.kind == Kind::Tab {
                line.settle();
                let (pos, kind, _) = next_tab(line.x, &st.tabs, hanging, env.default_tab);
                match kind {
                    TabKind::Left | TabKind::Bar | TabKind::Clear => {
                        atom.width = (pos - line.x).max(0.0);
                        line.push(atom);
                    }
                    _ => {
                        atom.width = 0.0;
                        let index = line.atoms.len();
                        line.push(atom);
                        line.pending = Some(Pending {
                            kind,
                            pos,
                            atom: index,
                            start: line.x,
                        });
                    }
                }
                continue;
            }
            word += atom.width;
            if atom.kind != Kind::Space {
                trimmed_word = word;
                let first = if first_visible { st.first } else { 0.0 };
                let extent = atom.width + st.left + st.right + first;
                result.atomic = result.atomic.max(extent);
                if !atom
                    .pic()
                    .is_some_and(|p| p.part.is_some() && p.shape.is_none() && p.graphic.is_none())
                {
                    result.unclippable = result.unclippable.max(extent);
                }
                if atom.width > 0.0 {
                    first_visible = false;
                }
            }
            line.push(atom);
        }
        line.settle();
        result.minimum = result
            .minimum
            .max(trimmed_word + st.right)
            .max(result.atomic)
            .max(0.0);
        result.maximum = result.maximum.max(line.x + st.right).max(result.minimum);
        result
    })
}

/// Word keeps a slash glued to the text after it (paths, URLs, dates), so the
/// break UAX #14 allows after `/` before a letter or digit is dropped.
fn word_break_ok(full: &str, i: usize) -> bool {
    let before = full[..i].chars().next_back();
    let after = full[i..].chars().next();
    !matches!((before, after), (Some('/'), Some(c)) if c.is_alphanumeric() && script_of(c) == Script::Latin)
}

/// Drawing-equivalent styles share one output run. Language and grid exemptions
/// have already affected shaping and atom positions before this comparison.
fn internal_source(style: &RunStyle) -> bool {
    style
        .source
        .as_ref()
        .and_then(|source| source.link.as_ref())
        .is_some_and(|link| {
            matches!(
                link.target,
                crate::office::pptx::scene::SourceLinkTarget::Bookmark { .. }
            )
        })
}

fn internal_shaping_equivalent(a: &Rc<RunStyle>, b: &Rc<RunStyle>) -> bool {
    Rc::ptr_eq(
        a.source_style.as_ref().unwrap_or(a),
        b.source_style.as_ref().unwrap_or(b),
    ) && internal_paint_equivalent(a, b)
}

// Formatting decisions use the source style from before internal-link attribution.
// Retained Rc identity separately controls which source text is shaped together.
fn original_format_equal(a: &RunStyle, b: &RunStyle) -> bool {
    if a.source_style.is_none() && b.source_style.is_none() {
        return a == b;
    }
    let original = |style: &RunStyle| {
        let mut value = style.clone();
        if let Some(base) = &style.source_style {
            value.source = base.source.clone();
        }
        value.source_style = None;
        value
    };
    original(a) == original(b)
}

fn internal_paint_equivalent(a: &RunStyle, b: &RunStyle) -> bool {
    (internal_source(a) || internal_source(b)) && original_format_equal(a, b)
}

fn same_format(a: &RunStyle, b: &RunStyle) -> bool {
    if a.lang == b.lang && a.lang_ea == b.lang_ea && a.snap_to_grid == b.snap_to_grid {
        return original_format_equal(a, b);
    }
    let mut x = a.clone();
    x.lang.clone_from(&b.lang);
    x.lang_ea.clone_from(&b.lang_ea);
    x.snap_to_grid = b.snap_to_grid;
    original_format_equal(&x, b)
}

/// `w:fitText`: consecutive runs sharing a fit id take exactly the given width,
/// the difference spread evenly over their characters.
fn fit_text(atoms: &mut [Atom]) {
    let mut i = 0;
    while i < atoms.len() {
        let Some((w, id)) = atoms[i].style.fit_text else {
            i += 1;
            continue;
        };
        let same = |a: &Atom| match a.style.fit_text {
            Some((_, Some(j))) => Some(j) == id,
            Some(_) => id.is_none() && original_format_equal(&a.style, &atoms[i].style),
            None => false,
        };
        let mut j = i + 1;
        while j < atoms.len() && same(&atoms[j]) {
            j += 1;
        }
        let chars: Vec<usize> = (i..j)
            .filter(|&k| {
                matches!(atoms[k].kind, Kind::Text | Kind::Space) && atoms[k].anchor().is_none()
            })
            .collect();
        if !chars.is_empty() {
            let natural: f64 = chars.iter().map(|&k| atoms[k].width).sum();
            let d = (w - natural) / chars.len() as f64;
            for &k in &chars {
                atoms[k].width += d;
            }
        }
        i = j;
    }
}

/// `w:autoSpaceDE`/`w:autoSpaceDN`: where an East Asian character meets a Latin
/// letter or a digit, Word widens the earlier of the two by a quarter of its em
/// (page-1 thumbnails: 3.75 pt at 15 pt, 5.25 pt at 21 pt).
fn auto_space(st: &crate::office::docx::styles::ParaStyle, atoms: &mut [Atom]) {
    let side = |c: char| {
        if script_of(c) == Script::EastAsian && c.is_alphabetic() {
            Some(true)
        } else if (st.auto_space_dn && c.is_ascii_digit())
            || (st.auto_space_de && c.is_alphabetic() && script_of(c) == Script::Latin)
        {
            Some(false)
        } else {
            None
        }
    };
    for i in 1..atoms.len() {
        let (a, b) = (&atoms[i - 1], &atoms[i]);
        if a.kind != Kind::Text
            || b.kind != Kind::Text
            || a.anchor().is_some()
            || b.anchor().is_some()
        {
            continue;
        }
        let (Some(x), Some(y)) = (
            a.text.chars().last().and_then(side),
            b.text.chars().next().and_then(side),
        ) else {
            continue;
        };
        if x != y {
            atoms[i - 1].width += atoms[i - 1].style.size * 0.25;
        }
    }
}

/// `w:balanceSingleByteDoubleByteWidth`: a run of spaces next to an East Asian
/// character takes half an em per space; spaces between Latin words keep their
/// width (Word for Mac probe: 6 pt at 12 pt and 8 pt at 16 pt, any ASCII font).
/// On a character grid the em includes the grid's tracking (`grid`, points).
fn balance_spaces(atoms: &mut [Atom], grid: f64) {
    let east_asian = |c: Option<char>| c.is_some_and(|c| script_of(c) == Script::EastAsian);
    let mut i = 0;
    while i < atoms.len() {
        if atoms[i].kind != Kind::Space {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < atoms.len() && atoms[j].kind == Kind::Space {
            j += 1;
        }
        let before = i
            .checked_sub(1)
            .filter(|&k| atoms[k].kind == Kind::Text)
            .and_then(|k| atoms[k].text.chars().last());
        let after = atoms
            .get(j)
            .filter(|a| a.kind == Kind::Text)
            .and_then(|a| a.text.chars().next());
        if east_asian(before) || east_asian(after) {
            for a in &mut atoms[i..j] {
                if a.text.chars().all(|c| c == ' ') {
                    let em = a.style.size + if a.style.snap_to_grid { grid } else { 0.0 };
                    a.width = a.text.chars().count() as f64 * em / 2.0;
                }
            }
        }
        i = j;
    }
}

fn blank(kind: Kind, s: &Rc<RunStyle>) -> Atom {
    Atom {
        kind,
        text: AtomText::new(""),
        glyphs: Glyphs::Empty,
        width: 0.0,
        m: Rc::new(AtomFace {
            face: None,
            draw_size: s.size,
            face_ascent: 0.0,
            face_descent: 0.0,
            rise: 0.0,
            ea_extra: 0.0,
        }),
        style: s.clone(),
        ascent: 0.0,
        descent: 0.0,
        break_before: false,
        bidi_range: None,
        bidi_level: 0,
        x: None,
    }
}

fn tab_atom(s: &Rc<RunStyle>, env: &TextEnv) -> Atom {
    let (asc, desc) = font_line(env, s, &s.font);
    Atom {
        ascent: asc,
        descent: desc,
        text: AtomText::new("\t"),
        ..blank(Kind::Tab, s)
    }
}

/// Line ascent/descent of a run's font (requested family's Word metrics).
fn font_line(env: &TextEnv, s: &RunStyle, family: &str) -> (f64, f64) {
    let family = &crate::office::docx::metrics::effective_family(env.fonts, family);
    let face = env.fonts.resolve(family, s.bold, s.italic);
    let (a, d) = crate::office::docx::metrics::line_metrics_styled(
        env.fonts, family, face, s.size, s.bold, s.italic,
    );
    if env.no_leading {
        return (
            a - crate::office::docx::metrics::external_leading(env.fonts, family, face, s.size),
            d + s.shadow_extra,
        );
    }
    (a, d + s.shadow_extra)
}

/// Shape each tier independently, then keep the guide attached to its base.
fn ruby_atom(r: &crate::office::docx::model::Ruby, env: &TextEnv) -> Atom {
    let shape = |runs: &[(String, Rc<RunStyle>)]| {
        let mut atoms = Vec::new();
        let mut text = String::new();
        let mut offsets = Vec::new();
        for (t, s) in runs {
            shape_text(t, s, env, &mut atoms, &mut text, &mut offsets);
        }
        (atoms, text)
    };
    let (base, text) = shape(&r.base);
    let (guide, _) = shape(&r.guide);
    let width = base
        .iter()
        .map(|a| a.width)
        .sum::<f64>()
        .max(guide.iter().map(|a| a.width).sum::<f64>());
    let mut ascent: f64 = 0.0;
    let mut descent: f64 = 0.0;
    let mut items = Vec::new();
    for (atoms, raise) in [(base, 0.0), (guide, r.raise)] {
        for a in &atoms {
            ascent = ascent.max(a.ascent + raise);
            descent = descent.max(a.descent - raise);
        }
        let b = ruby_tier(atoms, width, &r.align);
        emit(
            &b,
            0.0,
            -raise,
            &mut items,
            &mut Vec::new(),
            &mut Vec::new(),
            env,
        );
    }
    Atom {
        text: AtomText::new(&text),
        width,
        ascent,
        descent,
        x: Some(Box::new(AtomExtra {
            ruby: Some(items),
            ..Default::default()
        })),
        ..blank(Kind::Text, &r.base[0].1)
    }
}

/// Distribute spare width between shaped clusters, keeping ligatures intact.
fn ruby_tier(atoms: Vec<Atom>, width: f64, align: &str) -> LineBuild {
    let spare = (width - atoms.iter().map(|a| a.width).sum::<f64>()).max(0.0);
    let n = atoms.len();
    let (start, gap) = match align {
        "left" => (0.0, 0.0),
        "right" => (spare, 0.0),
        "distributeLetter" if n > 1 => (0.0, spare / (n - 1) as f64),
        "distributeSpace" if n > 0 => (spare / (2 * n) as f64, spare / n as f64),
        _ => (spare / 2.0, 0.0),
    };
    let mut b = LineBuild::new(start);
    for a in atoms {
        b.xs.push(b.x);
        b.x += a.width + gap;
        b.atoms.push(a);
    }
    b
}

// Full-width East Asian and Latin clusters receive additive tracking (Word for Mac:
// digits and spaces move by the grid's -0.21 pt and +0.73 pt alike). Half-width
// forms, combining marks and zero-width characters keep advances.
/// A run's letter spacing (`w:spacing`) after one character. Under
/// `w:balanceSingleByteDoubleByteWidth`, Word doubles it after a full-width East
/// Asian character (probe: -13 twips draws -1.3 pt beside YaHei and SimSun).
fn run_spacing(piece: &str, style: &RunStyle) -> f64 {
    let full_width = piece.chars().next().is_some_and(|ch| {
        script_of(ch) == Script::EastAsian && !('\u{FF61}'..='\u{FFDC}').contains(&ch)
    });
    if full_width && crate::office::docx::metrics::balance_spaces() {
        2.0 * style.spacing
    } else {
        style.spacing
    }
}

fn character_tracking(piece: &str, style: &RunStyle, env: &TextEnv, advance: f64) -> f64 {
    if !style.snap_to_grid || advance <= 0.0 {
        return 0.0;
    }
    let Some(ch) = piece.chars().next() else {
        return 0.0;
    };
    let eligible = match script_of(ch) {
        Script::EastAsian => {
            !('\u{FF61}'..='\u{FFDC}').contains(&ch) && !matches!(ch, '\u{3099}' | '\u{309A}')
        }
        Script::Latin => {
            !matches!(ch, '\u{0300}'..='\u{036F}' | '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}')
        }
        Script::Complex => false,
    };
    if eligible {
        env.char_grid.get().unwrap_or(0.0).max(-advance)
    } else {
        0.0
    }
}

fn shape_text(
    text: &str,
    style: &Rc<RunStyle>,
    env: &TextEnv,
    atoms: &mut Vec<Atom>,
    full: &mut String,
    offsets: &mut Vec<usize>,
) {
    shape_direction(
        text,
        style,
        env,
        atoms,
        full,
        offsets,
        ShapeContext::default(),
    );
}

// Explicit symbols retain their font-specific code while exported text stays
// accessible. Windows symbol cmap aliases are confined to that cmap encoding.
fn shape_source_symbol(
    text: &str,
    style: &Rc<RunStyle>,
    env: &TextEnv,
    atoms: &mut Vec<Atom>,
    full: &mut String,
    offsets: &mut Vec<usize>,
) -> Option<()> {
    let (family, code) = style.symbol.as_ref()?;
    if text.chars().count() != 1 {
        return None;
    }
    let id = env.fonts.source_face(family, style.bold, style.italic)?;
    let fd = env.fonts.face(id)?;
    let face = fd.shaper()?;
    let code = *code as u32;
    let cmap = face.tables().cmap?;
    let valid = |id: &rustybuzz::ttf_parser::GlyphId| id.0 != 0 && id.0 < face.number_of_glyphs();
    let mut glyph = None;
    for sub in cmap.subtables {
        if sub.platform_id == rustybuzz::ttf_parser::PlatformId::Windows && sub.encoding_id == 0 {
            glyph = sub.glyph_index(code).filter(valid).or_else(|| {
                let alias = if code <= 0xff {
                    code + 0xf000
                } else if (0xf000..=0xf0ff).contains(&code) {
                    code - 0xf000
                } else {
                    return None;
                };
                sub.glyph_index(alias).filter(valid)
            });
            if glyph.is_some() {
                break;
            }
        }
    }
    let glyph = glyph.or_else(|| {
        cmap.subtables
            .into_iter()
            .filter(|sub| sub.is_unicode())
            .find_map(|sub| sub.glyph_index(code).filter(valid))
    })?;
    let advance = face.glyph_hor_advance(glyph)? as f64;
    let (size, rise) = draw_size(style);
    let upm = face.units_per_em() as f64;
    let em = env.fonts.em_scale(family, id);
    let advance = advance * size * em / upm * style.scale;
    let (mut asc, mut desc) =
        crate::office::docx::metrics::source_line_metrics(env.fonts, id, style.size);
    if style.vert_align != VertAlign::Baseline || style.position != 0.0 {
        let (a, d) = crate::office::docx::metrics::source_line_metrics(env.fonts, id, size);
        asc = (a + rise).max(0.0);
        desc = (d - rise).max(0.0);
        if style.vert_align == VertAlign::Baseline {
            let (a, d) =
                crate::office::docx::metrics::source_line_metrics(env.fonts, id, style.size);
            asc = asc.max(a);
            desc = desc.max(d);
        }
    }
    offsets.push(full.len());
    full.push_str(text);
    // Source-only provenance stays on the parsed run for later reflow.
    let mut painted_style = (**style).clone();
    painted_style.symbol = None;
    let painted_style = Rc::new(painted_style);
    atoms.push(Atom {
        text: AtomText::new(text),
        glyphs: Glyphs::from_vec(vec![(glyph.0, advance, 0.0, 0.0)]),
        width: advance + run_spacing(text, style) + character_tracking(text, style, env, advance),
        m: Rc::new(AtomFace {
            face: Some(id),
            draw_size: size * em,
            face_ascent: fd.metrics.ascent as f64 * size / upm,
            face_descent: fd.metrics.descent as f64 * size / upm,
            rise,
            ea_extra: 0.0,
        }),
        ascent: asc,
        descent: desc,
        ..blank(Kind::Text, &painted_style)
    });
    Some(())
}

/// Direction and surrounding source text used together during glyph shaping.
#[derive(Clone, Copy, Default)]
struct ShapeContext<'a> {
    direction: Option<rustybuzz::Direction>,
    context: Option<(&'a str, usize)>,
}

fn shape_direction(
    text: &str,
    style: &Rc<RunStyle>,
    env: &TextEnv,
    atoms: &mut Vec<Atom>,
    full: &mut String,
    offsets: &mut Vec<usize>,
    shaping: ShapeContext<'_>,
) {
    let ShapeContext { direction, context } = shaping;
    if text.is_empty() {
        return;
    }
    if style.symbol.is_some() {
        if shape_source_symbol(text, style, env, atoms, full, offsets).is_none() {
            let mut painted = (**style).clone();
            painted.symbol = None;
            let mut fallback = painted.clone();
            fallback.caps = false;
            fallback.small_caps = false;
            let first = atoms.len();
            shape_direction(text, &Rc::new(fallback), env, atoms, full, offsets, shaping);
            let painted = Rc::new(painted);
            // The mapped character's fallback face draws it; a Webdings or Wingdings
            // line keeps that font's own height when Word's metrics for it are known.
            let symbol_line = style
                .symbol
                .as_ref()
                .filter(|(family, _)| {
                    let f = family.trim().to_lowercase();
                    (f == "webdings" || f.starts_with("wingdings"))
                        && crate::office::docx::metrics::office_tabled(&f)
                })
                .map(|(family, _)| {
                    crate::office::docx::metrics::line_metrics_styled(
                        env.fonts, family, None, style.size, false, false,
                    )
                });
            for atom in &mut atoms[first..] {
                atom.style = painted.clone();
                if let Some((a, d)) = symbol_line {
                    atom.ascent = a;
                    atom.descent = d;
                }
            }
        }
        return;
    }
    let text: String = if style.caps {
        text.to_uppercase()
    } else {
        text.to_string()
    };
    let (base_size, rise) = draw_size(style);
    let fonts = env.fonts;
    // Segments of one face: (byte offset, text, face, requested family, small caps).
    let mut segs: Vec<(usize, String, Option<fontdb::ID>, String, bool)> = Vec::new();
    // Per requested family (one per script slot): the effective family, whether
    // it is a symbol font and its resolved face, looked up once per run.
    let mut fams: Vec<(*const str, String, bool, Option<fontdb::ID>)> = Vec::new();
    let mut drawn_to_source = Vec::new();
    // A space inside small caps takes the size of the letter before it.
    let mut prev_small = false;
    for (off, ch) in text.char_indices() {
        let small = style.small_caps && (ch.is_lowercase() || (ch.is_whitespace() && prev_small));
        prev_small = small;
        let ch_draw = if small {
            ch.to_uppercase().next().unwrap_or(ch)
        } else {
            ch
        };
        let req = family_for(style, ch_draw);
        let k = match fams.iter().position(|f| std::ptr::eq(f.0, req)) {
            Some(k) => k,
            None => {
                let family = crate::office::docx::metrics::effective_family(fonts, req);
                let symbol = crate::office::pptx::fonts::map_symbol_char(&family, 'a').is_some();
                let primary = if symbol {
                    None
                } else {
                    fonts.resolve(&family, style.bold, style.italic)
                };
                fams.push((req as *const str, family, symbol, primary));
                fams.len() - 1
            }
        };
        let family = fams[k].1.clone();
        // Symbol fonts (Symbol, Wingdings) are drawn as their Unicode equivalents.
        let mapped = if fams[k].2 {
            crate::office::pptx::fonts::map_symbol_char(&family, ch_draw)
        } else {
            None
        };
        let ch_draw = mapped.unwrap_or(ch_draw);
        let segment_offset = drawn_to_source.len();
        drawn_to_source.extend(std::iter::repeat_n(off, ch_draw.len_utf8()));
        let primary = if mapped.is_some() { None } else { fams[k].3 };
        let face = match primary {
            Some(id) if ch_draw.is_whitespace() || fonts.has_glyph(id, ch_draw) => Some(id),
            _ => fonts
                .fallback_weighted(
                    ch_draw,
                    // Symbol fonts have no bold face: their characters draw at
                    // regular weight in bold runs (bullets of a bold mark).
                    crate::office::pptx::fonts::requested_weight(
                        &family,
                        style.bold && mapped.is_none(),
                    ),
                    style.italic,
                )
                .or(primary),
        };
        match segs.last_mut() {
            Some((_, s, f, fam, sm)) if *f == face && *fam == family && *sm == small => {
                s.push(ch_draw)
            }
            _ => segs.push((segment_offset, ch_draw.to_string(), face, family, small)),
        }
    }
    drawn_to_source.push(text.len());
    let source_offset = |drawn: usize| drawn_to_source[drawn];
    let base_off = full.len();
    full.push_str(&text);
    for (seg_off, seg, face, family, small) in split_long(segs) {
        let size = if small {
            base_size * SMALL_CAPS_SCALE
        } else {
            base_size
        };
        // Line metrics follow the requested family at the run's full size.
        let (mut asc, mut desc) = crate::office::docx::metrics::line_metrics_styled(
            fonts,
            &family,
            face,
            style.size,
            style.bold,
            style.italic,
        );
        let ea_extra =
            crate::office::docx::metrics::east_asian_extra(fonts, &family, face, style.size);
        if env.no_leading {
            asc -= crate::office::docx::metrics::external_leading(fonts, &family, face, style.size);
        }
        if style.vert_align != VertAlign::Baseline || style.position != 0.0 {
            let (a2, d2) = crate::office::docx::metrics::line_metrics_styled(
                fonts,
                &family,
                face,
                base_size,
                style.bold,
                style.italic,
            );
            asc = (a2 + rise).max(0.0);
            desc = (d2 - rise).max(0.0);
            if style.vert_align == VertAlign::Baseline {
                let (a0, d0) = crate::office::docx::metrics::line_metrics_styled(
                    fonts,
                    &family,
                    face,
                    style.size,
                    style.bold,
                    style.italic,
                );
                // Lowered text takes its ascent down with it: a line of only
                // lowered text keeps its height and draws from the line top (Word:
                // a 14 pt title lowered 8 pt on a grid sits at the margin).
                if rise >= 0.0 {
                    asc = asc.max(a0);
                }
                // Raised text lifts its descent with it: the line below the
                // baseline shrinks by the raise (a 36 pt run raised 9 pt).
                if rise <= 0.0 {
                    desc = desc.max(d0);
                }
            }
        }
        desc += style.shadow_extra;
        let Some(id) = face else {
            // No face at all: keep the text measurable with a nominal advance.
            for (k, ch) in seg.char_indices() {
                offsets.push(base_off + source_offset(seg_off + k));
                atoms.push(Atom {
                    kind: if hanging_space(ch) {
                        Kind::Space
                    } else {
                        Kind::Text
                    },
                    text: AtomText::new(ch.encode_utf8(&mut [0u8; 4])),
                    width: size * 0.5 * style.scale
                        + run_spacing(ch.encode_utf8(&mut [0u8; 4]), style)
                        + character_tracking(
                            ch.encode_utf8(&mut [0u8; 4]),
                            style,
                            env,
                            size * 0.5 * style.scale,
                        ),
                    ascent: asc,
                    descent: desc,
                    m: Rc::new(AtomFace {
                        face: None,
                        draw_size: style.size,
                        face_ascent: 0.0,
                        face_descent: 0.0,
                        rise,
                        ea_extra,
                    }),
                    ..blank(Kind::Text, style)
                });
            }
            continue;
        };
        let Some(fd) = fonts.face(id) else { continue };
        let em = fonts.em_scale(&family, id);
        let upm = fd.metrics.units_per_em as f64;
        let scale = size * em / upm;
        let face_asc = fd.metrics.ascent as f64 * size / upm;
        let face_desc = fd.metrics.descent as f64 * size / upm;
        let seg_face = Rc::new(AtomFace {
            face: Some(id),
            draw_size: size * em,
            face_ascent: face_asc,
            face_descent: face_desc,
            rise,
            ea_extra,
        });
        let Some(shaper) = fd.shaper() else {
            continue;
        };
        let mut buf = UnicodeBuffer::new();
        buf.push_str(&seg);
        if let Some((paragraph, start)) = context {
            let begin = start + source_offset(seg_off);
            let source_end = source_offset(seg_off + seg.len());
            if let (Some(before), Some(after)) =
                (paragraph.get(..begin), paragraph.get(start + source_end..))
            {
                buf.set_pre_context(before);
                buf.set_post_context(after);
            }
        }
        buf.guess_segment_properties();
        if let Some(direction) = direction {
            buf.set_direction(direction);
        }
        let kern_on = style.kern.is_some_and(|k| k > 0.0 && style.size >= k);
        let features = [
            Feature::new(
                rustybuzz::ttf_parser::Tag::from_bytes(b"kern"),
                kern_on as u32,
                ..,
            ),
            Feature::new(
                rustybuzz::ttf_parser::Tag::from_bytes(b"liga"),
                style.ligatures as u32,
                ..,
            ),
            Feature::new(rustybuzz::ttf_parser::Tag::from_bytes(b"clig"), 0, ..),
        ];
        let out = rustybuzz::shape(shaper, &features, buf);
        // Word's own advances and kerning when the requested font is tabled.
        let table =
            crate::office::docx::metrics::word_face(fonts, &family, style.bold, style.italic);
        let infos = out.glyph_infos();
        let poss = out.glyph_positions();
        let mut starts: Vec<usize> = infos.iter().map(|g| g.cluster as usize).collect();
        starts.sort_unstable();
        starts.dedup();
        // Atoms go straight onto `atoms` (cluster offsets beside them in `cl`); a
        // right-to-left run, shaped in visual order, is put back in text order.
        let start = atoms.len();
        let mut cl: Vec<usize> = Vec::with_capacity(infos.len());
        let mut i = 0;
        while i < infos.len() {
            let cluster = infos[i].cluster as usize;
            let mut j = i;
            let mut glyphs = Vec::new();
            let mut adv = 0.0;
            while j < infos.len() && infos[j].cluster as usize == cluster {
                let g = &poss[j];
                let xa = g.x_advance as f64 * scale * style.scale;
                glyphs.push((
                    infos[j].glyph_id as u16,
                    xa,
                    g.x_offset as f64 * scale,
                    g.y_offset as f64 * scale,
                ));
                adv += xa;
                j += 1;
            }
            // The next larger cluster start (clusters are sorted, see `starts`).
            let next = match starts.binary_search(&(cluster + 1)) {
                Ok(k) | Err(k) => starts.get(k).copied().unwrap_or(seg.len()),
            };
            let piece = seg
                .get(cluster..next.max(cluster))
                .unwrap_or("")
                .to_string();
            let mono =
                crate::office::docx::metrics::mono_cjk(&family) && !fonts.has_family(&family);
            if table.is_some() || mono {
                // A ligature glyph keeps the difference between its own advance
                // and its letters' in the font (Calibri "ti" is 15/2048 em
                // narrower than "t" plus "i").
                let lig = if style.ligatures && glyphs.len() == 1 && piece.chars().count() > 1 {
                    let letters: Option<f64> = piece
                        .chars()
                        .map(|c| {
                            shaper
                                .glyph_index(c)
                                .and_then(|g| shaper.glyph_hor_advance(g))
                                .map(|a| a as f64 * scale * style.scale)
                        })
                        .sum();
                    letters.map_or(0.0, |l| adv - l)
                } else {
                    0.0
                };
                let tabled: Option<f64> = table.and_then(|wf| {
                    let unit = size / wf.upm as f64 * style.scale;
                    piece
                        .chars()
                        .map(|c| wf.advance(c).map(|a| a as f64 * unit))
                        .sum::<Option<f64>>()
                        .map(|w| w + lig)
                });
                // Monospaced CJK fonts: full-width characters take one em, the
                // rest half an em (their tables keep only the Latin ranges).
                let want = tabled.or_else(|| {
                    mono.then(|| {
                        piece
                            .chars()
                            .map(|c| {
                                let half = script_of(c) != Script::EastAsian
                                    || ('\u{FF61}'..='\u{FFDC}').contains(&c);
                                if half { 0.5 } else { 1.0 }
                            })
                            .sum::<f64>()
                            * size
                            * style.scale
                    })
                });
                // Bold in a family without a bold face (SimSun, Microsoft Sans Serif):
                // Word emboldens each glyph and widens it by 0.0194 em (Word for Mac
                // probes: 12 pt +0.23, 16 pt +0.31, 18 pt +0.35; 11 pt Latin +0.20).
                // A heavy family (Arial Black) keeps its advances in bold.
                let synthetic = table.is_some_and(|t| {
                    let name = t.family.to_lowercase();
                    !t.bold
                        && !["black", "bold", "demi", "heavy"]
                            .iter()
                            .any(|w| name.contains(w))
                });
                let want = want.map(|w| {
                    if style.bold && (synthetic || crate::office::docx::metrics::mono_cjk(&family))
                    {
                        w + piece.chars().count() as f64 * 0.0194 * size * style.scale
                    } else {
                        w
                    }
                });
                if let Some(want) = want {
                    // Glyphs keep their relative advances, scaled to Word's total.
                    // A substitute face's glyph sits centred in Word's wider or
                    // narrower cell, so the difference splits between its sides.
                    if adv > 0.0 {
                        let k = want / adv;
                        if table.is_some() {
                            for g in glyphs.iter_mut() {
                                g.2 += g.1 * (k - 1.0) / 2.0;
                            }
                        }
                        glyphs.iter_mut().for_each(|g| g.1 *= k);
                    } else if let Some(g) = glyphs.last_mut() {
                        g.1 = want;
                    }
                    adv = want;
                }
            }
            let first = piece.chars().next().unwrap_or(' ');
            let kind = if hanging_space(first) {
                Kind::Space
            } else {
                Kind::Text
            };
            cl.push(cluster);
            atoms.push(Atom {
                kind,
                text: AtomText::new(&piece),
                glyphs: Glyphs::from_vec(glyphs),
                width: adv
                    + run_spacing(&piece, style)
                    + character_tracking(&piece, style, env, adv),
                m: seg_face.clone(),
                style: style.clone(),
                ascent: asc,
                descent: desc,
                break_before: false,
                bidi_range: None,
                bidi_level: 0,
                x: None,
            });
            i = j;
        }
        if !cl.is_sorted() {
            let mut pairs: Vec<(usize, Atom)> = cl.drain(..).zip(atoms.drain(start..)).collect();
            pairs.sort_by_key(|(c, _)| *c);
            for (c, a) in pairs {
                cl.push(c);
                atoms.push(a);
            }
        }
        let run = &mut atoms[start..];
        // A soft hyphen takes no room; a line that breaks after it shows a hyphen.
        for a in run.iter_mut().filter(|a| a.text == "\u{AD}") {
            a.width = 0.0;
            a.glyphs.clear();
        }
        // Pair kerning from the table adjusts the first character's advance.
        if let (Some(wf), true) = (table, kern_on) {
            let unit = size / wf.upm as f64 * style.scale;
            for k in 0..run.len().saturating_sub(1) {
                let a = run[k].text.chars().last();
                let b = run[k + 1].text.chars().next();
                if let (Some(a), Some(b)) = (a, b) {
                    let d = wf.kerning(a, b) as f64 * unit;
                    if d != 0.0 {
                        let at = &mut run[k];
                        at.width += d;
                        if let Some(g) = at.glyphs.last_mut() {
                            g.1 += d;
                        }
                    }
                }
            }
        }
        offsets.extend(cl.iter().map(|c| base_off + source_offset(seg_off + c)));
    }
}

/// Segments longer than 16k characters are shaped in pieces cut after a space
/// (or anywhere, for text without one), which bounds the shaper's buffers.
fn split_long(
    segs: Vec<(usize, String, Option<fontdb::ID>, String, bool)>,
) -> Vec<(usize, String, Option<fontdb::ID>, String, bool)> {
    const MAX: usize = 16 * 1024;
    let mut out = Vec::with_capacity(segs.len());
    for (off, seg, face, family, small) in segs {
        if seg.len() <= 4 * MAX {
            out.push((off, seg, face, family, small));
            continue;
        }
        let mut rest = seg.as_str();
        let mut at = off;
        while !rest.is_empty() {
            let mut cut = rest.len().min(MAX);
            while !rest.is_char_boundary(cut) {
                cut += 1;
            }
            if cut < rest.len()
                && let Some(sp) = rest[..cut].rfind(' ')
            {
                cut = sp + 1;
            }
            out.push((at, rest[..cut].to_string(), face, family.clone(), small));
            at += cut;
            rest = &rest[cut..];
        }
    }
    out
}

/// Where the next tab after `x` lands: (position, kind, leader). Custom stops
/// first; a hanging indent acts as a stop; then default stops.
fn next_tab(
    x: f64,
    tabs: &[TabStop],
    hanging_at: Option<f64>,
    default_tab: f64,
) -> (f64, TabKind, Option<char>) {
    let custom = tabs
        .iter()
        .find(|t| t.pos > x + 0.01 && t.kind != TabKind::Bar);
    let hang = hanging_at
        .filter(|h| *h > x + 0.01)
        .filter(|_| !crate::office::docx::metrics::no_tab_hang_ind());
    match (custom, hang) {
        (Some(t), Some(h)) if h < t.pos => return (h, TabKind::Left, None),
        (Some(t), _) => return (t.pos, t.kind, t.leader),
        (None, Some(h)) => return (h, TabKind::Left, None),
        _ => {}
    }
    let step = if default_tab > 1.0 { default_tab } else { 36.0 };
    let after_custom = tabs.last().map(|t| t.pos).unwrap_or(0.0).max(x);
    let n = (after_custom / step + 1e-6).floor() + 1.0;
    (n * step, TabKind::Left, None)
}

struct Pending {
    kind: TabKind,
    pos: f64,
    /// Index of the tab atom in the line.
    atom: usize,
    /// x where the tab starts.
    start: f64,
}

struct LineBuild {
    atoms: Vec<Atom>,
    /// Laid-out x of each atom (tab widths resolved).
    xs: Vec<f64>,
    x: f64,
    pending: Option<Pending>,
}

impl LineBuild {
    fn new(x: f64) -> LineBuild {
        LineBuild {
            atoms: Vec::new(),
            xs: Vec::new(),
            x,
            pending: None,
        }
    }

    /// Right edge of the content if `extra` more width followed the last atom.
    fn end_with(&self, extra: f64) -> f64 {
        match &self.pending {
            Some(p) => {
                let seg = self.x + extra - p.start;
                match p.kind {
                    TabKind::Right | TabKind::Decimal => (p.start + seg).max(p.pos),
                    TabKind::Center => (p.start + seg).max(p.pos + seg / 2.0),
                    _ => self.x + extra,
                }
            }
            None => self.x + extra,
        }
    }

    /// Resolves the pending right/center tab: its width and the positions of
    /// the atoms after it.
    fn settle(&mut self) {
        let Some(p) = self.pending.take() else { return };
        let seg = self.x - p.start;
        let target_start = match p.kind {
            TabKind::Right | TabKind::Decimal => p.pos - seg,
            TabKind::Center => p.pos - seg / 2.0,
            _ => p.start,
        };
        let shift = (target_start - p.start).max(0.0);
        self.atoms[p.atom].width = shift;
        for x in self.xs.iter_mut().skip(p.atom + 1) {
            *x += shift;
        }
        self.x += shift;
    }

    fn push(&mut self, a: Atom) {
        self.xs.push(self.x);
        self.x += a.width;
        self.atoms.push(a);
    }
}

// Label glyphs were shaped and resolved in their own bidi paragraph. Place
// those atom levels while retaining logical text and intra-cluster glyph order.
fn place_rtl_label(b: &mut LineBuild, end: usize, left: f64) {
    let levels: Vec<_> = b.atoms[..end]
        .iter()
        .map(|a| unicode_bidi::Level::new(a.bidi_level).unwrap())
        .collect();
    let mut x = left;
    for i in unicode_bidi::BidiInfo::reorder_visual(&levels) {
        b.xs[i] = x;
        x += b.atoms[i].width;
    }
}

fn label_shift(p: &Paragraph, width: f64) -> f64 {
    match p.num.as_ref().map(|n| n.jc.as_str()) {
        Some("right") | Some("end") => -width,
        Some("center") => -width / 2.0,
        _ => 0.0,
    }
}

/// Puts a list label and its suffix (tab, space) at the start of `cur`.
fn push_label(
    p: &Paragraph,
    cur: &mut LineBuild,
    label_atoms: Vec<Atom>,
    suffix: Option<Atom>,
    hanging_at: Option<f64>,
    env: &TextEnv,
) {
    let st = &p.style;
    let label_w: f64 = label_atoms.iter().map(|a| a.width).sum();
    // Right- and center-justified labels end at / centre on the indent.
    let shift = label_shift(p, label_w);
    cur.x += shift;
    for a in label_atoms {
        cur.push(a);
    }
    if let Some(mut s) = suffix {
        if s.kind == Kind::Tab {
            let (pos, _, _) = next_tab(cur.x, &st.tabs, hanging_at, env.default_tab);
            s.width = (pos - cur.x).max(0.0);
        }
        cur.push(s);
    }
}

/// Wrapping geometry and the already consumed lines for a paragraph continuation.
struct WrapResume<'a> {
    wrap: &'a WrapIn,
    prefix: &'a [Line],
}

fn break_lines(
    p: &Paragraph,
    mut atoms: Vec<Atom>,
    label: Option<(Vec<Atom>, Option<Atom>)>,
    width: f64,
    env: &TextEnv,
    resume: WrapResume<'_>,
    bidi: Option<&bidi::Context<'_>>,
) -> Vec<Line> {
    let WrapResume { wrap, prefix } = resume;
    let st = &p.style;
    let left = st.left;
    // First-line indentation follows the paragraph's logical start edge.
    // The line builder advances left-to-right before bidi visual placement,
    // so an RTL hanging tab uses the equivalent distance from its left origin.
    let first_x = left + if st.bidi { 0.0 } else { st.first };
    let right_edge = (width - st.right).max(first_x.min(left) + 1.0);
    let first_edge = if st.bidi {
        (right_edge - st.first).max(first_x + 1.0)
    } else {
        right_edge
    };
    let hanging_at = (st.first < 0.0).then_some(if st.bidi { left - st.first } else { left });
    let mut lines: Vec<Line> = Vec::new();
    let mut flow = WrapFlow {
        wrap,
        prefix,
        line_no: 0,
        source_end: 0,
        left: first_x,
        y: 0.0,
        last_h: wrap.est_h,
        skip: 0.0,
        edge: first_edge,
        narrowed: None,
        beside_contour: false,
    };
    let mut cur = flow.start(first_x, first_edge);
    let mut line_no = 0usize;
    // Lines in a row that ended with an automatic hyphen.
    let mut hyph_run = 0usize;
    // A paragraph that opens with a page or column break shows its label on
    // the line after the break, with the rest of its text.
    let mut label = label;
    let opens_with_break = matches!(
        atoms.first().map(|a| a.kind),
        Some(Kind::Break(BreakKind::Page | BreakKind::Column))
    );
    let deferred = label.is_some() && opens_with_break;
    let mut pending = if deferred { label.take() } else { None };
    // A paragraph holding only the break shows no label (Word's page-1 thumbnails).
    if deferred
        && atoms[1..]
            .iter()
            .all(|a| matches!(a.kind, Kind::Space | Kind::Break(_)))
    {
        pending = None;
    }
    if let Some((label_atoms, suffix)) = label {
        push_label(p, &mut cur, label_atoms, suffix, hanging_at, env);
    }
    let mut i = 0;
    // Index in `cur.atoms` after which the line may break (start of a word).
    while i < atoms.len() {
        let a = &atoms[i];
        match a.kind {
            // A page or column break inside a table cell does nothing: Word
            // keeps the cell's text on one line.
            Kind::Break(k)
                if env.in_cell.get()
                    && !matches!(k, crate::office::docx::model::BreakKind::Line) =>
            {
                i += 1;
                continue;
            }
            Kind::Break(k) => {
                cur.push(a.clone());
                cur.settle();
                let edge = flow.edge;
                lines.push(flow.finish(finish_line(
                    p,
                    cur,
                    line_no,
                    LineEnd {
                        last: false,
                        brk: Some(k),
                    },
                    edge,
                    env,
                    bidi,
                )));
                line_no += 1;
                let pending_label = pending.take();
                // The line after an opening break is the paragraph's first line.
                cur = if pending_label.is_some() || (opens_with_break && line_no == 1) {
                    flow.start(first_x, first_edge)
                } else {
                    flow.start(left, right_edge)
                };
                if let Some((label_atoms, suffix)) = pending_label {
                    push_label(p, &mut cur, label_atoms, suffix, hanging_at, env);
                }
                i += 1;
                continue;
            }
            Kind::Tab => {
                cur.settle();
                let line_hanging = if st.bidi && line_no > 0 {
                    None
                } else {
                    hanging_at
                };
                let (pos, kind, leader) = next_tab(cur.x, &st.tabs, line_hanging, env.default_tab);
                // Trailing and tab-only runs wrap too (Word for Mac probes: 20 tabs
                // alone take two lines; three tabs in a 27 pt gap take three). A tab
                // starting at the right edge stays when nothing visible follows it,
                // and always in a header or footer.
                let ends_at_edge = cur.x >= flow.edge - 0.5
                    && (env.in_header.get()
                        || atoms[i + 1..]
                            .iter()
                            .all(|a| !matches!(a.kind, Kind::Text | Kind::Pic)));
                if pos > flow.edge + 0.5
                    && !ends_at_edge
                    && !cur.atoms.is_empty()
                    && kind == TabKind::Left
                    && st.tabs.iter().all(|t| t.pos < pos - 0.01)
                {
                    // A default tab past the right indent wraps to the next line.
                    // Beside a tight or through wrapped object, Word 2003 layout
                    // (compatibility mode 11) continues below the object.
                    let edge = flow.edge;
                    let below = env.compat_mode < 15 && flow.beside_contour;
                    lines.push(flow.finish(finish_line(
                        p,
                        cur,
                        line_no,
                        LineEnd {
                            last: false,
                            brk: None,
                        },
                        edge,
                        env,
                        bidi,
                    )));
                    line_no += 1;
                    cur = if below {
                        flow.start_needing(left, right_edge, f64::INFINITY)
                    } else {
                        flow.start(left, right_edge)
                    };
                    continue;
                }
                let mut t = a.clone();
                t.extra().tab_leader = leader;
                match kind {
                    TabKind::Left | TabKind::Bar | TabKind::Clear => {
                        t.width = (pos - cur.x).max(0.0);
                        cur.push(t);
                    }
                    _ => {
                        t.width = 0.0;
                        let idx = cur.atoms.len();
                        cur.push(t);
                        cur.pending = Some(Pending {
                            kind,
                            pos,
                            atom: idx,
                            start: cur.x,
                        });
                    }
                }
                i += 1;
                continue;
            }
            _ => {}
        }
        // A word: this atom up to the next break opportunity.
        let mut j = i + 1;
        while j < atoms.len()
            && !atoms[j].break_before
            && matches!(atoms[j].kind, Kind::Text | Kind::Pic)
        {
            j += 1;
        }
        // Trailing spaces hang past the edge.
        let mut k = j;
        while k < atoms.len() && atoms[k].kind == Kind::Space && !atoms[k].break_before {
            k += 1;
        }
        let word_w: f64 = atoms[i..j]
            .iter()
            .filter(|a| a.kind != Kind::Space)
            .map(|a| a.width)
            .sum::<f64>()
            + atoms[i..j]
                .iter()
                .filter(|a| a.kind == Kind::Space)
                .map(|a| a.width)
                .sum::<f64>();
        let soft = atoms[j - 1].text == "\u{AD}";
        let word_w = if soft {
            word_w + soft_hyphen(&atoms[j - 1], env).width
        } else {
            word_w
        };
        let word_is_space = atoms[i..j].iter().all(|a| a.kind == Kind::Space);
        let has_content = cur.atoms.iter().any(|a| {
            a.anchor().is_none()
                && a.note().is_none()
                && matches!(a.kind, Kind::Text | Kind::Pic | Kind::Tab)
        }) || (line_no == 0 && p.num.is_some());
        // Custom tab stops past the right indent still hold their text.
        let edge = match &cur.pending {
            Some(t) => flow.edge.max(t.pos),
            None => flow.edge,
        };
        // Japanese punctuation compresses only on justified lines (a left-aligned
        // Japanese table's page-1 thumbnail: 20% to 100% same line breaks without it);
        // other text also on left-aligned lines (A/B: Chinese files 7 better, 1 worse).
        let japanese = atoms[i..j]
            .first()
            .and_then(|a| a.style.lang_ea.clone())
            .unwrap_or_else(crate::office::docx::metrics::doc_lang_ea)
            .to_ascii_lowercase()
            .starts_with("ja");
        let slack = if matches!(st.jc, Jc::Both | Jc::Distribute) || !japanese {
            punct_slack(&cur.atoms, &atoms[i..j], cur.end_with(0.0) < edge)
        } else {
            0.0
        };
        // With compressPunctuation, a comma, full stop or colon ending the line takes half
        // its width (Word: a line-final "，" fills the line to the margin exactly).
        let hang = if crate::office::docx::metrics::compress_punct() {
            hanging_punct_width(&atoms[i..j]) / 2.0
        } else {
            0.0
        };
        let over = cur.end_with(word_w) - edge - 0.01 - slack - hang;
        let mut fits = word_is_space
            || over <= 0.0
            || (over <= squeeze(st, &cur.atoms, env) && over <= 0.4 * word_w);
        if !fits && !has_content && cur.pending.is_none() {
            // Beside an object the word moves the line to a gap it fits in.
            let carried = (cur.x - flow.left).max(0.0);
            if let Some(fresh) = flow.restart_for(word_w + carried) {
                let held = std::mem::replace(&mut cur, fresh);
                for a in held.atoms {
                    cur.push(a);
                }
                fits = cur.end_with(word_w) - flow.edge - 0.01 <= 0.0;
            }
        }
        if fits || !has_content {
            if !fits && !has_content {
                // A word wider than the line breaks between characters.
                let mut m = i;
                while m < j {
                    let w = atoms[m].width;
                    // Floating-object anchors and note marks are not text: an
                    // object alone on the line does not push the word below it,
                    // a space does (cover pages of two thumbnails).
                    if cur.end_with(w) > flow.edge + 0.01
                        && cur.atoms.iter().any(|a| {
                            matches!(a.kind, Kind::Text | Kind::Space)
                                && a.anchor().is_none()
                                && a.note().is_none()
                        })
                    {
                        cur.settle();
                        let edge = flow.edge;
                        lines.push(flow.finish(finish_line(
                            p,
                            cur,
                            line_no,
                            LineEnd {
                                last: false,
                                brk: None,
                            },
                            edge,
                            env,
                            bidi,
                        )));
                        line_no += 1;
                        cur = flow.start(left, right_edge);
                    }
                    cur.push(atoms[m].clone());
                    m += 1;
                }
            } else {
                for a in &atoms[i..j] {
                    cur.push(a.clone());
                }
            }
            for a in &atoms[j..k] {
                cur.push(a.clone());
            }
            i = k;
        } else {
            let limit = edge + 0.01 + squeeze(st, &cur.atoms, env);
            if let Some((head, tail)) = hyphen_split(st, &atoms[i..j], &cur, limit, hyph_run, env) {
                for a in head {
                    cur.push(a);
                }
                atoms.splice(i..j, tail);
                hyph_run += 1;
            } else {
                hyph_run = 0;
            }
            if let Some(last) = cur.atoms.last().filter(|a| a.text == "\u{AD}") {
                let h = soft_hyphen(last, env);
                cur.push(h);
            }
            cur.settle();
            let edge = flow.edge;
            lines.push(flow.finish(finish_line(
                p,
                cur,
                line_no,
                LineEnd {
                    last: false,
                    brk: None,
                },
                edge,
                env,
                bidi,
            )));
            line_no += 1;
            cur = flow.start(left, right_edge);
        }
    }
    cur.settle();
    let edge = flow.edge;
    lines.push(flow.finish(finish_line(
        p,
        cur,
        line_no,
        LineEnd {
            last: true,
            brk: None,
        },
        edge,
        env,
        bidi,
    )));
    lines
}

/// The visible hyphen a soft hyphen (`w:softHyphen`) turns into at a line end.
fn soft_hyphen(shy: &Atom, env: &TextEnv) -> Atom {
    let (mut v, mut full, mut offs) = (Vec::new(), String::new(), Vec::new());
    shape_text("-", &shy.style, env, &mut v, &mut full, &mut offs);
    v.pop().unwrap_or_else(|| shy.clone())
}

/// `w:autoHyphenation`: splits a word that does not fit at its first hyphenation
/// point when the head plus a hyphen fits before `limit`. Returns the head atoms
/// (ending in the hyphen) and the tail atoms that replace the word.
fn hyphen_split(
    st: &crate::office::docx::styles::ParaStyle,
    word: &[Atom],
    cur: &LineBuild,
    limit: f64,
    run: usize,
    env: &TextEnv,
) -> Option<(Vec<Atom>, Vec<Atom>)> {
    let max_run = crate::office::docx::hyph::consecutive_limit();
    if !crate::office::docx::hyph::enabled()
        || st.suppress_auto_hyphens
        || (max_run > 0 && run >= max_run)
    {
        return None;
    }
    let a = word.first()?;
    if cur.pending.is_some()
        || word.iter().any(|w| {
            w.kind != Kind::Text
                || w.ruby().is_some()
                || w.anchor().is_some()
                || w.note().is_some()
                || !original_format_equal(&w.style, &a.style)
        })
    {
        return None;
    }
    // The zone is measured from where the word would start to the right edge.
    if cur.end_with(0.0) > limit - crate::office::docx::hyph::zone() {
        return None;
    }
    let lang = a.style.lang.as_deref()?;
    let chars: Vec<char> = word.iter().flat_map(|w| w.text.chars()).collect();
    let lead = chars.iter().take_while(|c| !c.is_alphabetic()).count();
    let core_n = chars[lead..]
        .iter()
        .take_while(|c| c.is_alphabetic())
        .count();
    if chars[lead + core_n..].iter().any(|c| c.is_alphanumeric()) {
        return None;
    }
    let core: String = chars[lead..lead + core_n].iter().collect();
    if !crate::office::docx::hyph::hyphenate_caps() && core.chars().all(|c| !c.is_lowercase()) {
        return None;
    }
    let mut drawn = 0;
    let ranges = word
        .iter()
        .map(|atom| {
            let start = drawn;
            drawn += atom.text.len();
            Some((start..drawn, atom.source_range()?.clone()))
        })
        .collect::<Option<Vec<_>>>()?;
    let source_end = ranges.last()?.1.end;
    let mut source_owners = Vec::new();
    for (atom, (range, _)) in word.iter().zip(&ranges) {
        if let Some(parts) = atom
            .x
            .as_ref()
            .map(|extra| &extra.ownership)
            .filter(|parts| !parts.is_empty())
        {
            for (part, source) in parts {
                let mut style = (*atom.style).clone();
                style.source = source.clone();
                source_owners.push((
                    range.start + part.start..range.start + part.end,
                    Rc::new(style),
                ));
            }
        } else {
            source_owners.push((range.clone(), atom.style.clone()));
        }
    }
    let mut bookmark_positions = Vec::new();
    let mut word_offset = 0;
    for atom in word {
        if let Some(extra) = &atom.x {
            for (offset, marker) in &extra.bookmarks {
                let mut utf16 = 0;
                let byte = atom
                    .text
                    .char_indices()
                    .find_map(|(byte, ch)| {
                        if utf16 == *offset {
                            return Some(byte);
                        }
                        utf16 += ch.len_utf16();
                        None
                    })
                    .unwrap_or(atom.text.len());
                bookmark_positions.push((word_offset + byte, marker.clone()));
            }
        }
        word_offset += atom.text.len();
    }
    let shape = |t: &str, drawn_start: usize, limit: usize| {
        let (mut v, mut full, mut offs) = (Vec::new(), String::new(), Vec::new());
        shape_text(t, &a.style, env, &mut v, &mut full, &mut offs);
        for (index, b) in v.iter_mut().enumerate() {
            b.break_before = false;
            let lo = drawn_start + offs[index];
            let hi = drawn_start + offs.get(index + 1).copied().unwrap_or(full.len());
            b.extra().source_range = Some(hyphen_source_range(&ranges, lo, hi, limit, source_end));
            let owner_at = lo.min(limit.saturating_sub(1));
            let owner_index = source_owners
                .partition_point(|(range, _)| range.end <= owner_at)
                .min(source_owners.len() - 1);
            b.style = source_owners[owner_index].1.clone();
            let mut ownership = Vec::new();
            for (range, style) in &source_owners[owner_index..] {
                if range.start >= hi.min(limit) {
                    break;
                }
                let start = range.start.max(lo);
                let end = range.end.min(hi).min(limit);
                if start < end {
                    ownership.push((start - lo..end - lo, style.source.clone()));
                }
            }
            if hi > limit {
                let last = source_owners
                    .partition_point(|(range, _)| range.end < limit)
                    .min(source_owners.len() - 1);
                ownership.push((
                    limit.max(lo) - lo..hi - lo,
                    source_owners[last].1.source.clone(),
                ));
            }
            if ownership
                .iter()
                .any(|(_, source)| source != &b.style.source)
            {
                b.extra().ownership = ownership;
            }
        }
        for (position, marker) in &bookmark_positions {
            if *position < drawn_start || *position > limit || (*position == limit && limit < drawn)
            {
                continue;
            }
            let byte = position - drawn_start;
            let index = offs
                .partition_point(|offset| *offset <= byte)
                .saturating_sub(1);
            if let Some(atom) = v.get_mut(index) {
                let offset = full[offs[index]..byte.min(full.len())]
                    .encode_utf16()
                    .count();
                atom.extra().bookmarks.push((offset, marker.clone()));
            }
        }
        v
    };
    // Word's splits on a page-1 thumbnail match the first pattern point.
    let t = *crate::office::docx::hyph::points(&core, lang).first()?;
    let head_text: String = chars[..lead + t]
        .iter()
        .chain(std::iter::once(&'-'))
        .collect();
    let boundary = chars[..lead + t]
        .iter()
        .map(|c| c.len_utf8())
        .sum::<usize>();
    // A split point must coincide with an original source cluster boundary.
    ranges.iter().find(|(draw, _)| draw.end == boundary)?;
    let head = shape(&head_text, 0, boundary);
    let w: f64 = head.iter().map(|h| h.width).sum();
    if cur.end_with(w) <= limit {
        let mut tail = shape(
            &chars[lead + t..].iter().collect::<String>(),
            boundary,
            drawn,
        );
        if let Some(f) = tail.first_mut() {
            f.break_before = a.break_before;
        }
        return Some((head, tail));
    }
    None
}

fn hyphen_source_range(
    ranges: &[(std::ops::Range<usize>, std::ops::Range<usize>)],
    lo: usize,
    hi: usize,
    limit: usize,
    source_end: usize,
) -> std::ops::Range<usize> {
    let mut overlap = ranges
        .iter()
        .filter(|(draw, _)| draw.start < hi && draw.end > lo && draw.start < limit);
    let boundary_end = ranges
        .iter()
        .rfind(|(draw, _)| draw.end <= limit)
        .map_or(source_end, |(_, source)| source.end);
    let start = overlap
        .clone()
        .next()
        .map_or(boundary_end, |(_, source)| source.start);
    let end = overlap.next_back().map_or(start, |(_, source)| source.end);
    start..end
}

/// Room a justified line gains by narrowing its spaces. In compatibility mode 15
/// Word fits a line whose spaces shrink to 0.75 of their width (Word for Mac probe:
/// a Cyrillic line fits at 0.754 and wraps at 0.504; A/B: 12 documents better, none
/// worse than the earlier 0.776).
fn squeeze(st: &crate::office::docx::styles::ParaStyle, atoms: &[Atom], env: &TextEnv) -> f64 {
    let f = 0.25;
    if st.jc != Jc::Both || env.compat_mode < 15 {
        return 0.0;
    }
    let mut end = atoms.len();
    while end > 0 && atoms[end - 1].kind == Kind::Space {
        end -= 1;
    }
    let spaces: f64 = atoms[..end]
        .iter()
        .filter(|a| a.kind == Kind::Space)
        .map(|a| a.width)
        .sum();
    // The space before the incoming word squeezes too.
    let trailing: f64 = atoms[end..].iter().map(|a| a.width).sum();
    f * (spaces + trailing)
}

// Plain tabs set horizontal positions; surrounding text or the paragraph mark
// supplies line metrics. Painted leaders and decorated tabs retain metrics.
fn plain_tab(a: &Atom) -> bool {
    a.kind == Kind::Tab
        && !a.is_label()
        && a.tab_leader().is_none()
        && a.style.underline.is_none()
        && !a.style.strike
        && a.style.highlight.is_none()
        && a.style.shading.is_none()
}

/// Whether a line ends the paragraph or carries an explicit source break.
struct LineEnd {
    last: bool,
    brk: Option<BreakKind>,
}

fn finish_line(
    p: &Paragraph,
    mut b: LineBuild,
    line_no: usize,
    ending: LineEnd,
    right_edge: f64,
    env: &TextEnv,
    bidi: Option<&bidi::Context<'_>>,
) -> Line {
    let LineEnd { last, brk } = ending;
    let source_end = b
        .atoms
        .iter()
        .filter_map(|a| a.source_range().map(|r| r.end))
        .max()
        .unwrap_or(0);
    let st = &p.style;
    // Metrics: every atom on the line, the paragraph mark on the last line.
    let mut asc: f64 = 0.0;
    let mut desc: f64 = 0.0;
    let mut font_h: f64 = 0.0;
    let mut has_metric = false;
    // Ascent, descent and height of the text atoms (t), label atoms (l) and pictures (p).
    let (mut at, mut dt, mut ht) = (0.0_f64, 0.0_f64, 0.0_f64);
    let (mut al, mut dl, mut hl) = (0.0_f64, 0.0_f64, 0.0_f64);
    let (mut ap, mut dp) = (0.0_f64, 0.0_f64);
    // Tallest text run sized by the East Asian 1.3 rule (0 without one).
    let mut ea_h = 0.0_f64;
    // Tabs after the line's last visible text do not size it either (a
    // trailing 18 pt tab after 11 pt words leaves their height unchanged).
    let last_text = b.atoms.iter().rposition(|a| {
        a.kind == Kind::Text
            && a.anchor().is_none()
            && a.note().is_none()
            && !a.text.trim().is_empty()
    });
    for (i, a) in b.atoms.iter().enumerate() {
        if a.anchor().is_some() || a.note().is_some() {
            continue;
        }
        let tab_unsized = a.kind == Kind::Tab && last_text.is_some_and(|t| i > t);
        // Spaces do not size a line with visible text (an 18 pt space among 14 pt words
        // leaves it 14 pt). On a line of spaces alone, ordinary spaces leave the mark's
        // height (Word for Mac probes: a 42 pt space, modes 11 and 15); en spaces, as
        // in form-field placeholders, still size it.
        let unsized_space =
            a.kind == Kind::Space && (last_text.is_some() || a.text.chars().all(|c| c == ' '));
        // A break sizes only a line without visible text (Word for Mac probes: a 24 pt
        // break after 10 pt "A" leaves the line 10 pt; after a lone space it is 24 pt).
        let break_unsized = matches!(a.kind, Kind::Break(_)) && last_text.is_some();
        if plain_tab(a) || unsized_space || tab_unsized || break_unsized {
            continue;
        }
        if a.ascent + a.descent > 0.0 {
            if a.kind == Kind::Pic {
                ap = ap.max(a.ascent);
                dp = dp.max(a.descent);
            } else if a.is_label() {
                al = al.max(a.ascent);
                dl = dl.max(a.descent);
                hl = hl.max(a.ascent + a.descent);
            } else {
                at = at.max(a.ascent);
                dt = dt.max(a.descent);
                ht = ht.max(a.ascent + a.descent);
                if a.m.ea_extra > 0.0 {
                    ea_h = ea_h.max(a.ascent + a.descent);
                }
            }
            has_metric = true;
        }
    }
    // Beside text, a list label adds only its ascent above the text's: its descent
    // and the line multiple stay the text's (Word for Mac probes: 13 and 20 pt Arial
    // labels on 10 pt Arial at single, 1.15 and 1.5 lines).
    font_h = font_h.max(if ht > 0.0 { ht } else { hl });
    let a_txt = at.max(al);
    let d_txt = if ht > 0.0 { dt } else { dl };
    asc = asc.max(a_txt).max(ap);
    desc = desc.max(d_txt.max(0.0)).max(dp);
    let has_text = b.atoms.iter().any(|a| {
        a.kind == Kind::Text
            && a.anchor().is_none()
            && a.note().is_none()
            && !a.text.trim().is_empty()
    });
    // In table cells the mark does not count (A/B: 9 documents better, 2 worse;
    // page-1 thumbnail of a form agrees). Visible text hides it too (Word for Mac
    // probes: a 20 pt mark leaves a 10 pt text line unchanged in modes 11-15).
    let mark_counts = env.mark_in_last_line && !env.in_cell.get() && !has_text;
    let mut mark_asc = 0.0_f64;
    // Highest ascent on the line without the East Asian 1.3-rule extra: grid
    // snapping measures the line's combined plain height from it.
    let mut plain_asc = b
        .atoms
        .iter()
        .filter(|a| a.anchor().is_none())
        .map(|a| a.ascent - a.m.ea_extra)
        .fold(0.0_f64, f64::max);
    if !has_metric || (last && mark_counts) {
        let (ma, md) = font_line(env, &p.mark, &p.mark.font);
        let family = crate::office::docx::metrics::effective_family(env.fonts, &p.mark.font);
        let face = env.fonts.resolve(&family, p.mark.bold, p.mark.italic);
        let extra =
            crate::office::docx::metrics::east_asian_extra(env.fonts, &family, face, p.mark.size);
        plain_asc = plain_asc.max(ma - extra);
        mark_asc = ma;
        asc = asc.max(ma);
        desc = desc.max(md);
        font_h = font_h.max(ma + md);
    }
    // A line holding an inline picture and no visible text is the picture's
    // height: the paragraph mark's descent fits inside it (page-1 thumbnails:
    // median line delta 1.73 to 1.07 pt). A picture beside text keeps the
    // text's descent below it.
    let pic_h = b
        .atoms
        .iter()
        .filter(|a| a.kind == Kind::Pic && a.anchor().is_none())
        .map(|a| a.ascent)
        .fold(0.0_f64, f64::max);
    // Headers and footers too: a logo line is the logo's height (page-1
    // thumbnails of header logos: 8 documents closer, 1 further).
    if pic_h > 0.0 && !has_text {
        let text_asc = b
            .atoms
            .iter()
            .filter(|a| {
                a.kind != Kind::Pic && !plain_tab(a) && a.anchor().is_none() && a.note().is_none()
            })
            .map(|a| a.ascent)
            .fold(0.0_f64, f64::max);
        asc = text_asc.max(pic_h - desc);
        // A picture shorter than the paragraph mark's line leaves the line at
        // the mark's height.
        asc = asc.max(mark_asc);
        // In a table cell too: a thin picture (a horizontal rule) takes the mark's
        // whole line, and a taller one adds no descent below it (Word for Mac probe:
        // a 0.05 pt rule in 10 pt Verdana steps 12.2 pt in a cell, as in the body).
        if env.in_cell.get() && last {
            let (ma, md) = font_line(env, &p.mark, &p.mark.font);
            if asc + desc < ma + md {
                asc = ma + md - desc;
            }
        }
    }
    // With an East Asian 1.3-rule run, the line is the taller of that run's height and
    // the combined plain height: a Western font's deeper descent fits inside the
    // East Asian line (Word for Mac probes: Century with MS Mincho, 10.5-14 pt).
    // Lines that snap to a document grid keep the combined height they snap by.
    let snaps = line_grid(env).is_some()
        && st.snap_to_grid
        && (!env.in_cell.get() || env.grid_in_table)
        && !env.in_header.get();
    if ea_h > 0.0 && pic_h == 0.0 && !snaps {
        let target = ea_h.max(plain_asc + desc);
        if asc + desc > target {
            desc = (target - asc).max(0.0);
        }
    }
    let natural = asc + desc;
    // How much the East Asian extras raised this line (0 when a Western font or a
    // picture sets its height).
    let ea_extra = (asc - plain_asc).clamp(0.0, natural);
    let (mut height, mut baseline) = match st.line {
        Some((v, LineRule::Auto)) => {
            let f = (v / 240.0).max(0.06);
            let h = natural + (f - 1.0) * font_h.max(if font_h > 0.0 { 0.0 } else { natural });
            (h.max(0.0), asc + (h - natural).min(0.0))
        }
        Some((v, LineRule::Exact)) => {
            let h = v / 20.0;
            // An exact line keeps the font's ascent share of its natural height
            // (page-1 thumbnails with exact spacing: median line delta 1.68 to 0.98 pt).
            let base = if natural > 0.0 {
                h * asc / natural
            } else {
                h - desc
            };
            (h, base)
        }
        Some((v, LineRule::AtLeast)) => {
            let h = (v / 20.0).max(natural);
            (h, asc + (h - natural))
        }
        None => (natural, asc),
    };
    // Lines in table cells snap to the grid only with `adjustLineHeightInTable`
    // (A/B: 3 documents better, none worse). Header and footer lines never snap:
    // measuring their height must match how they are drawn.
    let grid_here = (!env.in_cell.get() || env.grid_in_table) && !env.in_header.get();
    // Room below the text of a single-spaced grid line: half its snapped extra over
    // its natural height. It may run past the bottom of the page (Word for Mac
    // probes: 12 pt SimSun in two 14.3 pt pitches keeps a last line 6 pt over, not
    // 8 pt; 16 pt keeps 3.5 pt, not 4.5 pt).
    let mut grid_tail = 0.0;
    if let (Some(pitch), true) = (line_grid(env).filter(|_| grid_here), st.snap_to_grid) {
        // Document grid: a single line takes whole grid pitches. The East Asian
        // 1.3 extra comes off the line's own height first.
        if !matches!(
            st.line,
            Some((_, LineRule::Exact)) | Some((_, LineRule::AtLeast))
        ) {
            height = (height - ea_extra).max(0.0);
            baseline -= ea_extra;
        }
        // Whole pitches come from the taller of the line's plain height and its
        // tallest single font with the 1.3 extra (Word for Mac probes: 14 pt MS Mincho
        // takes two 18 pt pitches, 12 pt SimSun beside Times one 15.6 pt pitch).
        let snap_h = (natural - ea_extra).max(font_h);
        let units = (snap_h / pitch - 1e-3).ceil().max(1.0);
        let snapped = match st.line {
            Some((_, LineRule::Exact)) => height,
            // A multiple sets a floor in pitches; the text's own whole pitches win
            // when larger (Word for Mac probes on a 15.6 pt grid: 10.5 pt at 1.5 lines
            // is 1.5 pitches, 14 and 24 pt at 1.5 or 2 lines stay at 2).
            Some((v, LineRule::Auto)) => units.max(v / 240.0) * pitch,
            // "At least 0" keeps the line's own height (Word: 10.5 pt MS Gothic in
            // an 18 pt grid table spaces 13.7 pt).
            Some((v, LineRule::AtLeast)) if v <= 0.0 => height,
            Some((v, LineRule::AtLeast)) => (v / 20.0).max(units * pitch),
            // A raised or lowered run keeps the line's own pitches (Word for Mac: 10.5 pt
            // MS Mincho raised 4 pt stays one 16.1 pt pitch).
            _ if b.atoms.iter().any(|a| a.m.rise != 0.0) => {
                (height / pitch).ceil().max(1.0) * pitch
            }
            _ => (height / pitch).ceil().max(units) * pitch,
        };
        // Text sits centred in the snapped height as a single line would: a multiple
        // adds whole pitches around it (Word for Mac probe, 14 pt SimSun on a 15.6 pt
        // grid at 1.5, 2 and 3 lines).
        match st.line {
            Some((v, LineRule::Auto)) if v > 240.0 => {
                let single = (natural - ea_extra).max(0.0);
                baseline = asc - ea_extra + (snapped - single) / 2.0;
            }
            _ => baseline += (snapped - height) / 2.0,
        }
        if matches!(st.line, None | Some((240.0, LineRule::Auto))) {
            grid_tail = ((snapped - natural) / 2.0).max(0.0);
        }
        height = snapped;
    }
    // Alignment.
    let mut end = b.atoms.len();
    // Trailing RTL tabs move to the visual left edge and do not set the
    // right-alignment edge of the visible text.
    // A trailing ideographic space (U+3000) that fits on the line keeps its room,
    // as in Word: right-aligned form labels padded with it line up.
    let kept_ideographic = |k: usize| {
        let a = &b.atoms[k];
        a.kind == Kind::Space
            && a.text.starts_with('\u{3000}')
            && b.xs[k] + a.width <= right_edge + 0.01
    };
    while end > 0
        && !kept_ideographic(end - 1)
        && (matches!(b.atoms[end - 1].kind, Kind::Space | Kind::Break(_))
            || (bidi.is_some() && st.bidi && b.atoms[end - 1].kind == Kind::Tab))
    {
        end -= 1;
    }
    let mut content_end = if end > 0 {
        b.xs[end - 1] + b.atoms[end - 1].width
    } else {
        b.x
    };
    // A line admitted by compressing its CJK punctuation (`punct_slack`) takes the
    // room from that punctuation: each mark gives up to half its width, an opening
    // one on its left, a closing one on its right.
    if content_end > right_edge + 0.01 && crate::office::docx::metrics::compress_punct() {
        let marks: Vec<(usize, bool)> = (0..end)
            .filter_map(|k| {
                crate::office::docx::metrics::compressible_punct(&b.atoms[k].text).map(|o| (k, o))
            })
            .collect();
        let cap: f64 = marks.iter().map(|&(k, _)| b.atoms[k].width / 2.0).sum();
        if cap > 0.0 {
            let ratio = ((content_end - right_edge) / cap).min(1.0);
            let mut shift = 0.0;
            let mut next = marks.iter().peekable();
            for k in 0..b.atoms.len() {
                b.xs[k] -= shift;
                if let Some(&&(m, opening)) = next.peek().filter(|&&&(m, _)| m == k) {
                    next.next();
                    let cut = b.atoms[m].width / 2.0 * ratio;
                    b.atoms[m].width -= cut;
                    if opening {
                        if let Glyphs::One(g) = &mut b.atoms[m].glyphs {
                            g.2 -= cut;
                        } else if let Glyphs::Many(v) = &mut b.atoms[m].glyphs {
                            v.iter_mut().for_each(|g| g.2 -= cut);
                        }
                    }
                    shift += cut;
                }
            }
            b.x -= shift;
            content_end -= shift;
        }
    }
    let line_start = b.xs.first().copied().unwrap_or(b.x);
    let raw_free = right_edge - content_end;
    let free = raw_free.max(0.0);
    // `expandShiftReturn` stretches a line ended by a line break; one ended by a page
    // or column break stays left-aligned, like a paragraph's last line.
    let justify_ok = !last
        && match brk {
            None => true,
            Some(BreakKind::Line) => env.expand_shift_return,
            Some(_) => false,
        };
    let ends_with_line_break = matches!(
        b.atoms.last().map(|a| a.kind),
        Some(Kind::Break(BreakKind::Line))
    );
    let justify = match st.jc {
        Jc::Both => justify_ok && (!ends_with_line_break || env.expand_shift_return),
        Jc::Distribute => true,
        _ => false,
    };
    let mut dx = match st.jc {
        Jc::Center => free / 2.0,
        Jc::Right => free,
        _ => 0.0,
    };
    let labels = b.atoms.iter().take_while(|a| a.is_label()).count();
    if justify {
        let last_tab = b.atoms[..end].iter().rposition(|a| a.kind == Kind::Tab);
        let from = last_tab.map(|t| t + 1).unwrap_or(0);
        let from = if st.bidi {
            from.max(labels).min(end)
        } else {
            from
        };
        let spaces = b.atoms[from..end]
            .iter()
            .filter(|a| a.kind == Kind::Space)
            .count();
        if spaces > 0 {
            // A line that fits only with its spaces squeezed narrows them.
            let extra_per_space = if raw_free < 0.0 { raw_free } else { free } / spaces as f64;
            let mut add = 0.0;
            for k in from..b.atoms.len() {
                b.xs[k] += add;
                if k < end && b.atoms[k].kind == Kind::Space {
                    add += extra_per_space;
                    b.atoms[k].width += extra_per_space;
                }
            }
        } else if st.jc == Jc::Distribute && end > from + 1 {
            let per = free / (end - from - 1) as f64;
            for (n, k) in (from..b.atoms.len()).enumerate() {
                b.xs[k] += per * n.min(end - from - 1) as f64;
            }
        }
        dx = 0.0;
    }
    let label_block = if st.bidi && labels > 0 {
        let body_start = b.xs.get(labels).copied().unwrap_or(b.x);
        let suffix = usize::from(p.num.as_ref().is_some_and(|n| n.suffix != Suffix::Nothing));
        let label_end = labels.saturating_sub(suffix);
        let label_w: f64 = b.atoms[..label_end].iter().map(|a| a.width).sum();
        let shift = label_shift(p, label_w);
        // Justification has already settled the body advances. The label's
        // own alignment is relative to the pre-shift paragraph start anchor.
        let settled_end = if end > labels {
            b.xs[end - 1] + b.atoms[end - 1].width
        } else {
            body_start
        };
        let label_target = if end <= labels {
            right_edge - dx
        } else {
            settled_end
        };
        (label_end > 0).then_some({
            (
                body_start - line_start + shift,
                label_end,
                label_target - shift - label_w,
            )
        })
    } else {
        None
    };
    if let Some(ctx) = bidi {
        ctx.place(&mut b);
    }
    // Numbering has its own directional context. Place the label at its
    // start-edge anchor and the suffix in the gap before the body.
    if let Some((advance, label_end, label_left)) = label_block {
        place_rtl_label(&mut b, label_end, label_left);
        let mut suffix_left = label_left;
        for i in (label_end..labels).rev() {
            suffix_left -= b.atoms[i].width;
            b.xs[i] = suffix_left;
        }
        for x in &mut b.xs[labels..] {
            *x -= advance;
        }
    }
    let _ = line_no;
    let mut items = Vec::new();
    let mut anchors = Vec::new();
    let mut boxes = Vec::new();
    emit(&b, dx, baseline, &mut items, &mut anchors, &mut boxes, env);
    let text: String = b.atoms.iter().map(|a| &*a.text).collect();
    let notes = b.atoms.iter().filter_map(|a| a.note()).collect();
    Line {
        height,
        replay: Some(LineMeasure {
            source_end,
            left: 0.0,
            right: 0.0,
            skip: 0.0,
        }),
        trailing_leading: if line_grid(env).is_none()
            && matches!(st.line, Some((v, LineRule::Auto)) if v >= 240.0)
        {
            (height - natural).max(0.0)
        } else {
            grid_tail
        },
        baseline,
        items,
        break_after: brk.filter(|k| *k != BreakKind::Line),
        anchors,
        notes,
        text,
        skip: 0.0,
        boxes,
    }
}

/// Glyph runs, pictures, tab leaders and text decorations of a finished line.
fn emit(
    b: &LineBuild,
    dx: f64,
    baseline: f64,
    items: &mut Vec<Item>,
    anchors: &mut Vec<Anchored>,
    boxes: &mut Vec<InlineTextBox>,
    env: &TextEnv,
) {
    let mut decorations: Vec<Item> = Vec::new();
    let mut backgrounds: Vec<Item> = Vec::new();
    let mut i = 0;
    let n = b.atoms.len();
    // Trailing spaces are not underlined.
    let mut last_ink = n;
    while last_ink > 0 && b.atoms[last_ink - 1].kind == Kind::Space {
        last_ink -= 1;
    }
    // Source drawings delimit interior whitespace even when the line has no body text.
    let content =
        |a: &Atom| !a.text.trim().is_empty() || a.anchor().is_some() || a.kind == Kind::Pic;
    let first_text = b.atoms.iter().position(content);
    let last_text = b.atoms.iter().rposition(content);
    let source_spaces = source_split_spaces(b, last_ink);
    let line_start = items.len();
    let mut prev_space = false;
    let mut bound_bookmarks = std::collections::HashSet::new();
    while i < n {
        let a = &b.atoms[i];
        let source_slot = 2 * anchors.len();
        let source_split_space = source_spaces[i];
        let mut x = b.xs[i] + dx;
        if let Some(ruby) = a.ruby() {
            for it in ruby {
                let mut it = it.clone();
                translate_item(&mut it, x, baseline);
                set_reading_key(std::slice::from_mut(&mut it), source_slot);
                items.push(it);
            }
            prev_space = false;
            i += 1;
            continue;
        }
        if let Some(anc) = a.anchor() {
            let mut anc = anc.clone();
            if anc.h_rel == crate::office::docx::model::HRel::Character {
                anc.x = Ok(anc.x.clone().unwrap_or(0.0) + x);
            }
            anchors.push(anc);
            i += 1;
            continue;
        }
        match a.kind {
            Kind::Pic => {
                // Shapes are drawn after the picture (a picture's outline goes over it).
                let pic_at = items.len();
                if let Some(spec) = a.pic().and_then(|p| p.shape.as_ref()) {
                    let top = baseline - a.pic().map_or(0.0, |p| p.h) - a.rise();
                    for it in &spec.items {
                        let mut it = it.clone();
                        translate_item(&mut it, x, top);
                        set_reading_key(std::slice::from_mut(&mut it), source_slot);
                        items.push(it);
                    }
                    if !spec.texts.is_empty() {
                        boxes.push(InlineTextBox {
                            spec: spec.clone(),
                            x,
                            top,
                            source_slot,
                            text_before: items
                                .iter()
                                .filter(|i| matches!(i, Item::Text(_)))
                                .count(),
                        });
                    }
                }
                if let Some(pic) = a.pic()
                    && let Some(part) = &pic.part
                {
                    // A picture that sets the line's height starts at the line top.
                    let y = if baseline < pic.h {
                        (baseline - pic.h).max(0.0)
                    } else {
                        baseline - pic.h
                    } - a.rise();
                    items.push(Item::Image(ImageItem {
                        image: ImageRef {
                            part: part.clone(),
                            effects: pic.effects.clone(),
                        },
                        transform: Transform::from_row(
                            pic.w as f32,
                            0.0,
                            0.0,
                            pic.h as f32,
                            x as f32,
                            y as f32,
                        ),
                        crop: pic.crop,
                        alpha: pic.alpha,
                        clip: None,
                        shape: None,
                        alt_text: pic.alt.clone(),
                    }));
                    let img = items.pop();
                    items.splice(pic_at..pic_at, img);
                    if let Some(s) = crate::office::docx::decor::picture_shadow(pic, x, y) {
                        items.insert(pic_at, s);
                    }
                }
                prev_space = false;
                i += 1;
                continue;
            }
            Kind::Tab => {
                if let (Some(leader), true) = (a.tab_leader(), a.width > 2.0) {
                    let start = items.len();
                    leader_run(a, x, baseline, leader, env, items);
                    set_reading_key(&mut items[start..], source_slot);
                }
                // Underlined and shaded tabs are drawn across their width.
                let st = &a.style;
                if let Some(bg) = st.highlight.or(st.shading) {
                    let (asc, desc) = (st.size * 0.9, st.size * 0.25);
                    if let Some(path) = crate::office::pptx::textlayout::rect_path(
                        x,
                        baseline - asc,
                        a.width,
                        asc + desc,
                    ) {
                        backgrounds.push(fill_item(path, bg));
                    }
                }
                if st.underline.as_deref().is_some_and(|u| u != "words") {
                    let color = st.color.unwrap_or_else(|| {
                        auto_color(st.highlight.or(st.shading).or_else(background))
                    });
                    underline(st, x, x + a.width, baseline, color, &mut decorations);
                }
                prev_space = true;
                i += 1;
                continue;
            }
            Kind::Break(_) => {
                i += 1;
                continue;
            }
            // Struck spaces inside the line carry the strike (tracked deletions).
            Kind::Space
                if a.style.underline.is_none()
                    && a.style.highlight.is_none()
                    && a.style.shading.is_none()
                    && !(a.style.strike && i < last_ink) =>
            {
                let interior = first_text.is_some_and(|first| i > first)
                    && last_text.is_some_and(|last| i < last);
                if (!interior && !source_split_space)
                    || retain_interior_space(
                        &mut items[line_start..],
                        source_slot,
                        &a.text,
                        a.style.source.as_ref(),
                        x,
                        a.width,
                    )
                {
                    prev_space = true;
                    i += 1;
                    continue;
                }
            }
            _ => {}
        }
        // A run: atoms sharing face, size and style.
        let mut j = i;
        let mut glyphs = Vec::new();
        let mut text = String::new();
        while j < n {
            let c = &b.atoms[j];
            if !matches!(c.kind, Kind::Text | Kind::Space)
                || c.ruby().is_some()
                || c.anchor().is_some()
                || c.face() != a.face()
                || c.draw_size() != a.draw_size()
                || !(Rc::ptr_eq(&c.style, &a.style)
                    || same_format(&c.style, &a.style)
                    || internal_paint_equivalent(&c.style, &a.style))
                || c.rise() != a.rise()
                || c.bidi_level != a.bidi_level
                || (a.is_label()
                    && a.bidi_level.is_multiple_of(2)
                    && j > i
                    && b.xs[j] < b.xs[j - 1])
            {
                break;
            }
            let mut gx = b.xs[j] + dx - x;
            for (gid, adv, xo, yo) in c.glyphs.as_slice() {
                if c.kind == Kind::Text {
                    glyphs.push(Glyph {
                        id: *gid,
                        x: gx + xo,
                        y: -yo,
                    });
                }
                gx += adv;
            }
            text.push_str(&c.text);
            j += 1;
        }
        let rtl = a.bidi_level % 2 == 1;
        let end_x = if rtl {
            let origin = (i..j).map(|k| b.xs[k] + dx).fold(f64::INFINITY, f64::min);
            for g in &mut glyphs {
                g.x += x - origin;
            }
            x = origin;
            (i..j)
                .map(|k| b.xs[k] + b.atoms[k].width + dx)
                .fold(x, f64::max)
        } else {
            b.xs[j - 1] + b.atoms[j - 1].width + dx
        };
        let ink_end = if rtl {
            (i..j.min(last_ink))
                .map(|k| b.xs[k] + b.atoms[k].width + dx)
                .fold(x, f64::max)
        } else if j > last_ink {
            if last_ink > i {
                b.xs[last_ink - 1] + b.atoms[last_ink - 1].width + dx
            } else {
                x
            }
        } else {
            end_x
        };
        let width = end_x - x;
        let st = &a.style;
        let color = st
            .color
            .unwrap_or_else(|| auto_color(st.highlight.or(st.shading).or_else(background)));
        let by = baseline - a.rise();
        if let Some(bg) = st.highlight.or(st.shading)
            && let Some(path) = crate::office::pptx::textlayout::rect_path(
                x,
                by - a.face_ascent(),
                width,
                a.face_ascent() + a.face_descent(),
            )
        {
            backgrounds.push(fill_item(path, bg));
        }
        if let Some(bd) = st.bdr.as_ref() {
            // A box around runs sharing the border: sides only at its ends.
            let same = |k: usize| {
                b.atoms
                    .get(k)
                    .is_some_and(|o| o.style.bdr.as_ref() == Some(bd))
            };
            let (open, close) = {
                // Border joins follow touching visual edges after bidi placement.
                let joins = |edge: f64, left: bool| {
                    b.atoms.iter().enumerate().any(|(k, other)| {
                        !(i..j).contains(&k)
                            && other.width > 0.0
                            && same(k)
                            && ((b.xs[k] + dx + if left { other.width } else { 0.0 }) - edge).abs()
                                < 0.01
                    })
                };
                (!joins(x, true), !joins(end_x, false))
            };
            crate::office::docx::decor::run_border(
                bd,
                x,
                by - a.face_ascent(),
                width,
                a.face_ascent() + a.face_descent(),
                open,
                close,
                &mut decorations,
            );
        }
        let mut exported_text = text.clone();
        let mut preserve_whitespace = source_split_space && text.trim().is_empty();
        if text.trim().is_empty()
            && first_text.is_some_and(|first| i > first)
            && last_text.is_some_and(|last| j <= last)
        {
            if retain_interior_space(
                &mut items[line_start..],
                source_slot,
                &text,
                st.source.as_ref(),
                x,
                width,
            ) {
                exported_text.clear();
            } else {
                preserve_whitespace = true;
            }
        }
        if let Some(face) = a.face()
            && (!glyphs.is_empty() || preserve_whitespace)
        {
            let mut bookmarks = Vec::new();
            let exported_length = exported_text.encode_utf16().count();
            let mut offset = 0;
            for k in i..j {
                let atom = &b.atoms[k];
                let length = atom.text.encode_utf16().count();
                if let Some(extra) = &atom.x {
                    for (boundary, marker) in &extra.bookmarks {
                        let at = offset + boundary;
                        if at <= exported_length {
                            let fraction = *boundary as f64 / length.max(1) as f64;
                            let edge = atom.width
                                * if atom.bidi_level % 2 == 1 {
                                    1.0 - fraction
                                } else {
                                    fraction
                                };
                            bookmarks.push(crate::office::pptx::scene::BookmarkBoundary {
                                source: marker.clone(),
                                offset: at,
                                point: tiny_skia::Point::from_xy(
                                    (b.xs[k] + dx + edge - x) as f32,
                                    0.0,
                                ),
                            });
                            bound_bookmarks.insert(marker.id.clone());
                        }
                    }
                }
                offset += length;
            }
            let source_spans = run_source_spans(b, i, j, x, dx);
            items.push(Item::Text(GlyphRun {
                bookmarks: (!bookmarks.is_empty() || !source_spans.is_empty()).then(|| {
                    Box::new(crate::office::pptx::scene::RunBookmarks {
                        boundaries: bookmarks,
                        source_spans,
                    })
                }),
                source: st.source.clone(),
                face,
                font_name: env
                    .fonts
                    .face(face)
                    .map(|f| f.family.clone())
                    .unwrap_or_default(),
                size: a.draw_size(),
                color,
                glyphs,
                transform: Transform::from_translate(x as f32, by as f32),
                text: exported_text,
                preserve_whitespace,
                width,
                ascent: a.face_ascent(),
                descent: a.face_descent(),
                outline: None,
                fake_bold: false,
                fake_italic: st.italic && !env.fonts.is_italic(face),
                shape: None,
                paragraph: source_slot,
                hyperlink: None,
                space_before: prev_space,
                drawn: true,
                warp: None,
            }));
        }
        let thick = (st.size / 18.0).max(0.5);
        if st.underline.as_deref() == Some("words") {
            // Only words are underlined: spaces are skipped.
            for k in i..j.min(last_ink.max(i)) {
                if b.atoms[k].kind == Kind::Text {
                    let x0 = b.xs[k] + dx;
                    underline(st, x0, x0 + b.atoms[k].width, by, color, &mut decorations);
                }
            }
        } else if st.underline.is_some() && ink_end > x {
            let ink_start = if rtl {
                (i..j.min(last_ink))
                    .map(|k| b.xs[k] + dx)
                    .fold(ink_end, f64::min)
            } else {
                x
            };
            underline(st, ink_start, ink_end, by, color, &mut decorations);
        }
        if st.strike && width > 0.0 {
            let y = by - a.face_ascent() * 0.3;
            if let Some(path) = crate::office::pptx::textlayout::rect_path(x, y, width, thick) {
                decorations.push(fill_item(path, color));
            }
        }
        prev_space = b.atoms[j - 1].kind == Kind::Space;
        i = j;
    }
    for (k, atom) in b.atoms.iter().enumerate() {
        if let Some(extra) = &atom.x {
            for (boundary, marker) in &extra.bookmarks {
                if bound_bookmarks.contains(&marker.id) {
                    continue;
                }
                let fraction = *boundary as f64 / atom.text.encode_utf16().count().max(1) as f64;
                let edge = atom.width
                    * if atom.bidi_level % 2 == 1 {
                        1.0 - fraction
                    } else {
                        fraction
                    };
                items.push(Item::Bookmark(crate::office::pptx::scene::BookmarkItem {
                    source: marker.clone(),
                    reading_key: 0,
                    point: tiny_skia::Point::from_xy((b.xs[k] + dx + edge) as f32, baseline as f32),
                }));
            }
        }
    }
    let mut out = backgrounds;
    out.append(items);
    out.extend(decorations);
    *items = out;
}

/// Only ownership changes allocate intervals; painted glyphs stay in one run.
fn run_source_spans(
    b: &LineBuild,
    start: usize,
    end: usize,
    x: f64,
    dx: f64,
) -> Vec<crate::office::pptx::scene::RunSourceSpan> {
    let atoms = &b.atoms[start..end];
    if !atoms.iter().any(|atom| {
        atom.style.source != atoms[0].style.source
            || atom
                .x
                .as_ref()
                .is_some_and(|extra| !extra.ownership.is_empty())
    }) {
        return Vec::new();
    }
    let mut spans = Vec::new();
    let (mut text, mut glyph) = (0, 0);
    for (k, atom) in atoms.iter().enumerate() {
        let count = if atom.kind == Kind::Text {
            atom.glyphs.as_slice().len()
        } else {
            0
        };
        let ownership = atom
            .x
            .as_ref()
            .map(|extra| extra.ownership.as_slice())
            .unwrap_or(&[]);
        let mut add = |range: std::ops::Range<usize>, source| {
            spans.push(crate::office::pptx::scene::RunSourceSpan {
                text: text + range.start..text + range.end,
                glyphs: glyph..glyph + count,
                x: b.xs[start + k] + dx - x,
                width: atom.width,
                source,
            })
        };
        if ownership.is_empty() {
            add(0..atom.text.len(), atom.style.source.clone());
        } else {
            for (range, source) in ownership {
                add(range.clone(), source.clone());
            }
        }
        text += atom.text.len();
        glyph += count;
    }
    spans
}

/// Mark ownership-split whitespace in one pass over contiguous visual groups.
/// The group scan is shared by every whitespace atom on arbitrarily wide lines.
fn source_split_spaces(b: &LineBuild, last_ink: usize) -> Vec<bool> {
    let mut keep = vec![false; b.atoms.len()];
    let eligible = |atom: &Atom| {
        matches!(atom.kind, Kind::Text | Kind::Space)
            && atom.anchor().is_none()
            && atom.ruby().is_none()
    };
    let same = |a: &Atom, other: &Atom| {
        if !eligible(other)
            || other.face() != a.face()
            || other.draw_size() != a.draw_size()
            || other.rise() != a.rise()
            || other.bidi_level != a.bidi_level
        {
            return false;
        }
        if other.style.source == a.style.source {
            return same_format(&other.style, &a.style);
        }
        let mut style = (*other.style).clone();
        style.source = a.style.source.clone();
        style.source_style = a.style.source_style.clone();
        same_format(&style, &a.style)
    };
    let mut start = 0;
    while start < b.atoms.len() {
        let a = &b.atoms[start];
        if !eligible(a) {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < b.atoms.len() && same(a, &b.atoms[end]) {
            end += 1;
        }
        let group = &b.atoms[start..end];
        if group.iter().any(|atom| atom.style.source != a.style.source) {
            let first = group
                .iter()
                .position(|atom| !atom.text.trim().is_empty())
                .map(|v| v + start);
            let last = group
                .iter()
                .rposition(|atom| !atom.text.trim().is_empty())
                .map(|v| v + start);
            for (offset, (atom, retain)) in group.iter().zip(&mut keep[start..end]).enumerate() {
                let i = start + offset;
                if !atom.text.trim().is_empty() {
                    continue;
                }
                let decorated = atom.style.underline.is_some()
                    || atom.style.highlight.is_some()
                    || atom.style.shading.is_some()
                    || (atom.style.strike && i < last_ink);
                *retain = first.is_some_and(|v| v < i)
                    || ((atom.kind != Kind::Space || decorated) && last.is_some_and(|v| v > i));
            }
        }
        start = end;
    }
    keep
}

// Interior whitespace belongs to the adjacent text in its logical source slot.
// Only exported Unicode changes; glyph placement and whitespace decoration stay intact.
fn retain_interior_space(
    items: &mut [Item],
    slot: usize,
    text: &str,
    source: Option<&std::sync::Arc<crate::office::pptx::scene::TextSource>>,
    x: f64,
    width: f64,
) -> bool {
    if let Some(run) = items.iter_mut().rev().find_map(|item| match item {
        Item::Text(run) if run.paragraph == slot && !run.text.trim().is_empty() => Some(run),
        _ => None,
    }) {
        if run.source.as_ref() != source {
            let internal =
                |source: Option<&std::sync::Arc<crate::office::pptx::scene::TextSource>>| {
                    source
                        .and_then(|source| source.link.as_ref())
                        .is_some_and(|link| {
                            matches!(
                                link.target,
                                crate::office::pptx::scene::SourceLinkTarget::Bookmark { .. }
                            )
                        })
                };
            if !(internal(run.source.as_ref()) || internal(source)) {
                return false;
            }
        }
        if run
            .bookmarks
            .as_ref()
            .is_some_and(|metadata| !metadata.source_spans.is_empty())
            || run.source.as_ref() != source
        {
            let length = run.text.len();
            let glyphs = run.glyphs.len();
            let metadata = run.bookmarks.get_or_insert_with(Default::default);
            if metadata.source_spans.is_empty() {
                metadata
                    .source_spans
                    .push(crate::office::pptx::scene::RunSourceSpan {
                        text: 0..length,
                        glyphs: 0..glyphs,
                        x: 0.0,
                        width: run.width,
                        source: run.source.clone(),
                    });
            }
            metadata
                .source_spans
                .push(crate::office::pptx::scene::RunSourceSpan {
                    text: length..length + text.len(),
                    glyphs: glyphs..glyphs,
                    x: x - run.transform.tx as f64,
                    width,
                    source: source.cloned(),
                });
        }
        run.text.push_str(text);
        true
    } else {
        false
    }
}

/// An underline of style `st.underline` from `x0` to `x1` under the baseline
/// `by`: line width size/18 (doubled for thick and heavy styles), dashes and
/// dots as segments, double as two lines.
fn underline(st: &RunStyle, x0: f64, x1: f64, by: f64, text_color: Rgba, out: &mut Vec<Item>) {
    let kind = st.underline.as_deref().unwrap_or("single");
    let color = st.underline_color.unwrap_or(text_color);
    let mut w = (st.size / 18.0).max(0.5);
    if kind.starts_with("thick") || kind.ends_with("Heavy") {
        w *= 2.0;
    }
    let y = by + st.size * 0.12;
    let mut bar = |a: f64, b: f64, y: f64| {
        if let Some(path) = crate::office::pptx::textlayout::rect_path(a, y, (b - a).max(0.0), w) {
            out.push(fill_item(path, color));
        }
    };
    if x1 <= x0 {
        return;
    }
    if kind == "double" || kind == "wavyDouble" {
        bar(x0, x1, y - w);
        bar(x0, x1, y + w);
        return;
    }
    // Dash pattern (on, off) in multiples of the line width.
    let pattern: Option<(f64, f64)> = match kind {
        "dotted" | "dottedHeavy" => Some((1.0, 1.0)),
        "dash" | "dashedHeavy" => Some((4.0, 3.0)),
        "dashLong" | "dashLongHeavy" => Some((8.0, 3.0)),
        "dotDash" | "dashDotHeavy" | "dotDotDash" | "dashDotDotHeavy" => Some((4.0, 2.0)),
        _ => None,
    };
    match pattern {
        Some((on, off)) => {
            let (on, off) = (on * w * 1.5, off * w * 1.5);
            let mut x = x0;
            while x < x1 {
                bar(x, (x + on).min(x1), y);
                x += on + off;
            }
        }
        None => bar(x0, x1, y),
    }
}

fn fill_item(path: tiny_skia::Path, color: Rgba) -> Item {
    Item::Path(PathItem {
        path,
        fill: Some(Paint::Solid(color)),
        stroke: None,
        has_curve: false,
        shape: None,
        from_text: true,
    })
}

/// Leader characters filling a tab.
fn leader_run(a: &Atom, x: f64, baseline: f64, leader: char, env: &TextEnv, items: &mut Vec<Item>) {
    let st = &a.style;
    let Some(face) = env.fonts.resolve(&st.font, st.bold, st.italic) else {
        return;
    };
    let Some(fd) = env.fonts.face(face) else {
        return;
    };
    let Some(f) = rustybuzz::ttf_parser::Face::parse((*fd.data).as_ref(), fd.index).ok() else {
        return;
    };
    let Some(gid) = f.glyph_index(leader) else {
        return;
    };
    let upm = fd.metrics.units_per_em as f64;
    let adv = f.glyph_hor_advance(gid).unwrap_or(0) as f64 * st.size / upm;
    if adv <= 0.1 {
        return;
    }
    // Leaders sit on a grid of their own advance, ending before the stop.
    let start = (x / adv).ceil() * adv;
    let mut glyphs = Vec::new();
    let mut gx = start;
    while gx + adv <= x + a.width + 0.01 {
        glyphs.push(Glyph {
            id: gid.0,
            x: gx - x,
            y: 0.0,
        });
        gx += adv;
    }
    if glyphs.is_empty() {
        return;
    }
    let text: String = std::iter::repeat_n(leader, glyphs.len()).collect();
    items.push(Item::Text(GlyphRun {
        bookmarks: None,
        source: st.source.clone(),
        preserve_whitespace: false,
        face,
        font_name: fd.family.clone(),
        size: st.size,
        color: st.color.unwrap_or(Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }),
        glyphs,
        transform: Transform::from_translate(x as f32, baseline as f32),
        text,
        width: a.width,
        ascent: fd.metrics.ascent as f64 * st.size / upm,
        descent: fd.metrics.descent as f64 * st.size / upm,
        outline: None,
        fake_bold: false,
        fake_italic: false,
        shape: None,
        paragraph: 0,
        hyperlink: None,
        space_before: true,
        drawn: true,
        warp: None,
    }));
}

/// With `compressPunctuation`, Word squeezes full-width CJK punctuation on a
/// line so that one more character fits: up to half an em of overflow, and no
/// more than half the width of the line's compressible marks (page-1 EMF of a
/// Chinese document: every line matches).
fn punct_slack(line: &[Atom], word: &[Atom], straddles: bool) -> f64 {
    if !crate::office::docx::metrics::compress_punct() {
        return 0.0;
    }
    let cap: f64 = line
        .iter()
        .chain(word)
        .filter(|a| crate::office::docx::metrics::compressible_punct(&a.text).is_some())
        .map(|a| a.width / 2.0)
        .sum();
    // Japanese text compresses more than Chinese (CJK set: 6 documents better, none worse).
    let lang = word
        .first()
        .and_then(|a| a.style.lang_ea.clone())
        .unwrap_or_else(crate::office::docx::metrics::doc_lang_ea);
    if lang.to_ascii_lowercase().starts_with("ja") {
        return punct_slack_ja(line, word);
    }
    // Up to half an em for one more character. A bracket glued to its neighbour
    // (an opening bracket cannot end a line, a closing one cannot start it) takes
    // all the room the line's punctuation gives instead of moving the pair down
    // (Word for Mac probe: 31 characters of "测测（测测）测测，" on a 415.6 pt line,
    // an 18.4 pt squeeze; a lone character, or a pair starting past the margin,
    // does not squeeze in).
    let bracket = |t: &str| {
        t.chars()
            .any(|c| "（）「」『』【】〔〕《》〈〉".contains(c))
    };
    // A "‧" hyphenation point on the line lets a comma pair squeeze in; Latin
    // letters, digits and an em dash do not (Word for Mac probe).
    let dotted = line.iter().any(|a| a.text.contains('\u{2027}'));
    let kana = |t: &str| t.chars().any(|c| ('\u{3040}'..='\u{30FF}').contains(&c));
    let glued = straddles
        && word.iter().filter(|a| !a.text.trim().is_empty()).count() > 1
        && (word.iter().any(|a| bracket(&a.text))
            || (!word.iter().any(|a| kana(&a.text))
                && dotted
                && word
                    .iter()
                    .any(|a| crate::office::docx::metrics::compressible_punct(&a.text).is_some())));
    if glued {
        return cap;
    }
    // After spaces the word takes all the room the line's punctuation gives (Word for
    // Mac probe: 0.9 em with two colons, 1.1 em with four).
    if line.last().is_some_and(|a| a.kind == Kind::Space) {
        return cap;
    }
    let em = word.first().map_or(0.0, |a| a.draw_size()) / 2.0;
    // After a Latin character the squeeze stops at half that character's width
    // (Word for Mac probe: 0.14 em after "/", 0.22 em after "a").
    let after_latin = line
        .iter()
        .rev()
        .find(|a| !a.text.trim().is_empty())
        .and_then(|a| {
            let last = a.text.chars().last()?;
            (script_of(last) == Script::Latin)
                .then(|| a.width / a.text.chars().count() as f64 / 2.0)
        });
    cap.min(em).min(after_latin.unwrap_or(f64::INFINITY))
}

/// Room compressPunctuation gives a Japanese line: the punctuation that ends the
/// line gives up half its width, and the punctuation inside it up to an em in all.
fn punct_slack_ja(line: &[Atom], word: &[Atom]) -> f64 {
    let all: Vec<&Atom> = line.iter().chain(word).collect();
    let (mut full, mut other) = (0.0, 0.0);
    for (k, a) in all.iter().enumerate() {
        if crate::office::docx::metrics::compressible_punct(&a.text).is_none() {
            continue;
        }
        if k + 1 == all.len() {
            full += a.width / 2.0;
        } else {
            other += a.width / 2.0;
        }
    }
    let em = word.first().map_or(0.0, |a| a.draw_size());
    full + other.min(em)
}

thread_local! {
    static BACKGROUND: std::cell::Cell<Option<Rgba>> = const { std::cell::Cell::new(None) };
    static AUTO_TEXT: std::cell::Cell<Option<Rgba>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` with `c` as the colour of automatic text (a shape style's font colour).
pub fn with_auto_text<T>(c: Option<Rgba>, f: impl FnOnce() -> T) -> T {
    let old = AUTO_TEXT.with(|a| a.replace(c));
    let out = f();
    AUTO_TEXT.with(|a| a.set(old));
    out
}

/// The shading behind the text being laid out (table cell or paragraph).
pub fn background() -> Option<Rgba> {
    BACKGROUND.with(|b| b.get())
}

/// Runs `f` with `bg` as the shading behind the text, when it has one.
pub fn with_background<T>(bg: Option<Rgba>, f: impl FnOnce() -> T) -> T {
    let Some(bg) = bg else {
        return f();
    };
    let old = BACKGROUND.with(|b| b.replace(Some(bg)));
    let out = f();
    BACKGROUND.with(|b| b.set(old));
    out
}

/// Word's automatic text colour: white on dark shading, black otherwise.
pub fn auto_color(bg: Option<Rgba>) -> Rgba {
    if let Some(c) = AUTO_TEXT.with(|a| a.get()) {
        return c;
    }
    let dark = bg.is_some_and(|c| c.a > 0.5 && 0.299 * c.r + 0.587 * c.g + 0.114 * c.b < 0.4f32);
    let v = if dark { 1.0 } else { 0.0 };
    Rgba {
        r: v,
        g: v,
        b: v,
        a: 1.0,
    }
}

/// Moves an item by (dx, dy).
/// Text output order. In docx a glyph run's `paragraph` field holds its reading-order
/// key, not a paragraph index (docx runs have no `shape`, the only reader of the
/// index): body lines count up per page, anchored objects follow their anchor line,
/// headers come first (0), notes after the body and footers last.
pub(crate) fn reading_key(item: &Item) -> Option<usize> {
    match item {
        Item::Text(run) => Some(run.paragraph),
        Item::Bookmark(marker) => Some(marker.reading_key),
        _ => None,
    }
}

pub(crate) fn reading_key_mut(item: &mut Item) -> Option<&mut usize> {
    match item {
        Item::Text(run) => Some(&mut run.paragraph),
        Item::Bookmark(marker) => Some(&mut marker.reading_key),
        _ => None,
    }
}

thread_local! {
    /// Per layout thread: the line grid of the section being laid out, when the
    /// paginator has set one (`None` falls back to `TextEnv::grid`).
    static SECTION_GRID: std::cell::Cell<Option<Option<f64>>> = const { std::cell::Cell::new(None) };
}

/// Makes `grid` the line grid of the section laid out next; `None` clears it.
pub(crate) fn set_section_grid(grid: Option<Option<f64>>) {
    SECTION_GRID.with(|g| g.set(grid));
}

/// The document grid line pitch in force: the current section's, else the
/// environment's.
fn line_grid(env: &TextEnv) -> Option<f64> {
    SECTION_GRID.with(|g| g.get()).unwrap_or(env.grid)
}

/// Transfer a suppressed terminal line's boundaries to its accepted predecessor edge.
pub(crate) fn pop_empty_terminal_line(lines: &mut Vec<Line>) {
    let Some(last) = lines.pop() else {
        return;
    };
    if let Some(previous) = lines.last_mut() {
        for mut item in last.items {
            if let Item::Bookmark(marker) = &mut item {
                marker.point.y = previous.height as f32;
                previous.items.push(item);
            }
        }
    }
}

pub(crate) fn paint_empty(items: &[Item]) -> bool {
    items.iter().all(|item| matches!(item, Item::Bookmark(_)))
}

pub fn set_reading_key(items: &mut [Item], key: usize) {
    for it in items {
        match it {
            Item::Text(r) => r.paragraph = key,
            Item::Bookmark(marker) => marker.reading_key = key,
            _ => {}
        }
    }
}

/// Reading keys for notes and footers, after every body key.
pub const NOTE_KEY: usize = usize::MAX / 2;
pub const FOOTER_KEY: usize = usize::MAX;

pub fn translate_item(item: &mut Item, dx: f64, dy: f64) {
    let (dx, dy) = (dx as f32, dy as f32);
    match item {
        Item::Text(r) => r.transform = r.transform.post_translate(dx, dy),
        Item::Image(i) => {
            i.transform = i.transform.post_translate(dx, dy);
            if let Some(c) = i.clip.take() {
                i.clip = c.transform(Transform::from_translate(dx, dy));
            }
        }
        Item::Path(_) => {
            crate::office::docx::layout::transform_item(item, Transform::from_translate(dx, dy))
        }
        Item::PushClip(path) => {
            if let Some(moved) = path.clone().transform(Transform::from_translate(dx, dy)) {
                *path = moved;
            }
        }
        _ => crate::office::docx::layout::transform_item(item, Transform::from_translate(dx, dy)),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod bookmarks_tests;
