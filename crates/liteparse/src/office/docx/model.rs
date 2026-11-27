//! The parsed document: sections of blocks (paragraphs and tables) with styles
//! already resolved, ready for layout. Lengths are in points.

use std::rc::Rc;

use crate::office::docx::props::Borders;
use crate::office::docx::styles::{ParaStyle, RunStyle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakKind {
    Line,
    Page,
    Column,
}

/// A page-number field: replaced by the number when the page is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// Each carries the number format of a `\\*` switch (a `format_number` name).
    Page(Option<&'static str>),
    NumPages(Option<&'static str>),
    SectionPages(Option<&'static str>),
    /// A STYLEREF in a header or footer: an index into the document's style
    /// references, whose text each page sets from its nearest paragraph of the style.
    StyleRef(usize),
}

/// A picture placed in the text. `part` is `None` for drawings that are not
/// rendered yet (charts, shapes, SmartArt): they still take their room.
#[derive(Debug, Clone)]
pub struct Picture {
    pub part: Option<String>,
    pub w: f64,
    pub h: f64,
    /// Source crop as fractions of each edge (l, t, r, b).
    pub crop: (f64, f64, f64, f64),
    pub alt: Option<String>,
    /// A drawn shape or group (with its text boxes) instead of a picture.
    pub shape: Option<Rc<crate::office::docx::shapes::ShapeSpec>>,
    /// Recolouring (`a:grayscl`, `a:lum`, `a:duotone`, VML `grayscale`...).
    pub effects: Vec<crate::office::pptx::scene::BlipEffect>,
    /// `a:alphaModFix`: opacity 0..1.
    pub alpha: f32,
    /// A chart or diagram (`a:graphicData`) with the part it comes from,
    /// drawn into `shape` once fonts are known (`graphics.rs`).
    pub graphic: Option<(Rc<crate::office::pptx::xml::XNode>, String)>,
    /// `a:outerShdw` of the picture, drawn under it.
    pub shadow: Option<crate::office::pptx::scene::Shadow>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HRel {
    Page,
    Margin,
    Column,
    Character,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VRel {
    Page,
    Margin,
    Paragraph,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Wrap {
    None,
    Square,
    TopAndBottom,
}

/// A floating picture (`wp:anchor`), placed relative to its anchor paragraph.
#[derive(Debug, Clone)]
pub struct Anchored {
    pub pic: Picture,
    pub h_rel: HRel,
    /// Offset in points, or an alignment (`left`, `center`, `right`...).
    pub x: Result<f64, String>,
    pub v_rel: VRel,
    pub y: Result<f64, String>,
    pub behind: bool,
    /// Drawing order among floating objects (`relativeHeight`, VML `z-index`).
    pub z: i64,
    pub wrap: Wrap,
    /// DrawingML wrap polygon in fractions of the unrotated image extent.
    pub contour: Option<crate::office::docx::wrap::Contour>,
    /// Distance kept from text: top, bottom, left, right (points).
    pub dist: [f64; 4],
    /// `wrapText`: the sides text may run on (`bothSides`, `left`, `right`, `largest`).
    pub wrap_side: String,
    /// Text box contents laid out in the shape's local points.
    pub text_items: Vec<crate::office::pptx::scene::Item>,
    /// Anchored drawings inside a text frame, deferred until its page is known.
    pub frame_children: Vec<FrameAnchor>,
    /// Paragraph frame (`w:framePr`) provenance for frame-specific placement.
    pub text_frame: bool,
    /// Turned a quarter (45-135 or 225-315 degrees): alignment places the
    /// box with width and height swapped, centred on the unturned box.
    pub turned: bool,
}

/// A drawing's anchor origins in its containing frame's point coordinates.
#[derive(Debug, Clone)]
pub struct FrameAnchor {
    /// Textbox flow coordinates mapped into the owning drawing, with local alignment width.
    pub placement: Option<(tiny_skia::Transform, f64)>,
    pub anchor: Anchored,
    pub column_x: f64,
    pub paragraph_y: f64,
    pub line_y: f64,
}

#[derive(Debug, Clone)]
pub enum Inline {
    Bookmark(std::sync::Arc<crate::office::pptx::scene::SourceBookmark>),
    Text(String, Rc<RunStyle>),
    Ruby(Box<Ruby>),
    Tab(Rc<RunStyle>),
    Break(BreakKind, Rc<RunStyle>),
    Picture(Picture, Rc<RunStyle>),
    /// Continuation rule resolved against the accepted story container width.
    ContinuationSeparator(Rc<RunStyle>),
    Field(FieldKind, Rc<RunStyle>),
    Anchor(Box<Anchored>),
    /// A footnote reference: the note (by id) goes to the bottom of the page.
    Note(i64),
}

/// Horizontal phonetic guide and its base, with resolved run formatting.
#[derive(Debug, Clone)]
pub struct Ruby {
    pub base: Vec<(String, Rc<RunStyle>)>,
    pub guide: Vec<(String, Rc<RunStyle>)>,
    pub align: String,
    /// Guide baseline displacement above the base baseline, in points.
    pub raise: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suffix {
    Tab,
    Space,
    Nothing,
}

#[derive(Debug, Clone)]
pub struct NumLabel {
    pub text: String,
    /// A resolved image replaces each Unicode scalar in the literal level pattern.
    pub picture: Option<Rc<Picture>>,
    pub picture_count: usize,
    pub picture_table: Option<(String, usize)>,
    pub style: Rc<RunStyle>,
    pub suffix: Suffix,
    /// Label alignment within the hanging indent (`lvlJc`).
    pub jc: String,
}

#[derive(Debug, Clone)]
pub struct Paragraph {
    /// Shared by paragraphs of equal formatting (see `Parser::intern_para`).
    pub style: Rc<ParaStyle>,
    pub inlines: Vec<Inline>,
    /// Formatting of the paragraph mark: sizes an empty paragraph and, in Word,
    /// takes part in the height of the last line.
    pub mark: Rc<RunStyle>,
    pub num: Option<NumLabel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VMerge {
    None,
    Restart,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeightRule {
    Auto,
    AtLeast,
    Exact,
}

#[derive(Debug, Clone)]
pub struct Cell {
    pub preferred_width: TableWidth,
    pub no_wrap: bool,
    pub blocks: Vec<Block>,
    /// Diagonal borders: top-left to bottom-right, top-right to bottom-left.
    pub diagonals: [Option<crate::office::docx::props::Border>; 2],
    pub span: usize,
    pub vmerge: VMerge,
    pub borders: Borders,
    /// top, left, bottom, right.
    pub margins: [f64; 4],
    pub shading: Option<crate::office::pptx::color::Rgba>,
    pub v_align: String,
    /// `w:textDirection` when the text runs vertically (`btLr`, `tbRl`...).
    pub text_dir: Option<String>,
    /// `w:hideMark`: the end-of-cell mark takes no height.
    pub hide_mark: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TableLayout {
    #[default]
    AutoFit,
    Fixed,
}

#[derive(Debug, Clone)]
pub struct Row {
    /// Effective table layout after this row's property exceptions.
    pub layout: TableLayout,
    pub cells: Vec<Cell>,
    pub height: f64,
    pub height_rule: HeightRule,
    pub cant_split: bool,
    pub header: bool,
    pub grid_before: usize,
    /// Table borders overridden for this row (`w:tblPrEx`).
    pub borders: Option<crate::office::docx::props::Borders>,
}

/// Preferred table width, independent of the fixed/AutoFit layout algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TableWidth {
    #[default]
    Auto,
    Dxa(f64),
    Percent(f64),
}

impl TableWidth {
    pub fn fraction(self) -> Option<f64> {
        match self {
            Self::Percent(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Table {
    /// Semantic container edges retained independently of cell content and flow height.
    pub bookmarks_before: Vec<std::sync::Arc<crate::office::pptx::scene::SourceBookmark>>,
    pub bookmarks_after: Vec<std::sync::Arc<crate::office::pptx::scene::SourceBookmark>>,
    pub layout: TableLayout,
    pub grid: Vec<f64>,
    pub rows: Vec<Row>,
    pub jc: String,
    pub indent: f64,
    pub borders: Borders,
    pub float: Option<TableFloat>,
    pub preferred_width: TableWidth,
    /// `w:tblCellSpacing` (points): space between cells and around them.
    pub cell_spacing: f64,
    /// `w:bidiVisual`: cells run right to left.
    pub bidi_visual: bool,
}

/// `w:tblpPr`: a floating table's position. Offsets in points, or an
/// alignment keyword (`center`, `top`...).
#[derive(Debug, Clone)]
pub struct TableFloat {
    pub h_anchor: String,
    pub x: Result<f64, String>,
    pub v_anchor: String,
    pub y: Result<f64, String>,
    /// `w:tblOverlap="never"`: the table may not overlap other floating objects.
    pub no_overlap: bool,
}

#[derive(Debug, Clone)]
pub enum Block {
    Para(Paragraph),
    /// Boxed: a table is several times larger than a paragraph.
    Table(Box<Table>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionStart {
    NextPage,
    Continuous,
    OddPage,
    EvenPage,
    NextColumn,
}

#[derive(Debug, Clone, Default)]
pub struct HdrFtrRefs {
    pub default: Option<String>,
    pub first: Option<String>,
    pub even: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FootnotePosition {
    #[default]
    PageBottom,
    BeneathText,
    /// Retained for diagnostics; Word does not support this source value.
    DocumentEnd,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FootnoteColumns {
    #[default]
    Auto,
    Count(u32),
    Invalid,
}

#[derive(Debug, Clone)]
pub struct SectPr {
    pub page_w: f64,
    pub page_h: f64,
    pub top: f64,
    pub bottom: f64,
    pub left: f64,
    pub right: f64,
    pub header: f64,
    pub footer: f64,
    pub gutter: f64,
    /// Footnote numbering (`w:footnotePr`): first number and format.
    pub note_start: Option<i64>,
    pub note_fmt: Option<String>,
    pub note_position: FootnotePosition,
    pub note_columns: FootnoteColumns,
    /// Section overrides for endnote numbering.
    pub endnote_start: Option<i64>,
    pub endnote_fmt: Option<String>,
    pub endnote_restart: Option<String>,
    pub cols: usize,
    pub col_space: f64,
    /// Explicit column widths and spacings (`w:col`), when not equal width.
    pub col_widths: Vec<(f64, f64)>,
    pub start: SectionStart,
    pub title_pg: bool,
    pub headers: HdrFtrRefs,
    pub footers: HdrFtrRefs,
    pub pg_start: Option<i64>,
    pub pg_fmt: Option<String>,
    /// `w:docGrid` type and line pitch.
    pub grid_type: Option<String>,
    pub grid_pitch: Option<f64>,
    /// Signed extra character advance, in points (charSpace / 4096).
    pub grid_char_space: Option<f64>,
    pub v_align: Option<String>,
    pub pg_borders: Option<crate::office::docx::decor::PageBorders>,
    pub line_numbers: Option<crate::office::docx::decor::LineNumbers>,
}

impl Default for SectPr {
    fn default() -> Self {
        // Word's defaults: US Letter, 1-inch margins, header and footer at 0.5 in.
        SectPr {
            page_w: 612.0,
            page_h: 792.0,
            top: 72.0,
            bottom: 72.0,
            left: 72.0,
            right: 72.0,
            header: 36.0,
            footer: 36.0,
            gutter: 0.0,
            note_start: None,
            note_fmt: None,
            note_position: FootnotePosition::default(),
            note_columns: FootnoteColumns::default(),
            endnote_start: None,
            endnote_fmt: None,
            endnote_restart: None,
            cols: 1,
            col_space: 36.0,
            col_widths: Vec::new(),
            start: SectionStart::NextPage,
            title_pg: false,
            headers: HdrFtrRefs::default(),
            footers: HdrFtrRefs::default(),
            pg_start: None,
            pg_fmt: None,
            grid_type: None,
            grid_pitch: None,
            grid_char_space: None,
            v_align: None,
            pg_borders: None,
            line_numbers: None,
        }
    }
}

impl SectPr {
    /// Column frames (x offset from the page left, width).
    pub fn columns(&self) -> Vec<(f64, f64)> {
        let body_w = (self.page_w - self.left - self.right - self.gutter).max(36.0);
        let x0 = self.left + self.gutter;
        if !self.col_widths.is_empty() {
            let mut x = x0;
            return self
                .col_widths
                .iter()
                .map(|(w, space)| {
                    let col = (x, w.max(9.0));
                    x += w + space;
                    col
                })
                .collect();
        }
        let n = self.cols.clamp(1, 45);
        let w = ((body_w - self.col_space * (n - 1) as f64) / n as f64).max(9.0);
        (0..n)
            .map(|i| (x0 + i as f64 * (w + self.col_space), w))
            .collect()
    }

    /// Additive character spacing; snapToChars needs unit alignment.
    pub fn character_grid(&self) -> Option<f64> {
        (self.grid_type.as_deref() == Some("linesAndChars"))
            .then_some(self.grid_char_space)
            .flatten()
    }

    /// Whether the document grid snaps lines (`lines` and `linesAndChars`).
    pub fn line_grid(&self) -> Option<f64> {
        match self.grid_type.as_deref() {
            Some("lines") | Some("linesAndChars") | Some("snapToChars") => {
                self.grid_pitch.filter(|p| *p > 1.0)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Section {
    pub props: SectPr,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    /// Default display-math justification from `m:mathPr/m:defJc`.
    pub math_justification: Option<String>,
    pub default_tab: f64,
    pub even_and_odd: bool,
    pub compat_mode: u32,
    pub do_not_expand_shift_return: bool,
    /// `w:doNotUseHTMLParagraphAutoSpacing`: paragraph spacing adds up.
    pub no_html_spacing: bool,
    pub grow_autofit: bool,
    /// Document-wide footnote numbering (`w:footnotePr` in settings).
    pub note_start: Option<i64>,
    pub note_fmt: Option<String>,
    /// Endnote numbering (`w:endnotePr` in settings).
    pub endnote_start: Option<i64>,
    pub endnote_fmt: Option<String>,
    pub endnote_restart: Option<String>,
    /// Document-wide placement; a section-level `pos` is ignored.
    pub endnote_pos: Option<String>,
    /// `w:adjustLineHeightInTable`: lines in table cells snap to the document grid.
    pub grid_in_table: bool,
    /// `w:noLeading`: line heights leave out the fonts' external leading.
    pub no_leading: bool,
    pub lang_ea: Option<String>,
    /// `w:themeFontLang/@w:bidi`: picks the theme's script font for an empty `a:cs`.
    pub lang_bidi: Option<String>,
}
