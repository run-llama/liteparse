//! Pagination: places layout units on pages section by section, with Word's
//! rules for page and column breaks, widow/orphan control, keep-with-next,
//! keep-lines-together and table rows split across pages (repeating header
//! rows). Placement is computed first and drawn afterwards, so a keep chain
//! that fails can move back and restart from an earlier unit.

use std::collections::{HashMap, HashSet};

use crate::office::pptx::scene::{ImageItem, ImageRef, Item};
use tiny_skia::Transform;

use crate::office::docx::layout::{Flow, RowUnit, Unit, para_decor, split_row_in_area, stack};
use crate::office::docx::model::{Anchored, Block, HRel, SectPr, Section, SectionStart, VRel};
use crate::office::docx::text::{FieldValues, TextEnv, translate_item};

const EPS: f64 = 0.01;

/// A leading column control has no painted line, but retains paragraph spacing.
fn leading_column_transition(p: &crate::office::docx::layout::ParaUnit, line: usize) -> bool {
    line == 0
        && p.decor.is_none()
        && p.frame_anchors.is_empty()
        && p.lines.first().is_some_and(|l| {
            l.break_after == Some(crate::office::docx::model::BreakKind::Column)
                && l.text.is_empty()
                && crate::office::docx::text::paint_empty(&l.items)
                && l.anchors.is_empty()
                && l.notes.is_empty()
                && l.boxes.is_empty()
        })
        && p.lines.iter().skip(1).any(|l| !l.text.trim().is_empty())
}

#[derive(Debug, Clone)]
pub struct PageInfo {
    pub section: usize,
    pub w: f64,
    pub h: f64,
    pub top: f64,
    pub bottom: f64,
    pub cols: Vec<(f64, f64)>,
    /// Page number as counted (restarts honoured).
    pub number: i64,
    pub first_of_section: bool,
    /// Blank page inserted for an odd/even section start.
    pub blank: bool,
    /// Footnotes whose references are on this page, in order.
    pub notes: Vec<i64>,
    /// Final body geometry and reference owners, independent of header selection.
    pub note_regions: Vec<note_placement::NoteRegion>,
    pub note_areas: Vec<note_areas::NoteArea>,
    pub note_diagnostics: Vec<String>,
    /// The page's last footnote continues on the next page: (id, height shown here).
    pub note_cut: Option<(i64, f64)>,
    /// A footnote continued from the previous page: (id, offset where it resumes).
    pub note_carry: Option<(i64, f64)>,
}

/// Cached slice payload shrinks when ordinary or nested source content is consumed.
/// A paragraph whose last line ends in a page break starts the next unit on a
/// new page, so a keep-with-next chain cannot reach back across it.
fn ends_with_page_break(unit: &Unit) -> bool {
    matches!(unit, Unit::Para(p) if p.lines.last().is_some_and(|l| {
        matches!(l.break_after, Some(crate::office::docx::model::BreakKind::Page))
    }))
}

/// Merges a trailing `[page break]` + `[empty section mark]` pair, as two
/// paragraphs, two lines of one, or one line, into one empty line on the current
/// page; the next section's own page start follows.
fn drop_break_before_section_end(units: &mut Vec<Unit>) {
    let blank = |l: &crate::office::docx::text::Line| {
        crate::office::docx::text::paint_empty(&l.items) && l.anchors.is_empty()
    };
    let page_break = |l: &crate::office::docx::text::Line| {
        blank(l)
            && matches!(
                l.break_after,
                Some(crate::office::docx::model::BreakKind::Page)
            )
    };
    let n = units.len();
    let Some(Unit::Para(last)) = units.last_mut() else {
        return;
    };
    let m = last.lines.len();
    if m == 0 || !blank(&last.lines[m - 1]) {
        return;
    }
    if page_break(&last.lines[m - 1]) {
        last.lines[m - 1].break_after = None;
        return;
    }
    if last.lines[m - 1].break_after.is_some() {
        return;
    }
    if m >= 2 && page_break(&last.lines[m - 2]) {
        last.lines[m - 2].break_after = None;
        last.lines.pop();
        return;
    }
    if m == 1
        && n >= 2
        && let Unit::Para(prev) = &mut units[n - 2]
        && prev.lines.len() == 1
        && page_break(&prev.lines[0])
    {
        prev.lines[0].break_after = None;
        units.pop();
    }
}

fn row_payload_size(row: &RowUnit) -> usize {
    row.cells
        .iter()
        .flat_map(|cell| &cell.slices)
        .map(|slice| {
            1 + slice.anchors.len()
                + slice.notes.len()
                + slice
                    .items
                    .iter()
                    .map(|item| match item {
                        Item::Text(run) => run.text.len(),
                        _ => 1,
                    })
                    .sum::<usize>()
        })
        .sum()
}

/// Refresh note placement after later continuous sections have used existing pages.
/// Other page metadata still describes the section that created the body page.
fn refresh_note_metadata(out: &mut [OutPage], pages: &[PageInfo]) {
    for (page, info) in out.iter_mut().zip(pages) {
        page.info.notes = info.notes.clone();
        page.info.note_cut = info.note_cut;
        page.info.note_carry = info.note_carry;
        page.info.note_regions = info.note_regions.clone();
        page.info.note_areas = info.note_areas.clone();
        page.info.note_diagnostics = info.note_diagnostics.clone();
    }
}

pub struct OutPage {
    pub info: PageInfo,
    pub items: Vec<Item>,
}

#[derive(Clone)]
enum Placed {
    Line {
        unit: usize,
        line: usize,
        page: usize,
        column: usize,
        x: f64,
        y: f64,
        pad_top: f64,
        pad_bottom: f64,
        /// Top of the paragraph on this page: paragraph-relative objects count from it.
        para_y: f64,
        /// The paragraph laid out again beside a floating object.
        alt: Option<std::rc::Rc<crate::office::docx::layout::ParaUnit>>,
    },
    Row {
        row: Box<RowUnit>,
        merge_paint: Vec<merges::Paint>,
        page: usize,
        column: usize,
        normal_flow: bool,
        x: f64,
        y: f64,
    },
}

#[derive(Clone)]
struct State {
    body_merges: Vec<body_merges::Chain>,
    note_pages: Vec<std::rc::Rc<note_pages::PageNotes>>,
    pages: usize,
    placed: usize,
    col: usize,
    y: f64,
    region_top: f64,
    at_top: bool,
    hard_break: bool,
    // First content after a section transition, independent of header selection.
    // Forced page moves preserve it until a paragraph line or table row is placed.
    section_start: bool,
    /// Space after (points) of the last paragraph placed.
    last_after: f64,
    last_table: Option<(usize, usize)>,
    /// Floating objects on the current page that text may not run beside:
    /// (top, bottom, left, right) in page points.
    bands: Vec<(f64, f64, f64, f64)>,
    /// A floating table being placed: (table, flow y to return to, its top, page).
    float: Option<(usize, f64, f64, usize, f64, f64)>,
    /// Square-wrapped objects narrower than half the column: text runs beside them.
    side: Vec<(f64, f64, f64, f64)>,
    /// The `side` boxes of tight or through wrapped objects (contour wraps).
    contour_side: Vec<(f64, f64, f64, f64)>,
    /// The `side` boxes of floating tables.
    table_side: Vec<(f64, f64, f64, f64)>,
    /// The `side` boxes of objects placed relative to the page or its margins.
    fixed_side: Vec<(f64, f64, f64, f64)>,
    /// Column height cap while balancing columns before a continuous break.
    cap: Option<f64>,
    /// Surviving page snapshot for pagination retries and balancing replay.
    balance_page: Option<PageInfo>,
    /// Maximum normal-flow cursor on this page across all columns.
    flow_bottom: f64,
    /// Lowest declared edge of an eligible wrapped picture already on this page.
    picture_bottom: f64,
    picture_rects: Vec<(f64, f64, f64, f64)>,
}

/// Header and footer content per part, laid out on demand.
pub struct HdrFtr<'a> {
    pub blocks: &'a HashMap<String, Vec<Block>>,
    pub notes: &'a HashMap<i64, Vec<Block>>,
    pub separator: &'a Vec<Block>,
    pub continuation_separator: Option<&'a Vec<Block>>,
    pub continuation_notice: Option<&'a Vec<Block>>,
}

/// Floating objects per header/footer part and width: (object, x, line top).
type HfAnchors = HashMap<(String, u64, bool), Vec<(Anchored, f64, f64)>>;

pub struct Paginator<'a> {
    pub env: &'a TextEnv<'a>,
    pub compat_mode: u32,
    pub even_and_odd: bool,
    pub hf: HdrFtr<'a>,
    pages: Vec<PageInfo>,
    placed: Vec<Placed>,
    st: State,
    hf_heights: HashMap<(String, u64, bool), f64>,
    /// Floating objects of each header/footer part: (object, x, line top).
    hf_anchors: HfAnchors,
    /// The section being placed is the document's last.
    doc_last: bool,
    /// Header and footer STYLEREF fields: (style id, paragraph number wanted, saved
    /// result).
    style_refs: Vec<(Option<String>, bool, String)>,
    /// Paragraphs of a referenced style by (section, unit): (page, style id, text,
    /// number label). Keyed so that pagination retries replace earlier placements.
    headings: std::collections::BTreeMap<(usize, usize), (usize, String, String, String)>,
    /// The section being placed is followed by one that starts a new page.
    sect_breaks: bool,
    /// Laid-out footnotes (and the separator, id `i64::MIN`): slices and height.
    note_cache: std::cell::RefCell<HashMap<note_areas::NoteKey, note_fragments::NoteLayout>>,
    note_section: (usize, SectPr),
    legacy_note_sections: HashSet<usize>,
    /// Units whose floating objects stay where they were first placed: the copy has
    /// them page-absolute (see `lift_above`).
    pins: HashMap<usize, std::rc::Rc<crate::office::docx::layout::ParaUnit>>,
    /// Floating tables kept at their first top after moving earlier lines: table
    /// index to (page count when placed, top).
    float_pins: HashMap<usize, (usize, f64)>,
}

/// A placed line of a unit being pinned: (line, column, paragraph top, line top,
/// the paragraph copy it was laid out from).
type PinLine = (
    usize,
    usize,
    f64,
    f64,
    Option<std::rc::Rc<crate::office::docx::layout::ParaUnit>>,
);

impl<'a> Paginator<'a> {
    pub fn new(env: &'a TextEnv<'a>, compat_mode: u32, even_and_odd: bool, hf: HdrFtr<'a>) -> Self {
        Paginator {
            env,
            compat_mode,
            even_and_odd,
            hf,
            pages: Vec::new(),
            placed: Vec::new(),
            style_refs: Vec::new(),
            headings: std::collections::BTreeMap::new(),
            st: State {
                body_merges: Vec::new(),
                note_pages: Vec::new(),
                pages: 0,
                placed: 0,
                col: 0,
                y: 0.0,
                region_top: 0.0,
                at_top: true,
                hard_break: false,
                section_start: true,
                last_after: 0.0,
                last_table: None,
                bands: Vec::new(),
                float: None,
                side: Vec::new(),
                contour_side: Vec::new(),
                table_side: Vec::new(),
                fixed_side: Vec::new(),
                cap: None,
                balance_page: None,
                flow_bottom: 0.0,
                picture_bottom: 0.0,
                picture_rects: Vec::new(),
            },
            hf_heights: HashMap::new(),
            hf_anchors: HashMap::new(),
            doc_last: false,
            sect_breaks: false,
            note_cache: Default::default(),
            note_section: (0, SectPr::default()),
            legacy_note_sections: HashSet::new(),
            pins: HashMap::new(),
            float_pins: HashMap::new(),
        }
    }

    /// Lays out every section and returns the drawn pages.
    pub fn run(mut self, sections: &[Section]) -> Vec<OutPage> {
        self.legacy_note_sections = note_areas::mixed_geometry_sections(sections, self.hf.notes);
        let mut out: Vec<OutPage> = Vec::new();
        // Line numbering: (last numbered page, count).
        let mut ln_count = (usize::MAX, 0_i64);
        for (si, sec) in sections.iter().enumerate() {
            self.env.char_grid.set(sec.props.character_grid());
            crate::office::docx::text::set_section_grid(Some(sec.props.line_grid()));
            let flow = Flow::new(self.env, self.compat_mode);
            let cols = sec.props.columns();
            let width = cols.first().map_or(468.0, |c| c.1);
            let mut units = flow.units(&sec.blocks, width, false);
            self.st.body_merges = body_merges::chains(&units);
            // The empty paragraph holding a continuous section break takes no room
            // when the section has other content (Word for Mac probes: between
            // paragraphs and tables alike; a section of only that paragraph keeps
            // its line).
            let break_mark_hidden = sections
                .get(si + 1)
                .is_some_and(|n| n.props.start == SectionStart::Continuous);
            // A section-break paragraph before a section that starts a new page keeps
            // to the end of its own section: Word ignores its page break before (a
            // cover page whose break paragraph is styled TOC Heading).
            let next_page = sections.get(si + 1).is_some_and(|n| {
                matches!(
                    n.props.start,
                    SectionStart::NextPage | SectionStart::OddPage | SectionStart::EvenPage
                )
            });
            if let (true, Some(Unit::Para(p))) = (next_page, units.last_mut()) {
                p.page_break_before = false;
            }
            // A paragraph holding only a page break, then an empty section-break
            // paragraph before a new page: Word starts one page from mode 14 (mode
            // 11 files keep the blank page).
            if next_page && self.compat_mode >= 14 {
                drop_break_before_section_end(&mut units);
            }
            let mut suppressed_bookmarks = Vec::new();
            if break_mark_hidden
                && units.len() > 1
                && matches!(units.last(), Some(Unit::Para(p)) if p.lines.len() == 1
                    && crate::office::docx::text::paint_empty(&p.lines[0].items) && p.lines[0].anchors.is_empty())
                && let Some(Unit::Para(p)) = units.pop()
            {
                suppressed_bookmarks.extend(
                    p.lines
                        .into_iter()
                        .flat_map(|line| line.items)
                        .filter(|item| matches!(item, Item::Bookmark(_))),
                );
            }
            self.doc_last = si + 1 == sections.len();
            self.sect_breaks = sections
                .get(si + 1)
                .is_some_and(|n| n.props.start != SectionStart::Continuous);
            self.start_section(si, &sec.props, sections);
            let first_placed = self.placed.len();
            let snaps = self.place_units(&units, si, &sec.props);
            let next_continuous = sections
                .get(si + 1)
                .is_some_and(|n| n.props.start == SectionStart::Continuous);
            if next_continuous {
                self.balance(&units, &snaps, si, &sec.props);
                self.st.y = self.st.y.max(self.st.flow_bottom);
            }
            let placed: Vec<Placed> = self.placed.drain(first_placed..).collect();
            note_placement::record(&placed, &units, si, &sec.props, &mut self.pages);
            self.st.placed = self.placed.len();
            while out.len() < self.pages.len() {
                let info = self.pages[out.len()].clone();
                out.push(OutPage {
                    info,
                    items: Vec::new(),
                });
            }
            draw_placed(&placed, &mut units, &mut out);
            if let Some(page) = out.last_mut() {
                let x = self
                    .pages
                    .last()
                    .and_then(|page| page.cols.get(self.st.col))
                    .map_or(0.0, |column| column.0);
                let key = page
                    .items
                    .iter()
                    .filter_map(crate::office::docx::text::reading_key)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1);
                for mut item in suppressed_bookmarks {
                    if let Item::Bookmark(marker) = &mut item {
                        marker.point.x += x as f32;
                        marker.point.y = self.st.y as f32;
                        marker.reading_key = key;
                    }
                    page.items.push(item);
                }
            }
            if let Some(ln) = &sec.props.line_numbers {
                if ln.restart == "newSection" {
                    ln_count.1 = 0;
                }
                draw_line_numbers(&placed, &units, &mut out, ln, &mut ln_count, self.env);
            }
        }
        if out.is_empty() {
            let props = sections
                .first()
                .map(|s| s.props.clone())
                .unwrap_or_default();
            self.new_page(0, &props, sections);
            out.push(OutPage {
                info: self.pages[0].clone(),
                items: Vec::new(),
            });
        }
        self.drain_note_pages(sections, &mut out);
        {
            refresh_note_metadata(&mut out, &self.pages);
        }
        self.draw_headers(&mut out, sections);
        self.draw_notes(&mut out);
        crate::office::docx::text::set_section_grid(None);
        out
    }

    fn start_section(&mut self, si: usize, props: &SectPr, sections: &[Section]) {
        self.note_section = (si, props.clone());
        if self.pages.is_empty() {
            self.new_page(si, props, sections);
            self.st.section_start = true;
            return;
        }
        // A continuous break into a different page size starts a new page, down to
        // one twip (Word for Mac probe: 11906 to 11906 x 16839 twips breaks)
        let resized = self.pages.last().is_some_and(|p| {
            (p.w - props.page_w).abs() > 0.025 || (p.h - props.page_h).abs() > 0.025
        });
        match props.start {
            SectionStart::Continuous | SectionStart::NextColumn if resized => {
                self.new_page(si, props, sections)
            }
            SectionStart::Continuous | SectionStart::NextColumn => {
                // The new section's columns start below what is on the page;
                // its page setup applies from the next page.
                let page = self.pages.len() - 1;
                let cols = props.columns();
                self.pages[page].cols = cols;
                self.st.region_top = self.st.y;
                self.st.col = 0;
                if props.start == SectionStart::NextColumn && !self.st.at_top {
                    self.next_column(si, props, sections);
                }
            }
            SectionStart::NextPage => {
                // With even and odd headers, a section restarting its numbering starts
                // on a physical page of the same parity, after a blank page if needed
                // (page 3, then 1). A first section of type evenPage makes page 1 even.
                if let (true, Some(start)) = (self.even_and_odd, props.pg_start) {
                    let even_first = sections
                        .first()
                        .is_some_and(|s| s.props.start == SectionStart::EvenPage);
                    let physical = self.pages.len() as i64 + 1 + i64::from(even_first);
                    if (physical - start) % 2 != 0 {
                        self.blank_page(si, props, sections);
                    }
                }
                self.new_page(si, props, sections)
            }
            SectionStart::OddPage | SectionStart::EvenPage => {
                let want_odd = props.start == SectionStart::OddPage;
                // With even and odd headers, parity follows the pages so far, not a
                // restarted number: Word inserts a blank page before an odd-page section
                // that restarts at 1 on page 2.
                let next_number = self.next_number(props, !self.even_and_odd);
                if (next_number % 2 == 1) != want_odd {
                    self.blank_page(si, props, sections);
                }
                self.new_page(si, props, sections);
            }
        }
        self.st.hard_break = true;
        self.st.section_start = true;
    }

    /// An empty page Word inserts before a section to keep page parity. It
    /// continues the previous numbering; the section's first page follows it.
    fn blank_page(&mut self, si: usize, props: &SectPr, sections: &[Section]) {
        let number = self.next_number(props, false);
        self.new_page(si, props, sections);
        if let Some(p) = self.pages.last_mut() {
            p.blank = true;
            p.number = number;
            p.first_of_section = false;
        }
    }

    fn next_number(&self, props: &SectPr, first_of_section: bool) -> i64 {
        match (first_of_section, props.pg_start) {
            (true, Some(s)) => s,
            _ => self.pages.last().map_or(1, |p| p.number + 1),
        }
    }

    fn new_page(&mut self, si: usize, props: &SectPr, sections: &[Section]) {
        self.note_section = (si, props.clone());
        let first = self.pages.last().is_none_or(|p| p.section != si || p.blank);
        let number = self.next_number(props, first);
        let (hh, fh) = self.hf_extent(props, first, number, sections);
        let top = if props.top < 0.0 {
            -props.top
        } else {
            hh.map_or(props.top, |h| props.top.max(props.header + h))
        };
        let bottom = if props.bottom < 0.0 {
            props.page_h + props.bottom
        } else {
            props.page_h - fh.map_or(props.bottom, |f| props.bottom.max(props.footer + f))
        };
        // Oversized header/footer reservations retain a usable body interval.
        let bottom = bottom.max(top + 12.0);
        if std::env::var("DOCX_DEBUG").is_ok() {
            eprintln!("page {} top={top:.2} bottom={bottom:.2}", self.pages.len());
        }
        let carry = None;
        let incoming = self.st.note_pages.last().map_or_else(Vec::new, |p| {
            if self.pages.last().is_some_and(|p| p.blank) {
                p.incoming.clone()
            } else {
                p.outgoing.clone()
            }
        });
        self.pages.push(PageInfo {
            section: si,
            w: props.page_w,
            h: props.page_h,
            top,
            bottom,
            cols: props.columns(),
            number,
            first_of_section: first,
            blank: false,
            notes: Vec::new(),
            note_regions: Vec::new(),
            note_areas: Vec::new(),
            note_diagnostics: Vec::new(),
            note_cut: None,
            note_carry: carry,
        });
        self.st
            .note_pages
            .push(std::rc::Rc::new(note_pages::PageNotes::new(incoming, top)));
        self.st.col = 0;
        self.st.y = top;
        self.st.flow_bottom = top;
        self.st.picture_bottom = 0.0;
        self.st.picture_rects.clear();
        self.st.region_top = top;
        self.st.at_top = true;
        self.st.hard_break = false;
        self.st.bands.clear();
        self.st.side.clear();
        self.st.contour_side.clear();
        self.st.table_side.clear();
        self.st.fixed_side.clear();
        self.st.cap = None;
        self.hf_bands(props, first, number, hh, fh);
        self.seed_note_page();
    }

    /// Wrapped floating objects in the page's header and footer keep the body
    /// text off them, as objects in the body do (a logo's page-1 thumbnail).
    fn hf_bands(
        &mut self,
        props: &SectPr,
        first: bool,
        number: i64,
        hh: Option<f64>,
        fh: Option<f64>,
    ) {
        let (h, f) = self.hf_parts(props, first, number);
        let width = (props.page_w - props.left - props.right).max(36.0);
        let Some(page) = self.pages.last().cloned() else {
            return;
        };
        let parts = [
            (h, Some(props.header), false),
            (f, fh.map(|v| props.page_h - props.footer - v), true),
        ];
        let _ = hh;
        for (part, y0, footer) in parts {
            let (Some(part), Some(y0)) = (part, y0) else {
                continue;
            };
            let Some(list) = self.hf_anchors.get(&(part, width.to_bits(), footer)) else {
                continue;
            };
            for (a, ax, ay) in list {
                if a.wrap == crate::office::docx::model::Wrap::None
                    || (a.behind && !behind_wraps(a))
                {
                    continue;
                }
                let (x, y, w, hgt) = anchor_rect(a, &page, props.left + ax, y0 + ay, y0 + ay);
                let b = wrap_box(a, x, y, w, hgt);
                // Square-wrapped header and footer objects take the body's
                // line-by-line wrapping at every object width.
                if a.wrap == crate::office::docx::model::Wrap::TopAndBottom {
                    self.st
                        .bands
                        .push((b.0, b.1, f64::NEG_INFINITY, f64::INFINITY));
                } else {
                    let boxes = wrap_boxes(a, x, y, w, hgt);
                    if a.contour.is_some() {
                        self.st.contour_side.extend(boxes.iter().copied());
                    }
                    self.st.side.extend(boxes);
                }
            }
        }
    }

    fn next_column(&mut self, si: usize, props: &SectPr, sections: &[Section]) {
        let cols = self.pages.last().map_or(1, |p| p.cols.len());
        if self.st.col + 1 < cols {
            self.st.col += 1;
            self.st.y = self.st.region_top;
            self.st.at_top = true;
            self.st.hard_break = false;
        } else {
            self.new_page(si, props, sections);
        }
    }

    fn bottom(&self) -> f64 {
        let b = self.pages.last().map_or(720.0, |p| self.note_limit(p, &[]));
        self.st.cap.map_or(b, |c| b.min(c))
    }

    fn col_x(&self) -> f64 {
        self.pages
            .last()
            .and_then(|p| p.cols.get(self.st.col).or(p.cols.last()))
            .map_or(72.0, |c| c.0)
    }

    /// Resolve body table origins at the accepted destination, including retries
    /// and repeated headers, while retaining cell-local merge coordinates.
    fn row_x(&self, row: &RowUnit) -> f64 {
        match row.float_x.as_deref() {
            Some("page") => 0.0,
            Some("margin") => self
                .pages
                .last()
                .and_then(|page| page.cols.first())
                .map_or(self.col_x(), |column| column.0),
            _ => self.col_x(),
        }
    }

    fn snapshot(&self) -> State {
        State {
            pages: self.pages.len(),
            placed: self.placed.len(),
            balance_page: self.pages.last().cloned(),
            ..self.st.clone()
        }
    }

    fn restore(&mut self, s: &State) {
        self.pages.truncate(s.pages);
        if let (Some(page), Some(saved)) = (self.pages.last_mut(), &s.balance_page) {
            *page = saved.clone();
        }
        self.placed.truncate(s.placed);
        self.st = s.clone();
    }

    fn place_units(&mut self, units: &[Unit], si: usize, props: &SectPr) -> Vec<Option<State>> {
        let sections: &[Section] = &[];
        self.pins.clear();
        self.float_pins.clear();
        let mut snaps: Vec<Option<State>> = vec![None; units.len()];
        let mut forced: HashSet<usize> = HashSet::new();
        // Where each unit's first and last placement went: (page, column).
        let mut spots: Vec<((usize, usize), (usize, usize))> = vec![((0, 0), (0, 0)); units.len()];
        let mut i = 0;
        let mut guard = 0usize;
        // Units whose objects already moved the page's earlier text.
        let mut lifted: HashSet<usize> = HashSet::new();
        let mut lifted_tables: HashSet<usize> = HashSet::new();
        while i < units.len() {
            guard += 1;
            if guard > units.len() * 8 + 1000 {
                // Pathological keep chains: stop backtracking.
                forced.clear();
            }
            if let Some((t, ..)) = self.st.float
                && !matches!(&units[i], Unit::Row(r) if r.table == t)
            {
                let before = self.st.bands.len();
                self.end_float();
                let band = self.st.bands.get(before).copied();
                let r0 = (0..i)
                    .rev()
                    .take_while(|&k| matches!(&units[k], Unit::Row(r) if r.table == t))
                    .last();
                if let (Some(band), Some(r0)) = (band, r0)
                    && guard < units.len() * 8 + 1000
                    && lifted_tables.insert(t)
                    && let Some(s) = self.lift_float(units, &snaps, r0, t, band)
                {
                    i = s;
                    continue;
                }
            }
            snaps[i] = Some(self.snapshot());
            let force = forced.contains(&i);
            let first_spot = match &units[i] {
                Unit::Para(_) => self.place_para(units, i, force, si, props, sections),
                Unit::Row(r) => self.place_row(r, force, si, props, sections),
            };
            // After a clearing break the text resumes below the floats beside it.
            if matches!(&units[i], Unit::Para(p) if p.clear_after) {
                self.end_float();
                let below = self
                    .st
                    .side
                    .iter()
                    .filter(|s| s.0 < self.st.y + 0.01)
                    .map(|s| s.1)
                    .fold(self.st.y, f64::max);
                self.st.y = below;
            }
            let last_spot = (self.pages.len().saturating_sub(1), self.st.col);
            spots[i] = (first_spot, last_spot);
            self.note_heading(si, i, &units[i], first_spot.0);
            if !lifted.contains(&i)
                && guard < units.len() * 8 + 1000
                && let Some(s) = self.lift_above(units, &snaps, i)
            {
                lifted.insert(i);
                i = s;
                continue;
            }
            // Keep with next: this unit's first line must share the page (and
            // column) of the previous keep-with-next unit's last line, unless it
            // breaks the page itself (an empty keep-with-next paragraph before a
            // page-break-before one stays on its page, as in Word).
            if i > 0
                && !matches!(&units[i], Unit::Row(r) if r.page_break_before)
                && !matches!(&units[i], Unit::Para(p) if p.page_break_before)
                && units[i - 1].keep_next()
                && !ends_with_page_break(&units[i - 1])
                && spots[i - 1].1 != first_spot
                && guard < units.len() * 8 + 1000
            {
                let mut s = i - 1;
                while s > 0 && units[s - 1].keep_next() && !ends_with_page_break(&units[s - 1]) {
                    s -= 1;
                }
                // Only move the chain when it does not already start a page.
                let start_at_top = snaps[s].as_ref().is_some_and(|st| st.at_top);
                if !start_at_top && !forced.contains(&s) {
                    forced.insert(s);
                    let snap = snaps[s].clone().unwrap();
                    self.restore(&snap);
                    i = s;
                    continue;
                }
            }
            i += 1;
        }
        self.end_float();
        snaps
    }

    /// A blocking object anchored in unit `i` but placed above where the unit
    /// starts (page- or margin-relative) pushes the text placed earlier on the
    /// page below it, as Word lays the page out again: restores the state at the
    /// page's first unit with the object's band and returns that unit
    fn lift_above(&mut self, units: &[Unit], snaps: &[Option<State>], i: usize) -> Option<usize> {
        let start = snaps[i].as_ref()?;
        if self.pages.len() != start.pages || self.st.col != start.col {
            return None;
        }
        let new: Vec<(f64, f64, f64, f64)> = self
            .st
            .bands
            .iter()
            .skip(start.bands.len())
            .filter(|b| b.0 < start.y - EPS)
            .copied()
            .collect();
        // Square-wrapped page- or margin-placed objects narrow the earlier lines
        // they reach (Word for Mac: a letterhead line wraps around a page-placed
        // frame anchored after it).
        let new_side: Vec<(f64, f64, f64, f64)> = self
            .st
            .fixed_side
            .iter()
            .skip(start.fixed_side.len())
            .filter(|b| b.0 < start.y - EPS)
            .copied()
            .collect();
        if new.is_empty() && new_side.is_empty() {
            return None;
        }
        let s = Self::page_first_unit(snaps, start, i);
        let first = snaps[s].clone()?;
        let rects: Vec<_> = new.iter().chain(&new_side).copied().collect();
        let hit = self.overlaps_placed(units, &first, start, &rects, false);
        if s == i || !hit {
            return None;
        }
        self.pin_objects(units, start, i);
        let mut snap = first;
        snap.bands.extend(new);
        snap.side.extend(new_side.iter().copied());
        snap.fixed_side.extend(new_side);
        self.restore(&snap);
        Some(s)
    }

    /// The first unit placed in the page and column where unit `i` starts (one
    /// earlier when that unit began on the previous page and ran on to this one).
    fn page_first_unit(snaps: &[Option<State>], start: &State, i: usize) -> usize {
        let mut s = i;
        while s > 0
            && snaps[s - 1]
                .as_ref()
                .is_some_and(|x| x.pages == start.pages && x.col == start.col && !x.at_top)
        {
            s -= 1;
        }
        if s > 0
            && snaps[s - 1]
                .as_ref()
                .is_some_and(|x| x.pages == start.pages && x.col == start.col)
        {
            s -= 1;
        }
        s
    }

    /// Whether a line or row placed between `first` and `start` reaches into one of
    /// `rects` (top, bottom, left, right) vertically. With `text_only`, a line's
    /// room below its text (`trailing_leading`) does not count.
    fn overlaps_placed(
        &self,
        units: &[Unit],
        first: &State,
        start: &State,
        rects: &[(f64, f64, f64, f64)],
        text_only: bool,
    ) -> bool {
        self.placed[first.placed..start.placed].iter().any(|pl| {
            let (y, h) = match pl {
                Placed::Line {
                    unit, line, y, alt, ..
                } => {
                    let Unit::Para(u0) = &units[*unit] else {
                        return false;
                    };
                    let l = &alt.as_deref().unwrap_or(u0).lines[*line];
                    let tail = if text_only { l.trailing_leading } else { 0.0 };
                    (*y, l.height - tail)
                }
                Placed::Row { row, y, .. } => (*y - row.outer_before, row.flow_height()),
            };
            rects.iter().any(|b| y < b.1 - EPS && y + h > b.0 + EPS)
        })
    }

    /// A floating table wider than its column leaves a band; when that band starts
    /// above where the table's first row `r0` was reached and covers lines placed
    /// earlier on the page, those lines move below it and the table keeps its spot
    /// (Word for Mac probe: a text-anchored table 2 pt above its anchor pushes the
    /// line before it down; in Word 2013+ compatibility the room below a grid
    /// line's text does not count, so one 0.15 pt above on an 18 pt grid leaves
    /// the line). Returns the unit to lay out again from.
    fn lift_float(
        &mut self,
        units: &[Unit],
        snaps: &[Option<State>],
        r0: usize,
        table: usize,
        band: (f64, f64, f64, f64),
    ) -> Option<usize> {
        let start = snaps[r0].as_ref()?;
        if self.pages.len() != start.pages || self.st.col != start.col || band.0 >= start.y - EPS {
            return None;
        }
        let s = Self::page_first_unit(snaps, start, r0);
        let first = snaps[s].clone()?;
        if s == r0 || !self.overlaps_placed(units, &first, start, &[band], self.compat_mode >= 15) {
            return None;
        }
        self.float_pins.insert(table, (start.pages, band.0));
        let mut snap = first;
        snap.bands.push(band);
        self.restore(&snap);
        Some(s)
    }

    /// Keeps unit `i`'s floating objects where they were just placed: Word does not
    /// move an object whose band pushed its own anchor on to the next column (Word
    /// renders of two-column fact sheets: the photo stays in the first column).
    fn pin_objects(&mut self, units: &[Unit], start: &State, i: usize) {
        let Unit::Para(u0) = &units[i] else {
            return;
        };
        let lines: Vec<PinLine> = self.placed[start.placed..]
            .iter()
            .filter_map(|pl| match pl {
                Placed::Line {
                    unit,
                    line,
                    page,
                    column,
                    y,
                    para_y,
                    alt,
                    ..
                } if *unit == i && *page + 1 == self.pages.len() => {
                    Some((*line, *column, *para_y, *y, alt.clone()))
                }
                _ => None,
            })
            .collect();
        let Some(page) = self.pages.last() else {
            return;
        };
        let base = lines
            .first()
            .and_then(|l| l.4.clone())
            .map_or_else(|| u0.clone(), |a| (*a).clone());
        let mut pinned = base;
        let mut any = false;
        for (line, column, para_y, y, _) in lines {
            let col_x = page.cols.get(column).map_or(72.0, |c| c.0);
            let Some(l) = pinned.lines.get_mut(line) else {
                continue;
            };
            for a in &mut l.anchors {
                let (x, top, _, _) = anchor_rect(a, page, col_x, para_y, y);
                a.h_rel = HRel::Page;
                a.x = Ok(x);
                a.v_rel = VRel::Page;
                a.y = Ok(top);
                any = true;
            }
        }
        if any {
            self.pins.insert(i, std::rc::Rc::new(pinned));
        }
    }

    fn restore_balance_start(&mut self, start: &State, saved_pages: &[PageInfo]) {
        {
            self.pages = saved_pages.to_vec();
        }
        self.restore(start);
        {
            if let (Some(page), Some(saved_page)) = (self.pages.last_mut(), &start.balance_page) {
                *page = saved_page.clone();
            }
        }
    }

    /// Balances the columns of a section's last page before a continuous
    /// section break, as Word does: the smallest column height that keeps the
    /// content on the page.
    fn balance(&mut self, units: &[Unit], snaps: &[Option<State>], si: usize, props: &SectPr) {
        let ncols = self.pages.last().map_or(1, |p| p.cols.len());
        if ncols < 2 {
            return;
        }
        let npages = self.pages.len();
        let Some(us) = (0..units.len()).find(|u| {
            snaps[*u]
                .as_ref()
                .is_some_and(|s| s.pages == npages && s.col == 0)
        }) else {
            return;
        };
        let start = snaps[us].clone().unwrap();
        let end_state = self.snapshot();
        let top = start.y;
        let bottom = self.pages.last().map_or(720.0, |p| p.bottom);
        // Content height: what the columns hold below the start.
        let used: f64 = {
            let mut per: HashMap<u64, f64> = HashMap::new();
            for pl in &self.placed[start.placed..] {
                let (x, y, h) = match pl {
                    Placed::Line {
                        unit,
                        line,
                        x,
                        y,
                        alt,
                        ..
                    } => {
                        let Unit::Para(u0) = &units[*unit] else {
                            continue;
                        };
                        let u = alt.as_deref().unwrap_or(u0);
                        let height = if leading_column_transition(u, *line) {
                            0.0
                        } else {
                            u.lines[*line].height
                        };
                        (*x, *y, height)
                    }
                    Placed::Row { row, x, y, .. } => (*x, *y - row.outer_before, row.flow_height()),
                };
                let e = per.entry(x.to_bits()).or_insert(0.0);
                *e = e.max(y + h - top);
            }
            per.values().sum()
        };
        let mut t = (used / ncols as f64).max(1.0);
        let saved = std::mem::take(&mut self.placed);
        let saved_pages = self.pages.clone();
        self.placed = saved.clone();
        while top + t <= bottom {
            self.restore_balance_start(&start, &saved_pages);
            self.st.cap = Some(top + t);
            for u in us..units.len() {
                match &units[u] {
                    Unit::Para(_) => {
                        self.place_para(units, u, false, si, props, &[]);
                    }
                    Unit::Row(r) => {
                        self.place_row(r, false, si, props, &[]);
                    }
                }
            }
            if self.pages.len() == npages && (self.st.flow_bottom <= top + t + EPS) {
                self.st.cap = None;
                self.st.col = 0;
                // The trial cap bounds fitting; unused search slack is not flow.
                self.st.y = self.st.flow_bottom;
                return;
            }
            t += 4.0;
        }
        // No balanced height fits: keep the unbalanced layout.
        self.placed = saved;
        self.pages = saved_pages;
        self.st = end_state;
        self.st.cap = None;
    }

    /// Ends a floating table and restores the surrounding text flow.
    fn end_float(&mut self) {
        let Some((_, flow_y, top, page, x0, x1)) = self.st.float.take() else {
            return;
        };
        if page + 1 == self.pages.len() {
            let col_w = self.pages[page]
                .cols
                .get(self.st.col)
                .map_or(468.0, |c| c.1);
            // Text runs beside a floating table that leaves at least an inch of the
            // column free; the line wrapper measures the gap for each line.
            if col_w - (x1 - x0) >= 72.0 {
                self.st.side.push((top, self.st.y, x0, x1));
                self.st.table_side.push((top, self.st.y, x0, x1));
            } else {
                self.st
                    .bands
                    .push((top, self.st.y, f64::NEG_INFINITY, f64::INFINITY));
            }
            self.st.y = flow_y;
        } else if page + 1 < self.pages.len() {
            // The table continued on later pages: its last part sits at the top of
            // this page, and the text resumes there beside or below it (Word for Mac
            // probes: a six-row table split across pages).
            let current = self.pages.len() - 1;
            let (col_x, col_w) = self.pages[current]
                .cols
                .get(self.st.col)
                .copied()
                .unwrap_or((72.0, 468.0));
            let top = self.pages[current].top;
            if col_w - (x1 - x0) >= 72.0 && x0 >= col_x - 1.0 {
                self.st.side.push((top, self.st.y, x0, x1));
                self.st.table_side.push((top, self.st.y, x0, x1));
                self.st.y = top;
            }
        }
    }

    /// Places a paragraph; returns where its first line went.
    fn place_para(
        &mut self,
        units: &[Unit],
        ui: usize,
        force: bool,
        si: usize,
        props: &SectPr,
        sections: &[Section],
    ) -> (usize, usize) {
        let Unit::Para(p0) = &units[ui] else {
            unreachable!()
        };
        let pinned = self.pins.get(&ui).cloned();
        let p0: &crate::office::docx::layout::ParaUnit = pinned.as_deref().unwrap_or(p0);
        if p0.page_break_before && !self.st.at_top {
            self.new_page(si, props, sections);
            self.st.hard_break = true;
        } else if force && !self.st.at_top {
            self.next_column(si, props, sections);
        }
        let source_width = props.columns().first().map_or(468.0, |c| c.1);
        let mut measure = source_width;
        let mut alt = self.column_measured(p0, source_width);
        if alt.is_some() {
            measure = self.current_column_width().unwrap_or(source_width);
        }
        let measured = alt.as_deref().unwrap_or(p0);
        let wrapped = { self.wrapped(measured) };
        let mut had_exclusions = wrapped.is_some();
        let mut measured_spot = (self.pages.len(), self.st.col);
        if wrapped.is_some() {
            alt = wrapped;
        }
        if alt.is_none() {
            alt = pinned.clone();
        }
        // A paragraph opening with a column break that reaches another column on this
        // page moves there whole with its space before: the break's line takes no
        // height. In the last column the break acts as a page break and stays as is.
        let cols = self.pages.last().map_or(1, |p| p.cols.len());
        let to_column = p0.column_break_before && self.st.col + 1 < cols;
        if to_column {
            let mut q = alt.as_deref().unwrap_or(p0).clone();
            q.lines[0].height = 0.0;
            q.lines[0].trailing_leading = 0.0;
            alt = Some(std::rc::Rc::new(q));
        }
        let mut first_spot = None;
        let mut k = 0;
        while k < alt.as_deref().unwrap_or(p0).lines.len() {
            let current = alt.as_deref().unwrap_or(p0);
            // Equal-width destinations still need fresh wrapping when page floats
            // enter or leave the remaining text; consumed line intervals stay fixed.
            let spot = (self.pages.len(), self.st.col);
            let moved = spot != measured_spot;
            let has_exclusions = if moved {
                self.wrap_input(current, k)
                    .is_some_and(|wrap| !wrap.rects.is_empty())
            } else {
                had_exclusions
            };
            let mut fresh_tail_skip = false;
            if let Some(reflowed) = self.column_resumed(
                current,
                measure,
                k,
                moved && (had_exclusions || has_exclusions),
            ) {
                measure = self.current_column_width().unwrap_or(measure);
                fresh_tail_skip = k > 0;
                alt = Some(reflowed);
            }
            measured_spot = spot;
            had_exclusions = has_exclusions;
            let p = alt.as_deref().unwrap_or(p0);
            let n = p.lines.len();
            let seg_end = (k..n)
                .find(|m| p.lines[*m].break_after.is_some())
                .map_or(n, |m| m + 1);
            let at_top = self.st.at_top;
            let first = usize::from(to_column);
            let mut space = if k == first { p.before } else { 0.0 };

            // Modern compatibility modes also collapse automatic spacing after page breaks.
            // Section starts use flow state, including starts after a parity blank.
            if at_top
                && k == 0
                && p.src.as_ref().is_some_and(|s| s.style.before_auto)
                && ((self.pages.len() == 1 && self.placed.is_empty())
                    || (self.pages.len() > 1
                        && self.st.col == 0
                        && ((!self.st.hard_break && !p.page_break_before)
                            || (self.compat_mode >= 15 && !self.st.section_start))))
            {
                space = 0.0;
            }
            // Word drops space before at a page or column top reached by flow, and after
            // page breaks (pageBreakBefore from mode 15). Document and section starts keep
            // it, as do Word 97-2003 files (mode 11) after page breaks; see README.
            if at_top
                && k == 0
                && (self.pages.len() > 1 || self.st.col > 0)
                && !self.st.section_start
                && (!self.st.hard_break
                    || (self.st.col == 0
                        && (self.compat_mode >= 15
                            || (self.compat_mode >= 14 && !p.page_break_before))))
            {
                space = 0.0;
            }
            // The first paragraph of a section on a new page keeps only the part of its
            // space before that exceeds the previous paragraph's space after.
            if at_top
                && k == 0
                && self.st.section_start
                && self.st.hard_break
                && self.pages.len() > 1
                && self.st.col == 0
                && !matches!(
                    props.start,
                    SectionStart::Continuous | SectionStart::NextColumn
                )
            {
                space = (space - self.st.last_after).max(0.0);
            }
            let pad_top = if k == 0 { p.pad_top } else { 0.0 };
            let bottom = self.bottom();
            let para_top = self.st.y;
            let mut y = self.st.y + space + pad_top;
            let mut fit = 0;
            let mut picture_page_conflict = false;
            let mut bands = self.st.bands.clone();
            let mut pending: Vec<i64> = Vec::new();
            let mut picture_rects = self.st.picture_rects.clone();
            let cut: Option<(usize, i64, f64)> = None;
            let page_bottom = self.pages.last().map_or(bottom, |p| p.bottom);
            for m in k..seg_end {
                self.add_bands(&p.lines[m], para_top, y, &mut bands);
                let skip = if m == k && k > 0 && !fresh_tail_skip {
                    0.0
                } else {
                    // A first line moved below an object keeps its space before.
                    p.lines[m].skip
                        + if m == 0 && p.lines[m].skip > EPS {
                            space
                        } else {
                            0.0
                        }
                };
                let mut ly = self.clear_bands(y + skip, p.lines[m].height, &bands);
                // A first line pushed below an object keeps its space before.
                if m == 0 && ly > y + skip + EPS {
                    ly += space;
                }
                // A line's footnotes must fit on its page too.
                pending.extend(p.lines[m].notes.iter().copied());
                let line_rects = self.note_picture_rects(&p.lines[m], para_top, ly);
                let mut trial_pictures = picture_rects.clone();
                trial_pictures.extend_from_slice(&line_rects);
                let physical_limit = self.pages.last().map_or(page_bottom, |pg| {
                    self.note_limit_with_pictures(
                        pg,
                        &pending,
                        ly + p.lines[m].height,
                        &trial_pictures,
                    )
                });
                let limit = {
                    self.st
                        .cap
                        .map_or(physical_limit, |cap| physical_limit.min(cap))
                };
                let fresh_page_region = at_top
                    && self.st.col == 0
                    && m == k
                    && self
                        .pages
                        .last()
                        .is_some_and(|page| self.st.region_top <= page.top + EPS);
                let line_picture = self.wrapped_picture_bottom(&p.lines[m], para_top, ly);
                picture_page_conflict = self.pages.last().is_some_and(|page| {
                    self.fragment_retry(page, &pending, ly + p.lines[m].height)
                        || self.new_note_start_conflict(
                            page,
                            &pending,
                            ly + p.lines[m].height,
                            fresh_page_region,
                            &trial_pictures,
                        )
                }) || self.picture_note_conflict(&picture_rects, &pending)
                    || self.picture_column_overflows(&p.lines[m], para_top, ly)
                    || (!fresh_page_region
                        && (self.picture_note_conflict(&line_rects, &pending)
                            || line_picture
                                > self.pages.last().map_or(page_bottom, |pg| pg.h) + EPS));
                if picture_page_conflict {
                    break;
                }

                // A line holding only a page or column break stays on its page
                // even past the bottom (A/B: 21 documents better, 6 worse).
                let bare_break = crate::office::docx::text::paint_empty(&p.lines[m].items)
                    && p.lines[m].break_after.is_some();
                let doc_end = self.doc_last
                    && ui + 1 == units.len()
                    && n == 1
                    && crate::office::docx::text::paint_empty(&p.lines[m].items)
                    && !self.st.at_top
                    && (ly + p.lines[m].height * 0.5 <= limit + EPS);
                // An empty paragraph that ends a section before a new page stays on
                // its page past the bottom (A/B: 593 set 13 better, 1 worse; new 300
                let sect_end = self.sect_breaks
                    && ui + 1 == units.len()
                    && n == 1
                    && crate::office::docx::text::paint_empty(&p.lines[m].items)
                    && !self.st.at_top;
                let bare_break = bare_break || doc_end || sect_end;
                let tail = { p.lines[m].trailing_leading };
                if ly + p.lines[m].height - tail > limit + EPS && !bare_break {
                    break;
                }
                picture_rects.extend(line_rects);
                y = ly + p.lines[m].height;
                fit += 1;
            }
            let remaining = seg_end - k;
            let mut take = fit;
            if fit < remaining {
                if p.keep_lines && k == 0 && !at_top {
                    take = 0;
                }
                if p.widow && seg_end == n && n >= 2 && take > 0 {
                    // Widow: the last line may not stand alone on the next page.
                    if take == remaining - 1 {
                        take -= 1;
                    }
                    // Orphan: the first line may not stand alone at the bottom.
                    if k == 0 && take == 1 {
                        take = 0;
                    }
                }
                let low_region = self
                    .pages
                    .last()
                    .is_some_and(|page| self.st.region_top > page.top + EPS);
                // Continuous columns can begin near the physical page bottom.
                // Retry their first line on a new page before forcing progress.
                if take == 0 && at_top && !low_region && !(fit == 0 && picture_page_conflict) {
                    take = fit.max(1);
                }
            }
            let x = self.col_x();
            let mut y = self.st.y;
            if take > 0 {
                y += space + pad_top;
            }
            for m in k..k + take {
                if first_spot.is_none() {
                    first_spot = Some((self.pages.len() - 1, self.st.col));
                }
                let mut bands = std::mem::take(&mut self.st.bands);
                self.add_bands(&p.lines[m], para_top, y, &mut bands);
                let skip = if m == k && k > 0 && !fresh_tail_skip {
                    0.0
                } else {
                    // A first line moved below an object keeps its space before.
                    p.lines[m].skip
                        + if m == 0 && p.lines[m].skip > EPS {
                            space
                        } else {
                            0.0
                        }
                };
                let pushed = self.clear_bands(y + skip, p.lines[m].height, &bands);
                let y0 = y;
                y = if m == 0 && pushed > y + skip + EPS {
                    pushed + space
                } else {
                    pushed
                };
                // A line holding only a break, pushed below an object past the
                // bottom, stays at the bottom of its page.
                let lh = p.lines[m].height;
                if crate::office::docx::text::paint_empty(&p.lines[m].items)
                    && p.lines[m].break_after.is_some()
                    && y + lh > self.bottom() + EPS
                {
                    y = (self.bottom() - lh).max(y0);
                }
                self.st.bands = bands;
                self.add_side(&p.lines[m], para_top, y);
                self.st.picture_bottom = self.st.picture_bottom.max(self.wrapped_picture_bottom(
                    &p.lines[m],
                    para_top,
                    y,
                ));
                self.st
                    .picture_rects
                    .extend(self.note_picture_rects(&p.lines[m], para_top, y));
                if let Some(pg) = self.pages.last_mut()
                    && let Some((cm, id, shown)) = cut
                    && cm == m
                {
                    pg.note_cut = Some((id, shown));
                }
                self.placed.push(Placed::Line {
                    unit: ui,
                    line: m,
                    page: self.pages.len() - 1,
                    column: self.st.col,
                    x,
                    y,
                    pad_top: if m == 0 { p.pad_top } else { 0.0 },
                    pad_bottom: if m + 1 == n { p.pad_bottom } else { 0.0 },
                    para_y: para_top,
                    alt: alt.clone(),
                });
                y += p.lines[m].height;
                self.st.at_top = false;
                self.st.section_start = false;
                self.st.hard_break = false;
            }
            if take > 0 {
                // Commit the same accepted prefix used by the fit trial. Its final
                // bottom leaves one shared note budget for every accepted line.
                let accepted_notes = p.lines[k..k + take]
                    .iter()
                    .flat_map(|line| line.notes.iter().copied())
                    .collect::<Vec<_>>();
                self.commit_note_body(&accepted_notes, y);
            }
            self.st.y = y;
            let flow_y = if k == 0 && take == 1 && leading_column_transition(p, 0) {
                y - p.lines[0].height
            } else {
                y
            };
            self.record_flow_bottom_at(flow_y);
            k += take;
            if k < seg_end {
                self.next_column(si, props, sections);
                if k == 0 {
                    // A whole-paragraph retry uses the destination page's floats.
                    // Its previous first-line clearance belongs to the old page.
                    alt = self.column_measured(p0, source_width);
                    measure = self.current_column_width().unwrap_or(source_width);
                    let wrapped = self.wrapped(alt.as_deref().unwrap_or(p0));
                    had_exclusions = wrapped.is_some();
                    measured_spot = (self.pages.len(), self.st.col);
                    if let Some(wrapped) = wrapped {
                        alt = Some(wrapped);
                    }
                }
                continue;
            }
            if k == n {
                self.st.y += p.pad_bottom + p.after;
                self.st.last_after = p.after;
                self.record_flow_bottom();
            }
            if let Some(b) = p.lines[k - 1].break_after {
                match b {
                    crate::office::docx::model::BreakKind::Column => {
                        self.next_column(si, props, sections)
                    }
                    _ => self.new_page(si, props, sections),
                }
                self.st.hard_break = true;
            }
        }
        first_spot.unwrap_or((self.pages.len().saturating_sub(1), self.st.col))
    }

    /// Adds the bands of a line's floating objects that text cannot run beside:
    /// top-and-bottom wrapping, or square wrapping over half the column or more.
    fn add_bands(
        &self,
        line: &crate::office::docx::text::Line,
        para_top: f64,
        line_y: f64,
        bands: &mut Vec<(f64, f64, f64, f64)>,
    ) {
        let Some(page) = self.pages.last() else {
            return;
        };
        let col = page.cols.get(self.st.col).copied().unwrap_or((72.0, 468.0));
        for a in &line.anchors {
            let blocking = match a.wrap {
                crate::office::docx::model::Wrap::TopAndBottom => true,
                crate::office::docx::model::Wrap::Square => false,
                crate::office::docx::model::Wrap::None => false,
            };
            if !blocking || (a.behind && !behind_wraps(a)) {
                continue;
            }
            let (x, y, w, h) = anchor_rect(a, page, col.0, para_top, line_y);
            if x < col.0 + col.1 && x + w > col.0 {
                // Objects keep their wrap distances from the text (a page-anchored
                // text box's thumbnail: following text 10 pt high to in place).
                bands.push(wrap_box(a, x, y, w, h));
            }
        }
    }

    /// Lowest declared edge of ordinary, wrapping, locally anchored pictures.
    /// The caller reserves footnotes and enforces page fitting while preserving
    /// fresh-page progress for pictures that cannot fit on an empty page.
    fn wrapped_picture_bottom(
        &self,
        line: &crate::office::docx::text::Line,
        para_top: f64,
        line_y: f64,
    ) -> f64 {
        let Some(page) = self.pages.last() else {
            return 0.0;
        };
        let col_x = page.cols.get(self.st.col).map_or(72.0, |c| c.0);
        line.anchors
            .iter()
            .map(|a| {
                if !Self::movable_picture(a, page) {
                    return 0.0;
                }
                let (_, y, _, h) = anchor_rect(a, page, col_x, para_top, line_y);
                y + h
            })
            .fold(0.0, f64::max)
    }

    fn movable_picture(a: &Anchored, page: &PageInfo) -> bool {
        let raster = a.pic.part.is_some() && a.pic.shape.is_none();
        let chart =
            a.pic.part.is_none() && a.pic.shape.as_ref().is_some_and(|shape| shape.direct_chart);
        (raster || chart)
            && a.pic.graphic.is_none()
            && a.text_items.is_empty()
            && a.frame_children.is_empty()
            && !a.text_frame
            && !a.turned
            && (!a.behind || behind_wraps(a))
            && a.wrap != crate::office::docx::model::Wrap::None
            && matches!(a.v_rel, VRel::Paragraph | VRel::Line)
            && matches!(a.y, Ok(offset) if offset >= 0.0)
            && a.pic.h <= page.h
    }

    /// Column-relative companions can fit at the first column but cross the page
    /// edge when their paragraph retries in a later column. Preserve source
    /// positions that already exceed the first column's physical page extent.
    fn picture_column_overflows(
        &self,
        line: &crate::office::docx::text::Line,
        para_top: f64,
        line_y: f64,
    ) -> bool {
        let Some(page) = self.pages.last() else {
            return false;
        };
        if self.st.col == 0 {
            return false;
        }
        let first_x = page.cols.first().map_or(72.0, |c| c.0);
        let col_x = page.cols.get(self.st.col).map_or(first_x, |c| c.0);
        line.anchors.iter().any(|a| {
            if !Self::movable_picture(a, page) || a.h_rel != HRel::Column {
                return false;
            }
            let (x, _, w, _) = anchor_rect(a, page, col_x, para_top, line_y);
            let (first, _, fw, _) = anchor_rect(a, page, first_x, para_top, line_y);
            (x < -EPS || x + w > page.w + EPS) && first >= -EPS && first + fw <= page.w + EPS
        })
    }

    /// Retains square exclusions across the page; each destination column
    /// filters them by horizontal intersection when laying out its text.
    fn add_side(&mut self, line: &crate::office::docx::text::Line, para_top: f64, line_y: f64) {
        let Some(page) = self.pages.last() else {
            return;
        };
        let col = page.cols.get(self.st.col).copied().unwrap_or((72.0, 468.0));
        for a in &line.anchors {
            if a.wrap != crate::office::docx::model::Wrap::Square || (a.behind && !behind_wraps(a))
            {
                continue;
            }
            let (x, y, w, h) = anchor_rect(a, page, col.0, para_top, line_y);
            let boxes = wrap_boxes(a, x, y, w, h);
            if a.contour.is_some() {
                self.st.contour_side.extend(boxes.iter().copied());
            }
            if matches!(
                a.v_rel,
                crate::office::docx::model::VRel::Page | crate::office::docx::model::VRel::Margin
            ) {
                self.st.fixed_side.extend(boxes.iter().copied());
            }
            self.st.side.extend(boxes);
        }
    }

    fn current_column_width(&self) -> Option<f64> {
        self.pages.last()?.cols.get(self.st.col).map(|c| c.1)
    }

    /// Rebuilds a paragraph's lines and horizontal decoration at its starting
    /// column measure before any lines have been placed.
    fn column_measured(
        &self,
        p: &crate::office::docx::layout::ParaUnit,
        prior: f64,
    ) -> Option<std::rc::Rc<crate::office::docx::layout::ParaUnit>> {
        let width = self.current_column_width()?;
        if (width - prior).abs() < EPS || p.drop_cap_anchor.is_some() {
            return None;
        }
        let src = p.src.as_ref()?;
        let mut lines = crate::office::docx::text::layout_paragraph(src, width, self.env);
        restore_reflow_anchors(&p.lines, &p.frame_anchors, &mut lines);
        if p.lines
            .iter()
            .flat_map(|line| &line.anchors)
            .any(crate::office::docx::layout::inline_box_owner)
        {
            crate::office::docx::layout::Flow::new(self.env, self.env.compat_mode)
                .resolve_inline_textboxes(&mut lines);
        }
        let mut next = p.clone();
        next.lines = lines;
        if let Some(decor) = next.decor.as_mut() {
            decor.x1 += width - prior;
        }
        Some(std::rc::Rc::new(next))
    }

    /// Replays consumed line intervals before measuring the remaining source in
    /// its current column. Earlier placements retain their original line data.
    fn column_resumed(
        &self,
        p: &crate::office::docx::layout::ParaUnit,
        prior: f64,
        consumed: usize,
        rewrap: bool,
    ) -> Option<std::rc::Rc<crate::office::docx::layout::ParaUnit>> {
        let width = self.current_column_width()?;
        if (!rewrap && (width - prior).abs() < EPS) || p.drop_cap_anchor.is_some() {
            return None;
        }
        let src = p.src.as_ref()?;
        let wrap = self.wrap_input(p, consumed).unwrap_or_default();
        let mut lines = crate::office::docx::text::layout_paragraph_resuming(
            src,
            width,
            self.env,
            &p.lines[..consumed],
            &wrap,
        );
        restore_reflow_anchors(&p.lines, &p.frame_anchors, &mut lines);
        if p.lines
            .iter()
            .flat_map(|line| &line.anchors)
            .any(crate::office::docx::layout::inline_box_owner)
        {
            crate::office::docx::layout::Flow::new(self.env, self.env.compat_mode)
                .resolve_inline_textboxes(&mut lines);
        }
        let mut next = p.clone();
        next.lines = lines;
        if let Some(decor) = next.decor.as_mut() {
            decor.x1 += width - prior;
        }
        Some(std::rc::Rc::new(next))
    }

    /// Builds current-column exclusions from existing floats and the paragraph's
    /// remaining anchors, with vertical coordinates relative to the next line.
    fn wrap_input(
        &self,
        p: &crate::office::docx::layout::ParaUnit,
        consumed: usize,
    ) -> Option<crate::office::docx::text::WrapIn> {
        let page = self.pages.last()?;
        let (cx, cw) = page.cols.get(self.st.col).copied()?;
        let para_top = self.st.y;
        let first_top = self.st.y
            + if consumed == 0 {
                p.before + p.pad_top
            } else {
                0.0
            };
        let mut boxes: Vec<(f64, f64, f64, f64)> = self.st.side.clone();
        let mut contour: Vec<(f64, f64, f64, f64)> = self.st.contour_side.clone();
        for (index, a) in p
            .lines
            .iter()
            .skip(consumed)
            .flat_map(|l| l.anchors.iter())
            .enumerate()
        {
            if a.wrap == crate::office::docx::model::Wrap::Square && (!a.behind || behind_wraps(a))
            {
                let line_top = if consumed == 0 && p.drop_cap_anchor == Some(index) {
                    first_top
                } else {
                    para_top
                };
                let (x, y, w, h) = anchor_rect(a, page, cx, para_top, line_top);
                let own = wrap_boxes(a, x, y, w, h);
                if a.contour.is_some() {
                    contour.extend(own.iter().copied());
                }
                boxes.extend(own);
            }
        }
        let kept: Vec<&(f64, f64, f64, f64)> = boxes
            .iter()
            .filter(|b| b.1 > first_top + EPS && b.0 < page.bottom && b.2 < cx + cw && b.3 > cx)
            .collect();
        let rects = kept
            .iter()
            .map(|b| (b.0 - first_top, b.1 - first_top, b.2 - cx, b.3 - cx))
            .collect();
        Some(crate::office::docx::text::WrapIn {
            rects,
            contour: kept.iter().map(|b| contour.contains(b)).collect(),
            est_h: p.lines.get(consumed).map_or(12.0, |l| l.height),
            // A paragraph with no text (only its mark, perhaps an anchor) stays
            // beside a wrapped picture or shape in any gap (its anchor paragraph and
            // the empty ones after it, in Word's PDF); floating tables keep 18 pt.
            min_gap: if p
                .lines
                .iter()
                .all(|l| crate::office::docx::text::paint_empty(&l.items))
                && !kept.iter().any(|b| self.st.table_side.contains(b))
            {
                1.0
            } else {
                18.0
            },
        })
    }

    /// Lays out a paragraph around the square-wrapped objects on its page.
    fn wrapped(
        &self,
        p: &crate::office::docx::layout::ParaUnit,
    ) -> Option<std::rc::Rc<crate::office::docx::layout::ParaUnit>> {
        // A hidden empty paragraph keeps its zero-height lines: laying it out again
        // around a wrapped object would give it a line.
        if !p.lines.is_empty() && p.lines.iter().all(|l| l.height == 0.0) {
            return None;
        }
        let src = p.src.as_ref()?;
        let cw = self.current_column_width()?;
        let wrap = self.wrap_input(p, 0)?;
        if wrap.rects.is_empty() {
            return None;
        }
        let mut lines =
            crate::office::docx::text::layout_paragraph_wrapped(src, cw, self.env, &wrap);
        restore_reflow_anchors(&p.lines, &p.frame_anchors, &mut lines);
        if p.lines
            .iter()
            .flat_map(|line| &line.anchors)
            .any(crate::office::docx::layout::inline_box_owner)
        {
            crate::office::docx::layout::Flow::new(self.env, self.env.compat_mode)
                .resolve_inline_textboxes(&mut lines);
        }
        let n = lines.len();
        if n >= 2
            && lines[n - 2].break_after.is_some()
            && crate::office::docx::text::paint_empty(&lines[n - 1].items)
        {
            crate::office::docx::text::pop_empty_terminal_line(&mut lines);
        }
        Some(std::rc::Rc::new(crate::office::docx::layout::ParaUnit {
            lines,
            ..p.clone()
        }))
    }

    /// The first y at or below `y` where a line of height `h` overlaps no band.
    fn clear_bands(&self, mut y: f64, h: f64, bands: &[(f64, f64, f64, f64)]) -> f64 {
        let col = self
            .pages
            .last()
            .and_then(|p| p.cols.get(self.st.col).copied())
            .unwrap_or((72.0, 468.0));
        for _ in 0..8 {
            let hit = bands
                .iter()
                .filter(|b| b.2 < col.0 + col.1 && b.3 > col.0)
                .filter(|b| y < b.1 - EPS && y + h > b.0 + EPS)
                .map(|b| b.1)
                .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
            match hit {
                Some(b) if b > y => y = b,
                _ => break,
            }
        }
        y
    }

    fn place_row(
        &mut self,
        row: &RowUnit,
        force: bool,
        si: usize,
        props: &SectPr,
        sections: &[Section],
    ) -> (usize, usize) {
        if row.page_break_before && (!self.st.at_top || self.st.col > 0) {
            self.new_page(si, props, sections);
            self.st.hard_break = true;
        } else if force && !self.st.at_top {
            self.next_column(si, props, sections);
        }
        let mut row = row.clone();
        // A floating table is placed at its own position, out of the flow.
        if let Some((anchor, y)) = row.float_y.take() {
            self.end_float();
            let page_h = props.page_h;
            let margin_top = props.top.abs();
            let margin_bottom = props.page_h - props.bottom.abs();
            // A text-anchored table whose first row cannot start below its anchor
            // floats at the top of the next page with its anchor (Word for Mac probes:
            // the following text runs beside a narrow one there and below a wide one;
            // a long table's anchor heading leads the next page above it).
            if let ("text", Ok(v)) = (anchor.as_str(), &y) {
                let bottom = self.pages.last().map_or(page_h, |p| p.bottom);
                // Room for the first row's first lines is enough unless the row
                // cannot split (Word splits a long guidance row there).
                let first = if row.cant_split {
                    row.flow_height()
                } else {
                    row.outer_before
                        + row
                            .cells
                            .iter()
                            .map(|c| c.margins[0] + c.slices.first().map_or(0.0, |s| s.h))
                            .fold(0.0, f64::max)
                };
                if !self.st.at_top && self.st.y + v + first > bottom {
                    self.new_page(si, props, sections);
                }
            }
            let pinned = self
                .float_pins
                .get(&row.table)
                .filter(|p| p.0 == self.pages.len())
                .map(|p| p.1);
            let target = match (anchor.as_str(), &y) {
                _ if pinned.is_some() => pinned.unwrap_or_default(),
                ("page", Ok(v)) => *v,
                ("text", Ok(v)) => self.st.y + v,
                (_, Ok(v)) => margin_top + v,
                (_, Err(spec)) => match spec.as_str() {
                    "center" => (margin_top + margin_bottom - row.flow_height()) / 2.0,
                    "bottom" | "outside" => margin_bottom - row.flow_height(),
                    _ if anchor == "page" => 0.0,
                    _ => margin_top,
                },
            };
            if target >= 0.0 && target < page_h - row.flow_height() && !self.pages.is_empty() {
                let cx = self.row_x(&row);
                // moves below the wrapped objects beside its first row.

                let x0 = row.cells.iter().map(|c| c.x).fold(f64::INFINITY, f64::min);
                let x1 = row
                    .cells
                    .iter()
                    .map(|c| c.x + c.w)
                    .fold(f64::NEG_INFINITY, f64::max);
                self.st.float = Some((
                    row.table,
                    self.st.y,
                    target,
                    self.pages.len() - 1,
                    cx + x0,
                    cx + x1,
                ));
                self.st.y = target;
            }
        } else if self.st.float.is_none() {
            let mut bands = self.st.bands.clone();
            {
                bands.extend(self.st.side.iter().copied());
            }
            // A row runs beside a wrapped object that it clears horizontally.
            let cx = self.row_x(&row);
            let x0 = cx + row.cells.iter().map(|c| c.x).fold(f64::INFINITY, f64::min);
            let x1 = cx
                + row
                    .cells
                    .iter()
                    .map(|c| c.x + c.w)
                    .fold(f64::NEG_INFINITY, f64::max);
            if x0.is_finite() && x1.is_finite() {
                bands.retain(|b| b.2 < x1 - EPS && b.3 > x0 + EPS);
            }
            self.st.y = self.clear_bands(self.st.y, row.flow_height().min(1.0), &bands);
        }
        // Text wraps around a floating table's whole box: a later row wider than the
        // first (a short first row of two cells) widens it.
        let cx = self.row_x(&row);
        if let Some((.., x0, x1)) = self.st.float.as_mut().filter(|f| f.0 == row.table) {
            for c in &row.cells {
                *x0 = x0.min(cx + c.x);
                *x1 = x1.max(cx + c.x + c.w);
            }
        }
        if self.st.body_merges.iter().any(|chain| {
            chain.table == row.table && (chain.first..=chain.last).contains(&row.index)
        }) {
            return self.place_body_merge_row(row, si, props, sections);
        }
        let mut first_spot = None;
        let mut guard = 0;
        let mut merges = None;
        let mut merge_started = false;
        loop {
            guard += 1;
            // Nothing but repeated header rows above: the row cannot move further.
            let fresh = self.st.at_top;
            // A table continuing at the top of a page repeats its header rows.
            if self.st.at_top
                && !row.header
                && !row.headers.is_empty()
                && self
                    .st
                    .last_table
                    .is_some_and(|(t, p)| t == row.table && p < self.pages.len() - 1)
            {
                let headers = row.headers.clone();
                for h in headers.iter() {
                    if h.flow_height() < self.bottom() - self.st.y {
                        self.push_row(h.clone());
                    }
                }
            }
            // The row's footnotes need room at the bottom of the page too.
            let row_notes = row.notes();
            if self.pages.last().is_some_and(|page| {
                self.fragment_retry(page, &row_notes, self.st.y + row.flow_height())
            }) {
                self.next_column(si, props, sections);
                continue;
            }
            let physical = self
                .pages
                .last()
                .map_or(720.0, |p| self.note_limit(p, &row_notes));
            if !row_notes.is_empty()
                && self.picture_note_conflict(&self.st.picture_rects, &row_notes)
            {
                self.next_column(si, props, sections);
                continue;
            }
            let limit = self.st.cap.map_or(physical, |cap| physical.min(cap));
            let avail = limit - self.st.y;
            let spot = (self.pages.len() - 1, self.st.col);
            let row_y = self.st.y + row.outer_before;
            if !merge_started {
                merge_started = true;
                {
                    let row_x = self.row_x(&row);
                    merges =
                        merges::Batch::new(&mut row, &self.placed, spot.0, row_x, row_y, avail);
                }
            }
            if let Some(batch) = &mut merges {
                batch.prepare_source(
                    spot.0,
                    self.row_x(&row),
                    row_y,
                    self.placed.len(),
                    true,
                    merges::SourceProjection {
                        row: Some(&row),
                        env: None,
                    },
                );
                row.height = row.height.max(batch.required(row_y));
            }
            if row.flow_height() <= avail + EPS || guard > 200 {
                first_spot.get_or_insert(spot);
                self.push_allocated_row(row, merges.as_mut());
                debug_assert!(merges.as_ref().is_none_or(|batch| batch.complete()));
                break;
            }
            // Word moves a row whole where neither its authored minimum height nor its
            // content fits (Word for Mac probe: a 212 pt "at least" row of 220 pt text
            // with 176 pt left moves; the same row without the minimum splits).
            // A minimum taller than a fresh page cannot move whole: it splits in place.
            let page_room = self.pages.last().map_or(avail, |page| physical - page.top);
            // Only when the content itself does not fit either: a row whose text fits
            // keeps its place (Word kept a 155 pt minimum row 1.7 pt short of room).
            // A row kept with the rows before it would only drag them along: it splits.
            let minimum_moves = row.min_height > avail + EPS
                && row.natural_height > avail + EPS
                && row.min_height <= page_room + EPS
                && !row.after_keep;
            if (row.cant_split || minimum_moves) && !fresh {
                self.next_column(si, props, sections);
                continue;
            }
            let reserve = merges
                .as_mut()
                .map(|batch| batch.preview(row_y, avail - row.outer_before));
            let fresh_capacity = self.pages.last().map_or(avail, |page| physical - page.top);
            let split =
                split_row_in_area(&row, avail, !fresh, reserve, fresh_capacity).or_else(|| {
                    fresh
                        .then(|| {
                            crate::office::docx::layout::split_row_first_line(
                                &row,
                                avail,
                                reserve,
                                fresh_capacity,
                            )
                        })
                        .flatten()
                });
            match split {
                Some((top, rest)) => {
                    first_spot.get_or_insert(spot);
                    let consumed = merges.as_ref().map(|batch| batch.consumed());
                    self.push_allocated_row(top, merges.as_mut());
                    // Count stalled retries separately from consumed source payload or
                    // minimum-height debt, including nested cells and merged allocation.
                    if rest.flow_height() + EPS < row.flow_height()
                        || row_payload_size(&rest) < row_payload_size(&row)
                        || merges
                            .as_ref()
                            .is_some_and(|batch| Some(batch.consumed()) > consumed)
                    {
                        guard = 0;
                    }
                    if rest.flow_height() <= EPS && row_payload_size(&rest) == 0 {
                        break;
                    }
                    row = rest;
                    self.next_column(si, props, sections);
                }
                None if fresh => {
                    first_spot.get_or_insert(spot);
                    self.push_allocated_row(row, merges.as_mut());
                    break;
                }
                None => self.next_column(si, props, sections),
            }
        }
        first_spot.unwrap_or((self.pages.len() - 1, self.st.col))
    }

    fn record_flow_bottom(&mut self) {
        self.record_flow_bottom_at(self.st.y);
    }

    fn record_flow_bottom_at(&mut self, y: f64) {
        if self.st.float.is_none() {
            self.st.flow_bottom = self.st.flow_bottom.max(y);
        }
    }

    fn push_allocated_row(&mut self, row: RowUnit, batch: Option<&mut merges::Batch>) {
        let index = self.placed.len();
        let paint =
            batch.map(|batch| batch.commit(self.st.y + row.outer_before, row.height, index));
        self.push_row(row);
        if let (Some(paint), Some(Placed::Row { merge_paint, .. })) =
            (paint, self.placed.last_mut())
        {
            *merge_paint = paint;
        }
    }

    fn push_row(&mut self, row: RowUnit) {
        let x = self.row_x(&row);
        let h = row.flow_height();
        let y = self.st.y + row.outer_before;
        let table = row.table;
        // Footnotes referenced in the row's cells go to this page.
        let notes = row.notes();
        self.commit_note_body(&notes, self.st.y + h);
        self.placed.push(Placed::Row {
            row: Box::new(row),
            merge_paint: Vec::new(),
            page: self.pages.len() - 1,
            column: self.st.col,
            normal_flow: self.st.float.is_none(),
            x,
            y,
        });
        self.st.y += h;
        self.record_flow_bottom();
        self.st.at_top = false;
        self.st.section_start = false;
        self.st.hard_break = false;
        self.st.last_table = Some((table, self.pages.len() - 1));
    }

    /// Header and footer heights for a page of a section.
    fn hf_extent(
        &mut self,
        props: &SectPr,
        first: bool,
        number: i64,
        _sections: &[Section],
    ) -> (Option<f64>, Option<f64>) {
        let (h, f) = self.hf_parts(props, first, number);
        let width = (props.page_w - props.left - props.right).max(36.0);
        // `None` when the page has no header (or footer) part at all.
        let mut get = |part: Option<String>, footer: bool| -> Option<f64> {
            let part = part?;
            let key = (part.clone(), width.to_bits(), footer);
            if let Some(v) = self.hf_heights.get(&key) {
                return Some(*v);
            }
            let blocks = self.hf.blocks.get(&part)?;
            let flow = Flow::new(self.env, self.compat_mode);
            let flow = if footer {
                flow.with_footer_frames()
            } else {
                flow
            };
            self.env.in_header.set(true);
            let units = flow.units(blocks, width, false);
            self.env.in_header.set(false);
            let (slices, mut v) = stack(&units);
            // A top-and-bottom object anchored in a header paragraph moves that line,
            // and the header's end, below itself in Word 2013+ layout (Word for Mac:
            // a 170 pt box lowers the body to 202 pt; .doc headers stay put).
            if !footer && self.compat_mode >= 15 {
                let mut shift: f64 = 0.0;
                for s in &slices {
                    for (a, _, line_y) in &s.anchors {
                        if let (
                            crate::office::docx::model::Wrap::TopAndBottom,
                            crate::office::docx::model::VRel::Paragraph,
                            Ok(dy),
                        ) = (a.wrap, a.v_rel, &a.y)
                        {
                            let bottom = line_y + s.paragraph_offset + dy + a.pic.h + a.dist[1];
                            shift = shift.max(bottom - s.y);
                        }
                    }
                }
                v += shift;
            }
            let anchors = slices.into_iter().flat_map(|s| s.anchors).collect();
            self.hf_anchors.insert(key.clone(), anchors);
            self.hf_heights.insert(key, v);
            Some(v)
        };
        (get(h, false), get(f, true))
    }

    /// The header and footer parts a page uses.
    fn hf_parts(
        &self,
        props: &SectPr,
        first: bool,
        number: i64,
    ) -> (Option<String>, Option<String>) {
        let pick = |refs: &crate::office::docx::model::HdrFtrRefs| {
            if props.title_pg && first {
                refs.first.clone()
            } else if self.even_and_odd && number % 2 == 0 {
                refs.even.clone()
            } else {
                refs.default.clone()
            }
        };
        (pick(&props.headers), pick(&props.footers))
    }

    /// Sets the document's header and footer STYLEREF fields: (style id, number
    /// wanted, saved result).
    pub fn with_style_refs(mut self, refs: Vec<(Option<String>, bool, String)>) -> Self {
        self.style_refs = refs;
        self
    }

    /// Records a placed paragraph whose style a STYLEREF names.
    fn note_heading(&mut self, si: usize, ui: usize, unit: &Unit, page: usize) {
        let Unit::Para(p) = unit else { return };
        let Some(src) = p.src.as_ref() else { return };
        let Some(id) = src.style.style_id.as_deref() else {
            return;
        };
        if !self
            .style_refs
            .iter()
            .any(|(s, ..)| s.as_deref() == Some(id))
        {
            return;
        }
        let text: String = src
            .inlines
            .iter()
            .filter_map(|i| match i {
                crate::office::docx::model::Inline::Text(t, _) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        let label = src
            .num
            .as_ref()
            .map(|n| n.text.trim_end_matches(['\t', ' ']).to_string())
            .unwrap_or_default();
        self.headings.insert(
            (si, ui),
            (page, id.to_string(), text.trim().to_string(), label),
        );
    }

    /// Each STYLEREF's text on page `page`: the first paragraph of its style on the
    /// page, else the last one before it, else the saved result (Word for headers).
    fn style_ref_texts(&self, page: usize) -> Vec<String> {
        let mut placed: Vec<&(usize, String, String, String)> = self.headings.values().collect();
        placed.sort_by_key(|h| h.0);
        self.style_refs
            .iter()
            .map(|(id, number, saved)| {
                let Some(id) = id else { return saved.clone() };
                let of_style = || placed.iter().filter(|h| &h.1 == id);
                let hit = of_style()
                    .find(|h| h.0 == page)
                    .or_else(|| of_style().rfind(|h| h.0 < page));
                match hit {
                    Some(h) if *number => h.3.clone(),
                    Some(h) => h.2.clone(),
                    None => saved.clone(),
                }
            })
            .collect()
    }

    fn draw_headers(&self, out: &mut [OutPage], sections: &[Section]) {
        let total = out.len() as u32;
        let mut section_pages: HashMap<usize, u32> = HashMap::new();
        for p in out.iter() {
            *section_pages.entry(p.info.section).or_default() += 1;
        }
        for (index, page) in out.iter_mut().enumerate() {
            if !self.style_refs.is_empty() {
                crate::office::docx::text::set_style_ref_texts(self.style_ref_texts(index));
            }
            if page.info.blank {
                continue;
            }
            let Some(sec) = sections.get(page.info.section) else {
                continue;
            };
            let props = &sec.props;
            let (h, f) = self.hf_parts(props, page.info.first_of_section, page.info.number);
            let width = (props.page_w - props.left - props.right).max(36.0);
            let env = TextEnv {
                fonts: self.env.fonts,
                default_tab: self.env.default_tab,
                fields: FieldValues {
                    page: crate::office::docx::numbering::format_number(
                        props.pg_fmt.as_deref().unwrap_or("decimal"),
                        page.info.number,
                    ),
                    page_no: page.info.number,
                    num_pages: total,
                    section_pages: section_pages.get(&page.info.section).copied().unwrap_or(1),
                },
                grid: None,
                char_grid: std::cell::Cell::new(props.character_grid()),
                expand_shift_return: self.env.expand_shift_return,
                mark_in_last_line: self.env.mark_in_last_line,
                html_spacing: self.env.html_spacing,
                grow_autofit: self.env.grow_autofit,
                in_cell: Default::default(),
                in_header: std::cell::Cell::new(true),
                grid_in_table: self.env.grid_in_table,
                no_leading: self.env.no_leading,
                compat_mode: self.env.compat_mode,
            };
            let mut hf_items = Vec::new();
            for (part, is_header) in [(h, true), (f, false)] {
                let Some(blocks) = part.as_ref().and_then(|p| self.hf.blocks.get(p)) else {
                    continue;
                };
                let flow = Flow::new(&env, self.compat_mode);
                let flow = if is_header {
                    flow
                } else {
                    flow.with_footer_frames()
                };
                let units = flow.units(blocks, width, false);
                let (slices, height) = stack(&units);
                let y0 = if is_header {
                    props.header
                } else {
                    props.page_h - props.footer - height
                };
                for s in slices {
                    let key = if is_header {
                        0
                    } else {
                        crate::office::docx::text::FOOTER_KEY
                    };
                    for mut it in s.items {
                        translate_item(&mut it, props.left, y0);
                        crate::office::docx::text::set_reading_key(
                            std::slice::from_mut(&mut it),
                            key,
                        );
                        hf_items.push(it);
                    }
                    for (a, ax, ay) in &s.anchors {
                        if let Some((mut items, _)) = footer_aware_anchor_item(
                            a,
                            &page.info,
                            (props.left + ax, y0 + ay),
                            !is_header,
                        ) {
                            crate::office::docx::text::set_reading_key(&mut items, key);
                            hf_items.extend(items);
                        }
                    }
                }
            }
            if let Some(pb) = props.pg_borders.as_ref() {
                let body = (
                    props.left,
                    props.top,
                    props.page_w - props.right,
                    props.page_h - props.bottom,
                );
                crate::office::docx::decor::page_border(
                    pb,
                    props.page_w,
                    props.page_h,
                    body,
                    page.info.first_of_section,
                    &mut hf_items,
                );
            }
            if !hf_items.is_empty() {
                hf_items.append(&mut page.items);
                page.items = hf_items;
            }
        }
    }
}

/// Numbers the body lines of a section in the left margin: every
/// `count_by`-th line, restarting per page for `newPage`. Table rows and
/// paragraphs with `suppressLineNumbers` are not counted.
fn draw_line_numbers(
    placed: &[Placed],
    units: &[Unit],
    out: &mut [OutPage],
    ln: &crate::office::docx::decor::LineNumbers,
    count: &mut (usize, i64),
    env: &TextEnv,
) {
    for p in placed {
        let Placed::Line {
            unit,
            line,
            page,
            x,
            y,
            alt,
            ..
        } = p
        else {
            continue;
        };
        let Unit::Para(u0) = &units[*unit] else {
            continue;
        };
        let u: &crate::office::docx::layout::ParaUnit = alt.as_deref().unwrap_or(u0);
        if u.src
            .as_ref()
            .is_some_and(|s| s.style.suppress_line_numbers)
        {
            continue;
        }
        if ln.restart == "newPage" && count.0 != *page {
            count.1 = 0;
        }
        count.0 = *page;
        count.1 += 1;
        let n = count.1 + ln.start;
        if n % ln.count_by != 0 {
            continue;
        }
        let Some(pg) = out.get_mut(*page) else {
            continue;
        };
        let col_x = pg
            .info
            .cols
            .iter()
            .rev()
            .find(|c| c.0 <= *x + 0.01)
            .map_or(*x, |c| c.0);
        let l = &u.lines[*line];
        crate::office::docx::decor::line_number(ln, n, col_x, y + l.baseline, env, &mut pg.items);
    }
}

/// Draws placements onto their pages.
#[derive(Default)]
struct DeferredReadingOrder {
    slices: Vec<(usize, usize)>,
}

impl DeferredReadingOrder {
    fn key(&mut self, owner: usize, slice: usize) -> usize {
        let key = usize::MAX - self.slices.len();
        self.slices.push((owner, slice));
        key
    }

    fn source(&self, key: usize) -> (usize, usize) {
        self.slices
            .get(usize::MAX - key)
            .copied()
            .unwrap_or((key, 0))
    }

    /// Dense keys preserve physical groups while inserting deferred source slices.
    fn finish(&self, out: &mut [OutPage]) -> HashMap<usize, (usize, usize)> {
        if self.slices.is_empty() {
            return HashMap::new();
        }
        let mut sources = out
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|item| match item {
                Item::Text(run) => Some(self.source(run.paragraph)),
                Item::Bookmark(marker) => Some(self.source(marker.reading_key)),
                _ => None,
            })
            .collect::<Vec<_>>();
        sources.sort_unstable();
        sources.dedup();
        let ranks: HashMap<_, _> = sources
            .iter()
            .enumerate()
            .map(|(i, source)| (*source, i + 1))
            .collect();
        for item in out.iter_mut().flat_map(|p| &mut p.items) {
            if let Some(key) = crate::office::docx::text::reading_key_mut(item) {
                *key = ranks[&self.source(*key)];
            }
        }
        sources
            .into_iter()
            .enumerate()
            .map(|(i, source)| (i + 1, source))
            .collect()
    }
}

fn draw_placed(
    placed: &[Placed],
    units: &mut [Unit],
    out: &mut [OutPage],
) -> HashMap<usize, (usize, usize)> {
    let mut deferred_order = DeferredReadingOrder::default();
    let debug = std::env::var("DOCX_DEBUG").is_ok();
    // Floating objects per page: (behind, z, items), drawn in z order after the text.
    let mut floats: Vec<Vec<(bool, i64, Vec<Item>)>> = vec![Vec::new(); out.len()];
    // Reading keys continue after the page's earlier sections (stride 1024 per line).
    let mut keys: Vec<usize> = out
        .iter()
        .map(|pg| {
            pg.items
                .iter()
                .filter_map(|it| match it {
                    Item::Text(r) => Some(r.paragraph),
                    Item::Bookmark(marker) => Some(marker.reading_key),
                    _ => None,
                })
                .max()
                .unwrap_or(0)
        })
        .collect();
    let pieces = row_pieces(placed, &keys);
    let mut physical_keys = keys.clone();
    let placement_keys: Vec<_> = placed
        .iter()
        .map(|p| {
            let page = match p {
                Placed::Line { page, .. } | Placed::Row { page, .. } => *page,
            };
            physical_keys[page] += 1024;
            physical_keys[page]
        })
        .collect();
    let mut allocated: HashMap<usize, Vec<Item>> = HashMap::new();
    type AllocatedAnchor = (
        crate::office::docx::model::Anchored,
        f64,
        f64,
        Option<usize>,
    );
    let mut allocated_anchors: HashMap<usize, Vec<AllocatedAnchor>> = HashMap::new();
    for p in placed {
        if let Placed::Row { merge_paint, .. } = p {
            for paint in merge_paint {
                let mut items = paint.items.clone();
                crate::office::docx::layout::stamp_fragment_source(
                    &mut items,
                    paint.fragment_source.as_deref(),
                );
                let body_base = placement_keys[paint.first] + paint.cell.min(1023);
                if paint.body_source_order {
                    for item in &mut items {
                        if let Some(key) = crate::office::docx::text::reading_key_mut(item) {
                            *key = deferred_order.key(body_base, *key);
                        }
                    }
                } else if let Some(base) = paint.reading_key {
                    for item in &mut items {
                        if let Some(key) = crate::office::docx::text::reading_key_mut(item) {
                            *key += base;
                        }
                    }
                } else {
                    crate::office::docx::text::set_reading_key(
                        &mut items,
                        placement_keys[paint.first] + paint.cell.min(1023),
                    );
                }
                allocated.entry(paint.owner).or_default().extend(items);
                allocated_anchors.entry(paint.owner).or_default().extend(
                    paint.anchors.iter().enumerate().map(|(index, (a, x, y))| {
                        (
                            a.clone(),
                            *x,
                            *y,
                            if paint.body_source_order {
                                Some(deferred_order.key(body_base, paint.anchor_keys[index]))
                            } else {
                                paint
                                    .reading_key
                                    .map(|base| base + paint.anchor_keys[index])
                            },
                        )
                    }),
                );
                let _retained_references = &paint.notes;
            }
        }
    }
    for (placement, p) in placed.iter().enumerate() {
        if debug {
            debug_placed(p, units);
        }
        match p {
            Placed::Line {
                unit,
                line,
                page,
                x,
                y,
                pad_top,
                pad_bottom,
                para_y,
                alt,
                ..
            } => {
                let Some(pg) = out.get_mut(*page) else {
                    continue;
                };
                // A line's items move to the page (each line is placed once).
                let moved = match (&mut units[*unit], alt) {
                    (Unit::Para(u0), None) => u0
                        .lines
                        .get_mut(*line)
                        .map(|l| std::mem::take(&mut l.items)),
                    _ => None,
                };
                let Unit::Para(u0) = &units[*unit] else {
                    continue;
                };
                let u: &crate::office::docx::layout::ParaUnit = alt.as_deref().unwrap_or(u0);
                let l = &u.lines[*line];
                let mut items = Vec::new();
                if let Some(d) = &u.decor {
                    para_decor(
                        d,
                        -pad_top,
                        l.height + pad_top + pad_bottom,
                        *line == 0,
                        *line + 1 == u.lines.len(),
                        &mut items,
                    );
                }
                match moved {
                    Some(m) => items.extend(m),
                    None => items.extend(l.items.iter().cloned()),
                }
                for it in items.iter_mut() {
                    translate_item(it, *x, *y);
                }
                keys[*page] += 1024;
                let key = keys[*page];
                if l.anchors.is_empty() {
                    crate::office::docx::text::set_reading_key(&mut items, key);
                } else {
                    // Emit preserves logical text slots around inline anchor atoms.
                    // Deferred keys keep these slots independent of floating paint order.
                    for item in &mut items {
                        if let Some(slot) = crate::office::docx::text::reading_key_mut(item) {
                            *slot = deferred_order.key(key, *slot);
                        }
                    }
                }
                for (k, a) in l.anchors.iter().enumerate() {
                    if let Some((mut drawn, is_behind)) = anchor_item(a, &pg.info, *x, *para_y, *y)
                    {
                        let anchor_key = if *line == 0 && u.drop_cap_anchor == Some(k) {
                            key.saturating_sub(1)
                        } else {
                            deferred_order.key(key, 2 * k + 1)
                        };
                        crate::office::docx::text::set_reading_key(&mut drawn, anchor_key);
                        floats[*page].push((is_behind, a.z, drawn));
                    }
                }
                pg.items.extend(items);
            }
            Placed::Row {
                row, page, x, y, ..
            } => {
                keys[*page] += 1024;
                let key = keys[*page];
                let (row, spread) = spread_merges(row, *page, &pieces, &mut deferred_order);
                let mut owner_content = allocated.remove(&placement).unwrap_or_default();
                for mut spread in spread {
                    if spread.page == *page {
                        owner_content.append(&mut spread.items);
                    } else if let Some(o) = out.get_mut(spread.page) {
                        o.items.append(&mut spread.items);
                    }
                    if let Some(destination) = out.get(spread.page) {
                        for anchor in spread.anchors {
                            if let Some((mut drawn, behind)) = anchor_item(
                                &anchor.source,
                                &destination.info,
                                anchor.x,
                                anchor.y,
                                anchor.y,
                            ) {
                                crate::office::docx::text::set_reading_key(&mut drawn, anchor.key);
                                floats[spread.page].push((behind, anchor.source.z, drawn));
                            }
                        }
                    }
                }
                if let Some(pg) = out.get_mut(*page) {
                    let adjacent_next = adjacent_row(
                        &pieces,
                        (row.table, row.index + 1),
                        (*page, *x, *y + row.height),
                    );
                    let (mut items, borders) =
                        crate::office::docx::layout::row_items_fragment_layers(
                            &row,
                            *x,
                            *y,
                            adjacent_next,
                        );
                    let (text_keys, anchor_keys) =
                        retained_row_order(&row, key, &mut deferred_order);
                    let mut text_keys = text_keys.into_iter();
                    for it in items.iter_mut() {
                        if let Some(key) = crate::office::docx::text::reading_key_mut(it) {
                            *key = text_keys.next().expect("source row text key");
                        }
                    }
                    debug_assert!(text_keys.next().is_none());
                    // Relocated runs already carry their physical reading keys.
                    // Their owner shading is below them; perimeter strokes stay above.
                    items.extend(owner_content);
                    items.extend(borders);
                    pg.items.extend(items);
                    let retained_anchors = crate::office::docx::layout::row_anchors(&row, *x, *y);
                    assert_eq!(
                        retained_anchors.len(),
                        anchor_keys.len(),
                        "source row anchor keys"
                    );
                    let mut anchors: Vec<_> = retained_anchors
                        .into_iter()
                        .zip(anchor_keys)
                        .map(|((a, x, y), source_key)| (a, x, y, Some(source_key)))
                        .collect();
                    anchors.extend(allocated_anchors.remove(&placement).unwrap_or_default());
                    for (k, (a, ax, ay, source_key)) in anchors.iter().enumerate() {
                        if let Some((mut drawn, is_behind)) =
                            anchor_item(a, &pg.info, *ax, *ay, *ay)
                        {
                            crate::office::docx::text::set_reading_key(
                                &mut drawn,
                                source_key.unwrap_or(key + 512 + k.min(511)),
                            );
                            floats[*page].push((is_behind, a.z, drawn));
                        }
                    }
                }
            }
        }
    }
    for (pg, mut fl) in out.iter_mut().zip(floats) {
        // Stable sort: equal heights keep document order.
        fl.sort_by_key(|f| (!f.0, f.1));
        let mut back: Vec<Item> = Vec::new();
        for (behind, _, items) in fl {
            if behind {
                back.extend(items);
            } else {
                pg.items.extend(items);
            }
        }
        if !back.is_empty() {
            back.append(&mut pg.items);
            pg.items = back;
        }
    }
    deferred_order.finish(out)
}

/// Mirrors the source-slice traversal used by row text and anchor painting.
/// Each retained anchored cell gets ordered text/anchor slots within its source
/// row; repeated headers resolve that source row separately for each placement.
fn retained_row_order(
    row: &RowUnit,
    key: usize,
    order: &mut DeferredReadingOrder,
) -> (Vec<usize>, Vec<usize>) {
    let mut text = vec![key; row.bookmarks_before.len()];
    let mut anchors = Vec::new();
    for (cell_index, cell) in row.cells.iter().enumerate() {
        let owner = key.saturating_sub(cell.merged_from * 1024).max(1) + cell_index.min(1023);
        let ordered = cell
            .slices
            .iter()
            .chain(cell.moved_slices())
            .any(|slice| !slice.anchors.is_empty());
        let mut slot = 0;
        let mut text_slots = Vec::new();
        let mut anchor_slots = Vec::new();
        for slice in cell.slices.iter().chain(cell.moved_slices()) {
            note_areas::note_slice_keys(slice, &mut slot, &mut text_slots, &mut anchor_slots);
        }
        text.extend(text_slots.into_iter().map(|slot| {
            if ordered {
                order.key(owner, slot)
            } else {
                owner
            }
        }));
        anchors.extend(
            anchor_slots
                .into_iter()
                .map(|(slot, _)| order.key(owner, slot)),
        );
    }
    text.extend(std::iter::repeat_n(key + 1023, row.bookmarks_after.len()));
    (text, anchors)
}

/// Where each table row went, piece by piece: (table, row) to (page, x, y,
/// height, reading key), with the keys `draw_placed` gives the placements.
type Pieces = HashMap<(usize, usize), Vec<(usize, f64, f64, f64, usize)>>;

// A merge interior is painted only when the next source row is separated
// by a page, column, or geometric gap. Split pieces retain their original edges.
fn adjacent_row(pieces: &Pieces, next: (usize, usize), at: (usize, f64, f64)) -> bool {
    pieces.get(&next).is_some_and(|rows| {
        rows.iter().any(|(page, x, y, _, _)| {
            *page == at.0 && (*x - at.1).abs() < EPS && (*y - at.2).abs() < EPS
        })
    })
}

fn row_pieces(placed: &[Placed], keys: &[usize]) -> Pieces {
    let mut keys = keys.to_vec();
    let mut out: Pieces = HashMap::new();
    for p in placed {
        let page = match p {
            Placed::Line { page, .. } | Placed::Row { page, .. } => *page,
        };
        let Some(k) = keys.get_mut(page) else {
            continue;
        };
        *k += 1024;
        if let Placed::Row { row, x, y, .. } = p {
            out.entry((row.table, row.index))
                .or_default()
                .push((page, *x, *y, row.height, *k));
        }
    }
    out
}

struct SpreadAnchor {
    source: Anchored,
    x: f64,
    y: f64,
    key: usize,
}

struct SpreadPayload {
    page: usize,
    items: Vec<Item>,
    anchors: Vec<SpreadAnchor>,
}

/// A vertical merge whose rows sit on several pages: its content runs down the
/// merged cell's part on each page in turn, from the top. Returns the row
/// without that content and its text/object payload per destination page.
fn spread_merges(
    row: &RowUnit,
    page: usize,
    pieces: &Pieces,
    order: &mut DeferredReadingOrder,
) -> (RowUnit, Vec<SpreadPayload>) {
    let mut row = row.clone();
    let mut out: Vec<SpreadPayload> = Vec::new();
    // Repeated header rows own an independent copy at each placement. Their
    // local merged offsets apply within each copy's contiguous header block.
    if row.repeating_header {
        return (row, out);
    }
    for (ci, c) in row.cells.iter_mut().enumerate() {
        let Some(m) = c.moved.clone() else { continue };
        if c.merged_from == 0 || c.merged_from > row.index {
            continue;
        }
        let spans: Vec<_> = (row.index - c.merged_from..=row.index)
            .filter_map(|r| pieces.get(&(row.table, r)))
            .flatten()
            .copied()
            .collect();
        if spans.iter().all(|s| s.0 == page)
            && (spans
                .windows(2)
                .all(|w| (w[0].1 - w[1].1).abs() < EPS && (w[0].2 + w[0].3 - w[1].2).abs() < EPS))
        {
            continue;
        }
        // The cell's part on each page: (page, x, top, bottom, key).
        let mut parts: Vec<(usize, f64, f64, f64, usize)> = Vec::new();
        for (pg, x, y, h, key) in spans {
            match parts.last_mut() {
                Some(p) if p.0 == pg && ((p.1 - x).abs() < EPS && (p.3 - y).abs() < EPS) => {
                    p.3 = p.3.max(y + h)
                }
                _ => parts.push((pg, x, y, y + h, key)),
            }
        }
        let [m0, _, m2, _] = c.margins;
        let (mut k, mut start) = (0, { 0.0 });
        let ordered_anchors = m.0.iter().any(|slice| !slice.anchors.is_empty());
        for (source_index, s) in m.0.iter().enumerate() {
            let room = parts[k].3 - parts[k].2 - m0 - m2;
            if s.y + s.h - start > room + EPS && k + 1 < parts.len() && s.y > start + EPS {
                k += 1;
                start = s.y;
            }
            let (pg, x, top, _, key) = parts[k];
            let mut items = s.items.clone();
            for it in items.iter_mut() {
                translate_item(it, x + c.x + c.margins[1], top + m0 - start);
            }
            let owner_key = key + ci.min(1023);
            let text_key = if ordered_anchors {
                order.key(owner_key, source_index * 2 + 1)
            } else {
                owner_key
            };
            crate::office::docx::text::set_reading_key(&mut items, text_key);
            let anchor_key = if ordered_anchors {
                order.key(owner_key, source_index * 2 + 2)
            } else {
                owner_key
            };
            let anchors = s.anchors.iter().map(|(source, ax, ay)| SpreadAnchor {
                source: source.clone(),
                x: x + c.x + c.margins[1] + ax,
                y: top + m0 - start + ay,
                key: anchor_key,
            });
            if let Some(destination) = out.iter_mut().find(|o| o.page == pg) {
                destination.items.extend(items);
                destination.anchors.extend(anchors);
            } else {
                out.push(SpreadPayload {
                    page: pg,
                    items,
                    anchors: anchors.collect(),
                });
            }
        }
        c.moved = None;
    }
    (row, out)
}

/// `DOCX_DEBUG`: one stderr line per placement (page, y, height, text).
fn debug_placed(p: &Placed, units: &[Unit]) {
    match p {
        Placed::Line {
            unit,
            line,
            page,
            y,
            alt,
            ..
        } => {
            if let Unit::Para(u0) = &units[*unit] {
                let u: &crate::office::docx::layout::ParaUnit = alt.as_deref().unwrap_or(u0);
                let l = &u.lines[*line];
                let t: String = l.text.chars().take(50).collect();
                eprintln!(
                    "p{page} y={y:.2} h={:.2} u{unit}.{line} b={:.1} a={:.1} {t:?}",
                    l.height, u.before, u.after
                );
            }
        }
        Placed::Row { row, page, y, .. } => {
            let cells: Vec<String> = row
                .cells
                .iter()
                .map(|c| {
                    let sl: Vec<String> = c
                        .slices
                        .iter()
                        .map(|s| format!("{:.1}+{:.1}", s.y, s.h))
                        .collect();
                    format!(
                        "{:.1}/m{:.1},{:.1}({})",
                        c.content_h,
                        c.margins[0],
                        c.margins[2],
                        sl.join(",")
                    )
                })
                .collect();
            eprintln!(
                "p{page} y={y:.2} h={:.2} row t{} [{}]",
                row.height,
                row.table,
                cells.join(" ")
            );
        }
    }
}

// Source anchors retain their laid-out shapes; injected frame blocks are appended
// separately because the source paragraph does not contain those anchors.
pub(crate) fn restore_reflow_anchors(
    original: &[crate::office::docx::text::Line],
    frames: &[Anchored],
    lines: &mut [crate::office::docx::text::Line],
) {
    // Floating objects keep the text boxes laid out with the paragraph.
    let mut laid = original.iter().enumerate().flat_map(|(index, line)| {
        let count = line
            .anchors
            .len()
            .saturating_sub(if index == 0 { frames.len() } else { 0 });
        line.anchors[..count]
            .iter()
            .filter(|anchor| !crate::office::docx::layout::inline_box_owner(anchor))
    });
    for a in lines
        .iter_mut()
        .flat_map(|l| l.anchors.iter_mut())
        .filter(|anchor| !crate::office::docx::layout::inline_box_owner(anchor))
    {
        if let Some(o) = laid.next() {
            a.text_items = o.text_items.clone();
            // A growing shape keeps the height it was refitted to.
            a.pic.h = o.pic.h;
            a.pic.shape = o.pic.shape.clone();
            a.frame_children = o.frame_children.clone();
        }
    }
    // Frame blocks are absent from the paragraph source used for reflow.
    if let Some(first) = lines.first_mut() {
        first.anchors.extend(frames.iter().cloned());
    }
}

/// A floating picture's image item on the page; `None` for drawings that
/// are not rendered. Wrapping is not applied: text flows as if it were absent.
fn anchor_item(
    a: &Anchored,
    page: &PageInfo,
    col_x: f64,
    para_y: f64,
    line_y: f64,
) -> Option<(Vec<Item>, bool)> {
    anchor_item_with_fit(a, page, col_x, para_y, line_y, true)
}

/// `anchor_item` for header and footer objects. Word fits a header object inside
/// the page bottom like a body one (a full-page header background drawn from the
/// top), but leaves a footer object where its anchor puts it, past the page bottom
/// (a footer's 2-inch page-number text box hanging below the page, in Word's PDF).
fn footer_aware_anchor_item(
    a: &Anchored,
    page: &PageInfo,
    (x, y): (f64, f64),
    footer: bool,
) -> Option<(Vec<Item>, bool)> {
    anchor_item_with_fit(a, page, x, y, y, !footer)
}

fn anchor_item_with_fit(
    a: &Anchored,
    page: &PageInfo,
    col_x: f64,
    para_y: f64,
    line_y: f64,
    fit_bottom: bool,
) -> Option<(Vec<Item>, bool)> {
    let (x, y, w, h) = anchor_rect_fitted(a, page, (col_x, para_y, line_y), true, fit_bottom);
    let mut out = Vec::new();
    if let Some(part) = a.pic.part.clone() {
        out.extend(crate::office::docx::decor::picture_shadow(&a.pic, x, y));
        out.push(Item::Image(ImageItem {
            image: ImageRef {
                part,
                effects: a.pic.effects.clone(),
            },
            transform: Transform::from_row(w as f32, 0.0, 0.0, h as f32, x as f32, y as f32),
            crop: a.pic.crop,
            alpha: a.pic.alpha,
            clip: None,
            shape: None,
            alt_text: a.pic.alt.clone(),
        }));
    }
    // Shapes and their text are built in box-local points.
    let local = a
        .pic
        .shape
        .iter()
        .flat_map(|s| s.items.iter())
        .chain(a.text_items.iter());
    for it in local {
        let mut it = it.clone();
        translate_item(&mut it, x, y);
        out.push(it);
    }
    // Page and margin references keep the document page; local origins move with
    // the frame. Drawing order is resolved within the containing frame.
    let mut children: Vec<_> = a.frame_children.iter().collect();
    children.sort_by_key(|child| (!child.anchor.behind, child.anchor.z));
    let mut behind = Vec::new();
    for child in children {
        let mut nested = child.anchor.clone();
        if matches!(nested.h_rel, HRel::Column | HRel::Character)
            && let Err(alignment) = &nested.x
        {
            let box_width = if nested.turned {
                nested.pic.h
            } else {
                nested.pic.w
            };
            let available = child.placement.map_or(a.pic.w, |(_, width)| width) - child.column_x;
            nested.x = Ok((box_width - nested.pic.w) / 2.0
                + match alignment.as_str() {
                    "center" => (available - box_width) / 2.0,
                    "right" | "outside" => available - box_width,
                    _ => 0.0,
                });
        }
        let (mut child_x, mut child_para, mut child_line) =
            (x + child.column_x, y + child.paragraph_y, y + child.line_y);
        let mut paint_transform = None;
        if let Some((transform, _)) = child.placement {
            let (local_x, local_y, _, _) = anchor_rect_with_fit(
                &nested,
                page,
                child.column_x,
                child.paragraph_y,
                child.line_y,
                false,
            );
            let global_x = matches!(nested.h_rel, HRel::Page | HRel::Margin);
            let global_y = matches!(nested.v_rel, VRel::Page | VRel::Margin);
            let mut corners = [
                tiny_skia::Point::from_xy(0.0, 0.0),
                tiny_skia::Point::from_xy(nested.pic.w as f32, 0.0),
                tiny_skia::Point::from_xy(0.0, nested.pic.h as f32),
                tiny_skia::Point::from_xy(nested.pic.w as f32, nested.pic.h as f32),
            ];
            // Raster provenance retains its own turn without changing a group's frame.
            if let Some(raster) = nested
                .pic
                .shape
                .as_ref()
                .filter(|s| s.raster_picture)
                .and_then(|s| {
                    s.items.iter().find_map(|item| match item {
                        Item::Image(image) => Some(image.transform),
                        _ => None,
                    })
                })
            {
                corners = [
                    tiny_skia::Point::from_xy(0.0, 0.0),
                    tiny_skia::Point::from_xy(1.0, 0.0),
                    tiny_skia::Point::from_xy(0.0, 1.0),
                    tiny_skia::Point::from_xy(1.0, 1.0),
                ];
                raster.map_points(&mut corners);
            }
            let linear = Transform::from_row(
                transform.sx,
                transform.ky,
                transform.kx,
                transform.sy,
                0.0,
                0.0,
            );
            linear.map_points(&mut corners);
            let left = corners.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
            let top = corners.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            let right = corners
                .iter()
                .map(|p| p.x)
                .fold(f32::NEG_INFINITY, f32::max);
            let bottom = corners
                .iter()
                .map(|p| p.y)
                .fold(f32::NEG_INFINITY, f32::max);
            // Global alignment uses the painted rectangle's transformed extent.
            // Offsets retain their source base, including authored negative positions.
            let mut bounds_anchor = nested.clone();
            bounds_anchor.pic.w = (right - left) as f64;
            bounds_anchor.pic.h = (bottom - top) as f64;
            bounds_anchor.turned = false;
            let (global_left, global_top, _, _) = anchor_rect_with_fit(
                &bounds_anchor,
                page,
                child.column_x,
                child.paragraph_y,
                child.line_y,
                false,
            );
            let mut point = tiny_skia::Point::from_xy(local_x as f32, local_y as f32);
            if global_x != global_y {
                // Mixed reference axes resolve independently in physical page directions.
                point = tiny_skia::Point::from_xy(
                    child.column_x as f32,
                    if nested.v_rel == VRel::Line {
                        child.line_y
                    } else {
                        child.paragraph_y
                    } as f32,
                );
                transform.map_point(&mut point);
                if !global_x {
                    point.x += (local_x - child.column_x) as f32;
                }
                if !global_y {
                    let origin = if nested.v_rel == VRel::Line {
                        child.line_y
                    } else {
                        child.paragraph_y
                    };
                    point.y += (local_y - origin) as f32;
                }
            } else {
                transform.map_point(&mut point);
            }
            let px = if global_x {
                global_left - left as f64
            } else {
                x + point.x as f64
            };
            let py = if global_y {
                global_top - top as f64
            } else {
                y + point.y as f64
            };
            // Global reference axes retain the actual page; local flow follows its textbox.
            nested.h_rel = HRel::Page;
            nested.v_rel = VRel::Page;
            nested.x = Ok(px);
            nested.y = Ok(py);
            child_x = 0.0;
            child_para = 0.0;
            child_line = 0.0;
            paint_transform = Some(
                Transform::from_translate(-px as f32, -py as f32)
                    .post_concat(linear)
                    .post_translate(px as f32, py as f32),
            );
        }
        if let Some((mut items, is_behind)) =
            anchor_item(&nested, page, child_x, child_para, child_line)
        {
            if let Some(transform) = paint_transform {
                for item in &mut items {
                    crate::office::docx::layout::transform_item(item, transform);
                }
            }
            if is_behind {
                behind.append(&mut items);
            } else {
                out.append(&mut items);
            }
        }
    }
    behind.append(&mut out);
    let out = behind;
    (!out.is_empty()).then_some((out, a.behind))
}

/// The area a wrapped object keeps clear of text: (top, bottom, left, right),
/// its box grown by its wrap distances; a one-sided wrap blocks the other side.
fn wrap_box(a: &Anchored, x: f64, y: f64, w: f64, h: f64) -> (f64, f64, f64, f64) {
    let [t, b, l, r] = a.dist;
    let (mut x0, mut x1) = (x - l, x + w + r);
    match a.wrap_side.as_str() {
        "left" => x1 = f64::INFINITY,
        "right" => x0 = f64::NEG_INFINITY,
        _ => {}
    }
    (y - t, y + h + b, x0, x1)
}

/// Rotated shapes retain rectangular wrapping until polygon transforms are verified.
pub(crate) fn wrap_boxes(
    a: &Anchored,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Vec<(f64, f64, f64, f64)> {
    if !a.turned
        && let Some(contour) = &a.contour
        && let Some(mut boxes) = contour.bands(x, y, w, h, a.dist)
    {
        for b in &mut boxes {
            match a.wrap_side.as_str() {
                "left" => b.3 = f64::INFINITY,
                "right" => b.2 = f64::NEG_INFINITY,
                _ => {}
            }
        }
        return boxes;
    }
    vec![wrap_box(a, x, y, w, h)]
}

/// A floating object's box on the page: (x, y, w, h). Paragraph-relative
/// offsets count from the paragraph's top, line-relative ones from the line's.
fn anchor_rect(
    a: &Anchored,
    page: &PageInfo,
    col_x: f64,
    para_y: f64,
    line_y: f64,
) -> (f64, f64, f64, f64) {
    anchor_rect_with_fit(a, page, col_x, para_y, line_y, true)
}

fn anchor_rect_with_fit(
    a: &Anchored,
    page: &PageInfo,
    col_x: f64,
    para_y: f64,
    line_y: f64,
    fit_page: bool,
) -> (f64, f64, f64, f64) {
    anchor_rect_fitted(a, page, (col_x, para_y, line_y), fit_page, fit_page)
}

/// `anchor_rect_with_fit` with the page-bottom fit (`fit_bottom`) set apart from
/// the page-width fit of text frames (`fit_page`): footer objects keep the one
/// and skip the other.
fn anchor_rect_fitted(
    a: &Anchored,
    page: &PageInfo,
    (col_x, para_y, line_y): (f64, f64, f64),
    fit_page: bool,
    fit_bottom: bool,
) -> (f64, f64, f64, f64) {
    let (w, h) = (a.pic.w, a.pic.h);
    // Alignment places the turned box (`bw`×`bh`), which shares the unturned box's centre.
    let (bw, bh) = if a.turned { (h, w) } else { (w, h) };
    let margin_l = page.cols.first().map_or(72.0, |c| c.0);
    let margin_r = page.cols.last().map_or(page.w - 72.0, |c| c.0 + c.1);
    let (ref_x, ref_w) = match a.h_rel {
        HRel::Page => (0.0, page.w),
        HRel::Margin => (margin_l, margin_r - margin_l),
        HRel::Column | HRel::Character => (col_x, margin_r - col_x),
    };
    let x = match &a.x {
        Ok(v) => ref_x + v,
        Err(al) => {
            (bw - w) / 2.0
                + match al.as_str() {
                    "center" => ref_x + (ref_w - bw) / 2.0,
                    "right" | "outside" => ref_x + ref_w - bw,
                    _ => ref_x,
                }
        }
    };
    // Numeric page-relative text frames fit their box to the physical page.
    // Their paragraph indents and child coordinates remain local to that box.
    let x = if fit_page
        && a.text_frame
        && !a.turned
        && matches!(a.h_rel, HRel::Page)
        && a.x.is_ok()
        && w > 0.0
        && w <= page.w
    {
        x.clamp(0.0, page.w - w)
    } else {
        x
    };
    let (ref_y, ref_h) = match a.v_rel {
        VRel::Page => (0.0, page.h),
        VRel::Margin => (page.top, page.bottom - page.top),
        VRel::Paragraph => (para_y, 0.0),
        VRel::Line => (line_y, 0.0),
    };
    let y = match &a.y {
        Ok(v) => ref_y + v,
        Err(al) => {
            (bh - h) / 2.0
                + match al.as_str() {
                    "center" => ref_y + (ref_h - bh) / 2.0,
                    "bottom" | "outside" => ref_y + ref_h - bh,
                    _ => ref_y,
                }
        }
    };
    // Non-wrapping local objects fit their declared box inside the page bottom.
    let y = if fit_bottom
        && matches!(a.v_rel, VRel::Paragraph | VRel::Line)
        && a.wrap == crate::office::docx::model::Wrap::None
        && y + h > page.h
    {
        (page.h - h).max(0.0)
    } else if fit_bottom
        && matches!(a.v_rel, VRel::Paragraph | VRel::Line)
        && matches!(a.y, Ok(offset) if offset >= 0.0)
        && !a.behind
        && a.wrap != crate::office::docx::model::Wrap::None
        && !a.text_items.is_empty()
    {
        // Text may overflow its declared box. Resolve the same visible bottom
        // for wrapping and painting so a locally anchored text box stays visible.
        let bottom = anchored_text_bottom(a).max(h);
        if bottom <= page.h && y + bottom > page.h {
            page.h - bottom
        } else {
            y
        }
    } else {
        y
    };
    (x, y, w, h)
}

// Unclipped glyph runs may extend below a shape's declared height.
// Their four transformed corners cover rotated and vertically arranged text.
fn anchored_text_bottom(a: &Anchored) -> f64 {
    let mut bottom = 0.0_f64;
    let mut clip_depth = 0usize;
    for item in &a.text_items {
        match item {
            Item::PushClip(_) => clip_depth += 1,
            Item::PopClip => clip_depth = clip_depth.saturating_sub(1),
            Item::Text(run) if clip_depth == 0 => {
                let mut corners = [
                    tiny_skia::Point::from_xy(0.0, -run.ascent as f32),
                    tiny_skia::Point::from_xy(run.width as f32, -run.ascent as f32),
                    tiny_skia::Point::from_xy(0.0, run.descent as f32),
                    tiny_skia::Point::from_xy(run.width as f32, run.descent as f32),
                ];
                run.transform.map_points(&mut corners);
                for corner in corners {
                    if corner.y.is_finite() {
                        bottom = bottom.max(corner.y as f64);
                    }
                }
            }
            _ => {}
        }
    }
    bottom
}

/// Square-wrapped objects painted behind text still reserve side space.
fn behind_wraps(a: &Anchored) -> bool {
    a.wrap == crate::office::docx::model::Wrap::Square
}

/// Paragraph-relative offsets count from above the paragraph's space before
mod body_merges;
pub(crate) mod merges;
pub(crate) mod note_areas;
pub(crate) mod note_fragments;
mod note_pages;
mod note_placement;

#[cfg(test)]
mod tests;
