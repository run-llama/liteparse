//! The per-slide display list. Building a slide produces a `Scene`; the rasterizer
//! and every JSON output (text items, images, paths) read from the same scene, so
//! what is reported always matches what was drawn.
//!
//! All coordinates are slide points, top-left origin.

use crate::office::pptx::color::Rgba;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn union(&self, o: &Rect) -> Rect {
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = (self.x + self.w).max(o.x + o.w);
        let y1 = (self.y + self.h).max(o.y + o.h);
        Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
}

#[derive(Debug, Clone)]
pub struct GradientStop {
    pub pos: f32,
    pub color: Rgba,
}

#[derive(Debug, Clone)]
pub enum Paint {
    Solid(Rgba),
    /// Linear gradient between two points in slide coordinates.
    Linear {
        start: (f64, f64),
        end: (f64, f64),
        stops: Vec<GradientStop>,
    },
    /// Radial gradient centred at `center`, reaching `radius` at the last stop.
    Radial {
        center: (f64, f64),
        radius: f64,
        stops: Vec<GradientStop>,
    },
    /// A picture fill stretched over `rect`, given in a frame that `frame` maps to
    /// slide coordinates (identity unless the fill turns with a rotated or flipped
    /// shape, `rot_with_shape`; otherwise moving it keeps it upright).
    Image {
        image: ImageRef,
        rect: Rect,
        tile: Option<Tile>,
        alpha: f32,
        frame: tiny_skia::Transform,
        rot_with_shape: bool,
    },
    /// Two-color hatch pattern (`a:pattFill`).
    Pattern {
        preset: String,
        fg: Rgba,
        bg: Rgba,
    },
    /// Path gradient (`a:path`: circle, rect, shape), see `pathgrad`.
    Path(Box<crate::office::pptx::pathgrad::PathGradient>),
}

impl Paint {
    /// A representative flat color, used for JSON outputs.
    pub fn representative(&self) -> Option<Rgba> {
        match self {
            Paint::Solid(c) => Some(*c),
            Paint::Linear { stops, .. } | Paint::Radial { stops, .. } => {
                stops.first().map(|s| s.color)
            }
            Paint::Path(g) => g.stops.first().map(|s| s.color),
            Paint::Pattern { fg, .. } => Some(*fg),
            Paint::Image { .. } => None,
        }
    }

    pub fn is_visible(&self) -> bool {
        match self {
            Paint::Solid(c) => c.a > 0.0,
            Paint::Linear { stops, .. } | Paint::Radial { stops, .. } => {
                stops.iter().any(|s| s.color.a > 0.0)
            }
            Paint::Path(g) => g.stops.iter().any(|s| s.color.a > 0.0),
            Paint::Image { alpha, .. } => *alpha > 0.0,
            Paint::Pattern { fg, bg, .. } => fg.a > 0.0 || bg.a > 0.0,
        }
    }
}

/// `a:tile`: the picture repeats at its natural size times (sx, sy), offset by
/// (tx, ty) points from the fill origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    pub sx: f64,
    pub sy: f64,
    pub tx: f64,
    pub ty: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    Flat,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrowKind {
    None,
    Triangle,
    Stealth,
    Diamond,
    Oval,
    Arrow,
}

#[derive(Debug, Clone, Copy)]
pub struct Arrow {
    pub kind: ArrowKind,
    /// Width and length multipliers of the line width (sm=2, med=3, lg=5).
    pub w: f64,
    pub len: f64,
}

#[derive(Debug, Clone)]
pub struct Stroke {
    pub paint: Paint,
    pub width: f64,
    /// Dash pattern in multiples of the line width; empty = solid.
    pub dash: Vec<f64>,
    pub cap: LineCap,
    pub join: LineJoin,
    pub head: Option<Arrow>,
    pub tail: Option<Arrow>,
}

/// Index of the source shape in `Scene::shapes`.
pub type ShapeIdx = usize;

#[derive(Debug, Clone)]
pub struct PathItem {
    pub path: tiny_skia::Path,
    pub fill: Option<Paint>,
    pub stroke: Option<Stroke>,
    /// Path outline uses curves (for the `hasCurve` output flag).
    pub has_curve: bool,
    pub shape: Option<ShapeIdx>,
    /// True for decorations synthesized from text (underline, highlight).
    pub from_text: bool,
}

/// An image part referenced by the scene. Bytes are loaded by part name, then
/// the blip's recolor effects are applied in order.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRef {
    pub part: String,
    pub effects: Vec<BlipEffect>,
}

/// Picture recoloring from `a:blip` children.
#[derive(Debug, Clone, PartialEq)]
pub enum BlipEffect {
    /// Maps luminance onto a ramp from the first color (dark) to the second.
    Duotone(Rgba, Rgba),
    Grayscale,
    /// Brightness and contrast, each -1..1.
    Lum {
        bright: f32,
        contrast: f32,
    },
    /// Pixels of `from` become `to` (usually transparent).
    ClrChange {
        from: Rgba,
        to: Rgba,
    },
    /// Black and white at a luminance threshold (0..1).
    BiLevel(f32),
}

#[derive(Debug, Clone)]
pub struct ImageItem {
    pub image: ImageRef,
    /// Maps the unit square (0..1, 0..1) onto the placed image in slide space.
    pub transform: tiny_skia::Transform,
    /// Source crop as fractions of each edge (l, t, r, b); negative extends.
    pub crop: (f64, f64, f64, f64),
    pub alpha: f32,
    /// Optional clip path (the shape geometry) in slide coordinates.
    pub clip: Option<tiny_skia::Path>,
    pub shape: Option<ShapeIdx>,
    pub alt_text: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct Glyph {
    pub id: u16,
    /// Pen position (baseline origin) in run-local space: x along the baseline.
    pub x: f64,
    pub y: f64,
}

/// One source-owned UTF-8 text interval and its intersecting shaped clusters.
#[derive(Debug, Clone)]
pub struct RunSourceSpan {
    pub text: std::ops::Range<usize>,
    pub glyphs: std::ops::Range<usize>,
    pub x: f64,
    pub width: f64,
    pub source: Option<std::sync::Arc<TextSource>>,
}

/// A run of glyphs sharing one face, size and color on one line.
#[derive(Debug, Clone)]
pub struct GlyphRun {
    /// Source boundaries expressed as UTF-16 offsets in this exported run.
    pub bookmarks: Option<Box<RunBookmarks>>,
    /// Source-owned structure retained through DOCX reflow and page fragmentation.
    pub source: Option<std::sync::Arc<TextSource>>,
    pub face: fontdb::ID,
    pub font_name: String,
    pub size: f64,
    pub color: Rgba,
    pub glyphs: Vec<Glyph>,
    /// Maps run-local coordinates (baseline at y=0, x from the run start) to slide space.
    pub transform: tiny_skia::Transform,
    pub text: String,
    /// Retain source whitespace with its own logical position in text export.
    pub preserve_whitespace: bool,
    /// Advance width of the run in points.
    pub width: f64,
    pub ascent: f64,
    pub descent: f64,
    /// Glyph outline (`a:ln` on the run): colour and width in points.
    pub outline: Option<(Rgba, f64)>,
    /// Synthetic bold/italic when the resolved face lacks the style.
    pub fake_bold: bool,
    pub fake_italic: bool,
    pub shape: Option<ShapeIdx>,
    /// Paragraph index within the shape's text body.
    pub paragraph: usize,
    pub hyperlink: Option<String>,
    /// Starts a new word relative to the previous run on the same line.
    pub space_before: bool,
    /// False for text whose pixels are already in a picture (metafile text):
    /// reported as a text item but not drawn again.
    pub drawn: bool,
    /// WordArt warp: glyphs are drawn through it, and `transform` then maps the
    /// run onto its warped bounding box (for text items) rather than placing glyphs.
    pub warp: Option<std::sync::Arc<crate::office::pptx::warp::RunWarp>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSource {
    pub part: String,
    pub paragraph: usize,
    pub cells: Vec<SourceCell>,
    pub link: Option<SourceLink>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCell {
    pub table: usize,
    pub row: usize,
    pub cell: usize,
    pub span: usize,
    pub cell_count: usize,
    pub cell_spans: Vec<usize>,
    pub grid_before: usize,
    pub grid_after: usize,
    pub vertical_merges: Vec<SourceVerticalMerge>,
    pub fragment: Option<Box<SourceCell>>,
    pub text_only: bool,
}

/// A source row's participation in a vertical merge, in logical grid columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceVerticalMerge {
    pub cell: usize,
    pub grid_start: usize,
    pub span: usize,
    pub start_row: usize,
    pub end_row: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLink {
    pub id: String,
    pub target: SourceLinkTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceLinkTarget {
    ExternalUri(String),
    Bookmark { part: String, name: String },
}

/// A source bookmark boundary, independent of visible glyphs and page placement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBookmark {
    /// Block boundaries have a story/container owner without a paragraph owner.
    pub block_boundary: bool,
    pub id: String,
    pub xml_id: String,
    pub name: String,
    pub source: TextSource,
}

/// Optional semantic metadata keeps ordinary glyph runs free of bookmark allocations.
#[derive(Debug, Clone, Default)]
pub struct RunBookmarks {
    pub source_spans: Vec<RunSourceSpan>,
    pub boundaries: Vec<BookmarkBoundary>,
}

#[derive(Debug, Clone)]
pub struct BookmarkBoundary {
    pub source: std::sync::Arc<SourceBookmark>,
    pub offset: usize,
    pub point: tiny_skia::Point,
}

#[derive(Debug, Clone)]
pub struct BookmarkItem {
    pub reading_key: usize,
    pub source: std::sync::Arc<SourceBookmark>,
    pub point: tiny_skia::Point,
}

#[derive(Debug, Clone)]
pub enum Item {
    /// Zero-width source boundary; transforms with content and emits no paint.
    Bookmark(BookmarkItem),
    /// A blurred, offset silhouette drawn under the item that follows it.
    Shadow(ShadowItem),
    Path(PathItem),
    /// `a:fillOverlay blend="mult"`: the path's fill multiplied over what is beneath.
    Multiply(PathItem),
    Image(ImageItem),
    Text(GlyphRun),
    /// Restricts following items to a clip path until the matching `PopClip`.
    PushClip(tiny_skia::Path),
    PopClip,
    /// `a:softEdge`: what is drawn until the matching `PopSoftEdge` fades to
    /// transparent inward from the silhouette's outline.
    PushSoftEdge(SoftEdge),
    PopSoftEdge,
}

/// A soft-edge group: the silhouette (slide space) and the fade width in points.
#[derive(Debug, Clone)]
pub struct SoftEdge {
    pub path: tiny_skia::Path,
    pub radius: f64,
}

/// `a:outerShdw`: color (with alpha), blur radius and offset in points.
/// `a:glow` uses the same item with `glow` set: no offset, and `blur` is the
/// distance the color reaches outside the silhouette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub color: Rgba,
    pub blur: f64,
    pub dx: f64,
    pub dy: f64,
    pub glow: bool,
    /// The shape's `a:softEdge` radius (points, 0 for none): the silhouette is
    /// faded like the shape before it is blurred or grown.
    pub soft: f64,
}

#[derive(Debug, Clone)]
pub enum ShadowSource {
    /// A filled outline in slide space.
    Path(tiny_skia::Path),
    Glyphs(GlyphRun),
    /// A picture's opaque pixels (glows follow the picture's transparency).
    Image(Box<ImageItem>),
}

#[derive(Debug, Clone)]
pub struct ShadowItem {
    pub source: ShadowSource,
    pub shadow: Shadow,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphInfo {
    pub level: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bullet: Option<String>,
    /// Half-open range into the page's `text.items`.
    pub text_item_range: [usize; 2],
}

#[derive(Debug, Clone)]
pub struct ShapeInfo {
    pub id: String,
    pub name: String,
    pub parent: Option<ShapeIdx>,
    pub kind: &'static str,
    pub placeholder: Option<String>,
    pub bounds: Rect,
    pub rotation: f64,
    pub alt_text: Option<String>,
    pub hyperlink: Option<String>,
    /// Runs per paragraph, filled when text items are emitted.
    pub paragraphs: Vec<(u32, Option<String>)>,
    /// From master or layout (decoration), not the slide itself.
    pub inherited: bool,
    /// `cNvPr hidden="1"`: not drawn; its text is emitted as invisible items.
    pub hidden: bool,
}

#[derive(Debug, Clone)]
pub struct TableCell {
    pub text: String,
    pub bounds: Rect,
    pub row_span: u32,
    pub col_span: u32,
}

#[derive(Debug, Clone)]
pub struct TableInfo {
    pub shape: ShapeIdx,
    pub bounds: Rect,
    pub rows: Vec<Vec<TableCell>>,
}

#[derive(Debug, Clone)]
pub struct LinkInfo {
    pub uri: String,
    pub rect: Rect,
}

#[derive(Debug, Default)]
pub struct Scene {
    pub width: f64,
    pub height: f64,
    pub items: Vec<Item>,
    pub shapes: Vec<ShapeInfo>,
    pub tables: Vec<TableInfo>,
    pub links: Vec<LinkInfo>,
    pub charts: Vec<serde_json::Value>,
    pub smart_art: Vec<serde_json::Value>,
    pub media: Vec<serde_json::Value>,
    pub warnings: Vec<(String, String)>,
}
