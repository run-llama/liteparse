//! WordprocessingML → [`crate::office::docx::model`]: the body's sections, paragraphs, runs,
//! fields, pictures and tables, with the style cascade and list numbering
//! applied in document order.

use std::cell::{Cell as StateCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::office::pptx::package::Package;
use crate::office::pptx::xml::XNode;

use crate::office::docx::model::*;
use crate::office::docx::numbering::Numbering;
use crate::office::docx::props::{Borders, PPr, RPr, tw_attr};
use crate::office::docx::styles::{ParaStyle, RunStyle, StyleSheet};

mod bookmarks;
mod chunks;
mod grid;
mod source;

pub const EMU_PER_PT: f64 = 12_700.0;

fn parse_preferred_width(node: Option<&Rc<XNode>>) -> TableWidth {
    let Some(w) = node else {
        return TableWidth::Auto;
    };
    match (w.attr("type"), w.attr("w")) {
        (_, Some(v)) if v.ends_with('%') || w.attr("type") == Some("pct") => {
            let value = if let Some(percent) = v.strip_suffix('%') {
                percent.parse::<f64>().ok().map(|p| p / 100.0)
            } else {
                v.parse::<f64>().ok().map(|p| p / 5000.0)
            };
            value
                .filter(|p| p.is_finite() && *p > 0.0)
                .map(TableWidth::Percent)
                .unwrap_or_default()
        }
        (Some("dxa") | None, _) => tw_attr(w, "w")
            .filter(|p| p.is_finite() && *p > 0.0)
            .map(TableWidth::Dxa)
            .unwrap_or_default(),
        _ => TableWidth::Auto,
    }
}

/// Recover signed character-grid values emitted as unsigned 32-bit decimals.
/// The grid advance is measured in 1/4096 point units.
fn character_grid_space(raw: &str) -> Option<f64> {
    {
        if let Ok(value) = raw.parse::<u32>()
            && value > i32::MAX as u32
        {
            return Some((value as i32) as f64 / 4096.0);
        }
    }
    raw.parse::<f64>().ok().map(|value| value / 4096.0)
}

/// Accepts tracked deletions of paragraph marks (`w:pPr/w:rPr/w:del`): Word joins such a
/// paragraph's runs onto the next sibling paragraph, which keeps its own properties. A mark
/// that ends a section, or has no paragraph after it, stays.
fn join_deleted_marks(nodes: &[Rc<XNode>]) -> std::borrow::Cow<'_, [Rc<XNode>]> {
    let deleted = |n: &XNode| {
        n.name == "w:p"
            && n.path(&["w:pPr", "w:sectPr"]).is_none()
            && n.path(&["w:pPr", "w:rPr"]).is_some_and(|r| {
                r.children
                    .iter()
                    .any(|c| matches!(c.name.as_str(), "w:del" | "w:moveFrom"))
            })
    };
    let in_code = marks_in_field_code(nodes);
    let hidden = |i: usize| deleted(&nodes[i]) || in_code[i];
    if !(0..nodes.len()).any(hidden) {
        return std::borrow::Cow::Borrowed(nodes);
    }
    let mut out: Vec<Rc<XNode>> = Vec::with_capacity(nodes.len());
    let mut carried: Vec<Rc<XNode>> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        if hidden(i) && nodes.get(i + 1).is_some_and(|next| next.name == "w:p") {
            carried.extend(n.children.iter().filter(|c| c.name != "w:pPr").cloned());
            continue;
        }
        if carried.is_empty() || n.name != "w:p" {
            out.push(n.clone());
            continue;
        }
        let (props, runs): (Vec<_>, Vec<_>) =
            n.children.iter().cloned().partition(|c| c.name == "w:pPr");
        let children = props
            .into_iter()
            .chain(carried.drain(..))
            .chain(runs)
            .collect();
        out.push(Rc::new(XNode {
            children,
            ..(**n).clone()
        }));
    }
    std::borrow::Cow::Owned(out)
}

/// For each node, whether it is a paragraph whose mark falls inside a field's
/// code (after `begin`, before `separate`). Word hides the code, mark included,
/// so the paragraph runs on into the next one (a HYPERLINK whose tooltip text
/// spans the document's first paragraph mark).
fn marks_in_field_code(nodes: &[Rc<XNode>]) -> Vec<bool> {
    fn walk(n: &XNode, open: &mut Vec<bool>) {
        if matches!(n.name.as_str(), "w:del" | "w:moveFrom" | "w:pPr") {
            return;
        }
        if n.name == "w:fldChar" {
            match n.attr("fldCharType").unwrap_or("") {
                "begin" => open.push(false),
                "separate" => {
                    if let Some(s) = open.last_mut() {
                        *s = true;
                    }
                }
                "end" => {
                    open.pop();
                }
                _ => {}
            }
        }
        for c in &n.children {
            walk(c, open);
        }
    }
    let mut open: Vec<bool> = Vec::new();
    nodes
        .iter()
        .map(|n| {
            if n.name != "w:p" {
                open.clear();
                return false;
            }
            walk(n, &mut open);
            n.path(&["w:pPr", "w:sectPr"]).is_none() && open.iter().any(|separated| !separated)
        })
        .collect()
}

/// Adapt accepted Word revisions and standalone math runs to the shared equation reader.
fn word_math_text(node: &Rc<XNode>) -> String {
    fn accepted(node: &Rc<XNode>) -> Rc<XNode> {
        let children = node
            .children
            .iter()
            .filter(|c| !matches!(c.name.as_str(), "w:del" | "w:moveFrom"))
            .map(accepted)
            .collect();
        Rc::new(XNode {
            children,
            ..(**node).clone()
        })
    }
    fn paragraph_children(node: &Rc<XNode>, out: &mut Vec<Rc<XNode>>) {
        for child in &node.children {
            if matches!(child.name.as_str(), "m:r" | "m:oMath") {
                out.push(child.clone());
            } else if !child.name.ends_with("Pr") {
                paragraph_children(child, out);
            }
        }
    }
    let node = accepted(node);
    if node.name != "m:oMathPara" {
        return crate::office::pptx::math::linearize(&node);
    }
    let mut children = Vec::new();
    paragraph_children(&node, &mut children);
    let mut equations = Vec::new();
    let mut runs = Vec::new();
    let flush = |runs: &mut Vec<Rc<XNode>>, equations: &mut Vec<String>| {
        if !runs.is_empty() {
            equations.push(crate::office::pptx::math::linearize(&XNode {
                name: "m:oMath".into(),
                attrs: Vec::new(),
                children: std::mem::take(runs),
                text: String::new(),
            }));
        }
    };
    for child in children {
        match child.name.as_str() {
            "m:r" => runs.push(child),
            "m:oMath" => {
                flush(&mut runs, &mut equations);
                equations.push(crate::office::pptx::math::linearize(&child));
            }
            _ => {}
        }
    }
    flush(&mut runs, &mut equations);
    equations
        .into_iter()
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Table text formatting handed to the paragraphs of its cells.
#[derive(Debug, Clone, Default)]
pub struct TableText {
    pub ppr: PPr,
    pub rpr: RPr,
}

#[derive(Debug)]
struct OpenField {
    source_id: String,
    instr: String,
    separated: bool,
    /// A nested instruction field prevents using its unevaluated static prefix as a target.
    dynamic_instruction: bool,
    /// The field's cached result is replaced (page numbers).
    replaced: bool,
    /// A legacy check box form field (`w:ffData/w:checkBox`): checked or not.
    checkbox: Option<bool>,
    /// A legacy drop-down form field (`w:ffData/w:ddList`): its selected entry.
    dropdown: Option<String>,
    /// The STYLEREF this field's result stands for (an index into `style_refs`).
    style_ref: Option<usize>,
}

pub struct Parser<'a> {
    source: RefCell<source::SourceState>,
    bookmarks: Rc<RefCell<bookmarks::Registry>>,
    imports: Rc<RefCell<chunks::Imports>>,
    grid_budget: Rc<RefCell<grid::Budget>>,
    pub pkg: &'a Package,
    pub sheet: &'a StyleSheet,
    pub settings: &'a Settings,
    pub numbering: RefCell<Numbering>,
    fields: RefCell<Vec<OpenField>>,
    /// Picture parts shown as an equation object's preview (EMBED Equation or
    /// MathType): Word's PDF carries no text for them.
    equation_parts: RefCell<std::collections::HashSet<String>>,
    /// STYLEREF fields of headers and footers.
    style_refs: RefCell<Vec<StyleRef>>,
    /// Document properties shown by header and footer TITLE/DOCPROPERTY fields.
    doc_props: crate::office::docx::doc_props::DocProps,
    footnotes: RefCell<u32>,
    /// Footnote id → its number as shown at the reference.
    pub note_numbers: RefCell<HashMap<i64, String>>,
    /// Number shown by `w:footnoteRef` inside the note being parsed.
    pub current_note: RefCell<Option<String>>,
    /// Endnote ids in reference order, with the number shown and source section.
    pub endnote_order: RefCell<Vec<(i64, String, usize)>>,
    body_section: StateCell<usize>,
    endnote_counter: StateCell<Option<(usize, i64)>>,
    styles_cache: RefCell<HashMap<String, Rc<RunStyle>>>,
    /// Paragraph styles by content (their `Debug` form): equal styles share
    /// one allocation. Run styles stay one per run (text code compares them by pointer).
    para_pool: RefCell<HashMap<String, Rc<ParaStyle>>>,
    /// Theme colors and fills for DrawingML shapes, with its part name.
    pub theme: Option<(crate::office::pptx::theme::Theme, String)>,
    /// Page geometry of the section being parsed (relative object sizes).
    cur_sect: RefCell<SectPr>,
    /// Sections still to come, last first.
    sect_list: RefCell<Vec<SectPr>>,
}

impl<'a> Parser<'a> {
    pub fn new(
        pkg: &'a Package,
        sheet: &'a StyleSheet,
        settings: &'a Settings,
        numbering: Numbering,
    ) -> Parser<'a> {
        Parser {
            source: RefCell::new(source::SourceState::default()),
            bookmarks: Rc::new(RefCell::new(bookmarks::Registry::default())),
            imports: Rc::new(RefCell::new(chunks::Imports::default())),
            grid_budget: Rc::new(RefCell::new(grid::Budget::document())),
            pkg,
            sheet,
            settings,
            numbering: RefCell::new(numbering),
            fields: RefCell::new(Vec::new()),
            equation_parts: RefCell::new(Default::default()),
            style_refs: RefCell::new(Vec::new()),
            doc_props: crate::office::docx::doc_props::DocProps::read(pkg),
            footnotes: RefCell::new(0),
            note_numbers: RefCell::new(HashMap::new()),
            current_note: RefCell::new(None),
            endnote_order: RefCell::new(Vec::new()),
            body_section: StateCell::new(0),
            endnote_counter: StateCell::new(None),
            styles_cache: RefCell::new(HashMap::new()),
            para_pool: RefCell::new(HashMap::new()),
            theme: None,
            cur_sect: RefCell::new(SectPr::default()),
            sect_list: RefCell::new(Vec::new()),
        }
    }

    pub fn share_grid_budget(&mut self, parent: &Self) {
        self.grid_budget = parent.grid_budget.clone();
    }

    pub fn check_grid_budget(&self) -> crate::office::pptx::error::DocResult<()> {
        self.grid_budget.borrow().check()
    }

    /// The body as sections. A paragraph carrying `w:sectPr` ends a section;
    /// the body's own `w:sectPr` describes the last one.
    pub fn body(&self, body: &XNode, part: &str) -> Vec<Section> {
        self.index_source_tables(&body.children, part);
        let mut sections = Vec::new();
        let mut cur = Vec::new();
        let mut last = None;
        let mut boundaries = Vec::new();
        // Section geometry in document order, for sizes relative to the page.
        let mut sects = Vec::new();
        collect_sect_prs(&body.children, &mut sects);
        *self.sect_list.borrow_mut() = sects.iter().map(|n| self.sect_pr(n, part)).rev().collect();
        self.next_sect();
        self.body_nodes(
            &body.children,
            part,
            &mut sections,
            &mut cur,
            &mut last,
            &mut boundaries,
        );
        bookmarks::place_boundaries(&mut cur, &mut boundaries);
        let props = last.map(|s| self.sect_pr(&s, part)).unwrap_or_default();
        sections.push(Section { props, blocks: cur });
        inherit_hdr_ftr(&mut sections);
        sections
    }

    fn body_nodes(
        &self,
        nodes: &[Rc<XNode>],
        part: &str,
        sections: &mut Vec<Section>,
        cur: &mut Vec<Block>,
        last: &mut Option<Rc<XNode>>,
        boundaries: &mut Vec<(
            usize,
            std::sync::Arc<crate::office::pptx::scene::SourceBookmark>,
        )>,
    ) {
        for n in join_deleted_marks(nodes).iter() {
            self.body_section.set(sections.len());
            match n.name.as_str() {
                "w:bookmarkStart" => {
                    if let Some(marker) = self.source_bookmark(n, part, true) {
                        boundaries.push((cur.len(), marker));
                    }
                }
                "w:p" => {
                    cur.push(Block::Para(self.paragraph(n, part, None)));
                    if let Some(sp) = n.path(&["w:pPr", "w:sectPr"]) {
                        bookmarks::place_boundaries(cur, boundaries);
                        sections.push(Section {
                            props: self.sect_pr(sp, part),
                            blocks: std::mem::take(cur),
                        });
                        self.next_sect();
                    }
                }
                "m:oMathPara" | "m:oMath" => {
                    cur.push(Block::Para(self.math_paragraph(n, part, None)));
                }
                "w:tbl" => {
                    if let Some(table) = self.table(n, part) {
                        cur.push(Block::Table(Box::new(table)));
                    }
                }
                "w:sdt" => {
                    if let Some(c) = n.child("w:sdtContent") {
                        self.body_nodes(&c.children, part, sections, cur, last, boundaries);
                    }
                }
                "w:customXml" | "w:ins" | "w:moveTo" => {
                    self.body_nodes(&n.children, part, sections, cur, last, boundaries)
                }
                "w:altChunk" => cur.extend(self.import_chunk(n, part, None)),
                "w:sectPr" => *last = Some(n.clone()),
                _ => {}
            }
        }
    }

    /// Sets the page geometry that headers and footers parsed next are sized against.
    pub fn set_section(&self, sp: &SectPr) {
        *self.cur_sect.borrow_mut() = sp.clone();
    }

    fn next_sect(&self) {
        if let Some(sp) = self.sect_list.borrow_mut().pop() {
            *self.cur_sect.borrow_mut() = sp;
        }
    }

    /// Blocks of a header, footer, cell or text box.
    pub fn blocks(&self, nodes: &[Rc<XNode>], part: &str, table: Option<&TableText>) -> Vec<Block> {
        let mut out = Vec::new();
        let mut boundaries = Vec::new();
        self.blocks_into(nodes, part, table, &mut out, &mut boundaries);
        bookmarks::place_boundaries(&mut out, &mut boundaries);
        // A first push reserves four slots; most cells hold one paragraph.
        out.shrink_to_fit();
        out
    }

    fn blocks_into(
        &self,
        nodes: &[Rc<XNode>],
        part: &str,
        table: Option<&TableText>,
        out: &mut Vec<Block>,
        boundaries: &mut Vec<(
            usize,
            std::sync::Arc<crate::office::pptx::scene::SourceBookmark>,
        )>,
    ) {
        for n in join_deleted_marks(nodes).iter() {
            match n.name.as_str() {
                "w:bookmarkStart" => {
                    if let Some(marker) = self.source_bookmark(n, part, true) {
                        boundaries.push((out.len(), marker));
                    }
                }
                "w:p" => out.push(Block::Para(self.paragraph(n, part, table))),
                "w:altChunk" => out.extend(self.import_chunk(n, part, table)),
                "m:oMathPara" | "m:oMath" => {
                    out.push(Block::Para(self.math_paragraph(n, part, table)));
                }
                "w:tbl" => {
                    if let Some(table) = self.table(n, part) {
                        out.push(Block::Table(Box::new(table)));
                    }
                }
                "w:sdt" => {
                    if let Some(c) = n.child("w:sdtContent") {
                        self.blocks_into(&c.children, part, table, out, boundaries);
                    }
                }
                "w:customXml" | "w:ins" | "w:moveTo" => {
                    self.blocks_into(&n.children, part, table, out, boundaries)
                }
                _ => {}
            }
        }
    }

    /// Projects a standalone math block through the existing linear paragraph flow.
    fn math_paragraph(&self, math: &Rc<XNode>, part: &str, table: Option<&TableText>) -> Paragraph {
        let wrapper = XNode {
            name: "w:p".into(),
            attrs: Vec::new(),
            children: vec![math.clone()],
            text: String::new(),
        };
        let mut paragraph = self.paragraph(&wrapper, part, table);
        if math.name == "m:oMathPara" {
            let justification = math
                .path(&["m:oMathParaPr", "m:jc"])
                .map(|jc| jc.attr("val").unwrap_or("centerGroup"))
                .or(self.settings.math_justification.as_deref())
                .unwrap_or("centerGroup");
            // Linearized centerGroup equations share one centered text paragraph.
            Rc::make_mut(&mut paragraph.style).jc = match justification {
                "left" => crate::office::docx::styles::Jc::Left,
                "right" => crate::office::docx::styles::Jc::Right,
                _ => crate::office::docx::styles::Jc::Center,
            };
        }
        paragraph
    }

    /// Whether a left indent is set by the style that carries the paragraph's
    /// list (`numPr`) or by a style below it in `id`'s chain.
    fn own_list_indent(&self, id: &str) -> bool {
        let chain = self.sheet.chain(id);
        let Some(from) = chain.iter().position(|s| s.ppr.num_id.is_some()) else {
            return false;
        };
        chain[from..].iter().any(|s| s.ppr.ind_left.is_some())
    }

    fn style_num(&self) -> impl Fn(&str) -> Option<i64> + '_ {
        move |id: &str| self.sheet.style_ppr(id).num_id
    }

    pub fn paragraph(&self, p: &XNode, part: &str, table: Option<&TableText>) -> Paragraph {
        let previous_paragraph = {
            let mut source = self.source.borrow_mut();
            let previous = source.paragraph;
            source.paragraph = source.next_paragraph;
            source.next_paragraph += 1;
            previous
        };
        // A paragraph with several w:pPr (some generators split the style and
        // the spacing) takes all of them, later ones winning, as Word does.
        let mut parts = p.children_named("w:pPr");
        let mut direct = parts.next().map(|n| PPr::parse(n)).unwrap_or_default();
        for n in parts {
            direct.apply(&PPr::parse(n));
        }
        let mut ppr = self.sheet.resolve_ppr(&direct, table.map(|t| &t.ppr));
        let style_id = self
            .sheet
            .para_style_id(direct.style.as_deref())
            .map(str::to_string);
        let mut num = None;
        let num_id = ppr.num_id.filter(|n| *n > 0);
        if let Some(num_id) = num_id {
            let style_num = self.style_num();
            let numbering = self.numbering.borrow();
            let ilvl = ppr
                .ilvl
                .or_else(|| {
                    style_id
                        .as_deref()
                        .and_then(|s| numbering.level_for_style(num_id, s, &style_num))
                })
                .unwrap_or(0);
            drop(numbering);
            let label = self
                .numbering
                .borrow_mut()
                .next_label(num_id, ilvl, &style_num);
            if let Some((text, level)) = label {
                // The list level's indents and tabs sit between the paragraph
                // style and direct formatting.
                let from_style = direct.num_id.is_none();
                let mut base = self.sheet.resolve_ppr(
                    &PPr {
                        style: direct.style.clone(),
                        ..Default::default()
                    },
                    table.map(|t| &t.ppr),
                );
                let mut lvl = level.ppr.clone();
                // Indents of the style that carries the list (or a style based on
                // it) replace the level's; indents it inherits give way to the level.
                let own = style_id.as_deref().is_some_and(|s| self.own_list_indent(s));
                if from_style && own {
                    lvl.ind_left = None;
                    lvl.ind_first = None;
                }
                base.apply(&lvl);
                base.apply(&direct);
                ppr = base;
                let mark = direct.mark.clone().unwrap_or_default();
                let mut rpr =
                    self.sheet
                        .resolve_rpr(style_id.as_deref(), &mark, table.map(|t| &t.rpr), None);
                // Labels ignore the mark's underline and take the level's own run formatting.
                rpr.underline = None;
                rpr.apply(&level.rpr);
                let mut style = self.sheet.run_style(&rpr);
                // A hidden mark hides the label of an empty paragraph only; one with
                // text keeps its label (Word for Mac: "2. text" under a hidden mark).
                if p.find("w:t").is_some() {
                    style.hidden = false;
                }
                // An empty paragraph that ends a section shows no list label.
                let section_mark = p.path(&["w:pPr", "w:sectPr"]).is_some()
                    && p.find("w:t").is_none()
                    && p.find("w:drawing").is_none();
                // An explicitly empty numbering pattern contributes no label suffix.
                let empty_none =
                    level.fmt == "none" && text.is_empty() && (level.explicit_empty_text);
                if !style.hidden && !section_mark && !empty_none {
                    let style = Rc::new(style);
                    let picture = self
                        .numbering_picture(level.picture_id, &style)
                        .map(Rc::new);
                    num = Some(NumLabel {
                        text,
                        picture,
                        picture_count: level.text.chars().count(),
                        picture_table: self
                            .source
                            .borrow()
                            .cells
                            .first()
                            .map(|cell| (part.to_owned(), cell.table)),
                        style,
                        suffix: level.suffix,
                        jc: level.jc.clone(),
                    });
                }
            }
        }
        // `beforeLines`/`afterLines` count in grid lines on a line grid, else in
        // 12 pt lines (Word's saved twips agree in 757 of 796 grid paragraphs).
        let line_unit = self.cur_sect.borrow().line_grid().unwrap_or(12.0);
        let style = self.sheet.para_style(&ppr, line_unit);
        let mark_rpr = self.sheet.resolve_rpr(
            style_id.as_deref(),
            &direct.mark.unwrap_or_default(),
            table.map(|t| &t.rpr),
            None,
        );
        let mark = Rc::new(self.sheet.run_style(&mark_rpr));
        let mut inlines = Vec::new();
        let ctx = RunCtx {
            part,
            para_style: style_id.as_deref(),
            table,
        };
        self.inline_nodes(&p.children, &ctx, &mut inlines);
        inlines.shrink_to_fit();
        if let Some(label) = &mut num {
            label.style = self.with_source(label.style.clone(), part);
        }
        // A text-wrapping break that clears floats (`w:clear` all, left or right).
        fn clears(n: &XNode) -> bool {
            n.children.iter().any(|c| {
                (c.name == "w:br" && matches!(c.attr("clear"), Some("all" | "left" | "right")))
                    || (matches!(
                        c.name.as_str(),
                        "w:r" | "w:hyperlink" | "w:ins" | "w:smartTag"
                    ) && clears(c))
            })
        }
        let clear_break = clears(p);
        let paragraph = Paragraph {
            style: self.intern_para(ParaStyle {
                style_id,
                clear_break,
                ..style
            }),
            inlines,
            mark,
            num,
        };
        self.source.borrow_mut().paragraph = previous_paragraph;
        paragraph
    }

    fn inline_nodes(&self, nodes: &[Rc<XNode>], ctx: &RunCtx, out: &mut Vec<Inline>) {
        for n in nodes {
            match n.name.as_str() {
                "w:bookmarkStart" if self.displaying() => {
                    if let Some(marker) = self.source_bookmark(n, ctx.part, false) {
                        out.push(Inline::Bookmark(marker));
                    }
                }
                "w:r" => self.run(n, ctx, out),
                "w:hyperlink" => {
                    let link = n
                        .attr("r:id")
                        .and_then(|id| self.pkg.rel_by_id(ctx.part, id))
                        .filter(|rel| rel.external)
                        .map(|rel| rel.target);
                    let anchor = n
                        .attr("anchor")
                        .filter(|_| n.attr("r:id").is_none() || link.is_some());
                    let previous = self.enter_source_link(link, anchor, ctx.part);
                    self.inline_nodes(&n.children, ctx, out);
                    self.restore_source_link(previous);
                }
                "w:smartTag" | "w:customXml" | "w:ins" | "w:moveTo" | "w:dir" | "w:bdo" => {
                    self.inline_nodes(&n.children, ctx, out)
                }
                "w:sdt" => {
                    if let Some(c) = n.child("w:sdtContent") {
                        self.inline_nodes(&c.children, ctx, out);
                    }
                }
                "w:fldSimple" => {
                    if let Some(parent) = self
                        .fields
                        .borrow_mut()
                        .last_mut()
                        .filter(|field| !field.separated)
                    {
                        parent.dynamic_instruction = true;
                    }
                    let kind = field_kind(n.attr("instr").unwrap_or(""));
                    match kind {
                        Some(k) if self.displaying() => {
                            let style = self.run_style_of(n.find("w:rPr").map(|r| r.as_ref()), ctx);
                            out.push(Inline::Field(k, style));
                        }
                        Some(_) => {}
                        None => {
                            let target =
                                source::hyperlink_target(n.attr("instr").unwrap_or(""), ctx.part);
                            if target.is_some() {
                                let previous = self.enter_source_target(
                                    target,
                                    source::reference_instruction(n.attr("instr").unwrap_or("")),
                                );
                                self.inline_nodes(&n.children, ctx, out);
                                self.restore_source_link(previous);
                            } else {
                                self.inline_nodes(&n.children, ctx, out);
                            }
                        }
                    }
                }
                "m:oMathPara" | "m:oMath" if self.displaying() => {
                    // Office's linear format: `(a+b)/c`, `x^2`, `√(x)`.
                    let text = word_math_text(n);
                    if !text.is_empty() {
                        let style = self.run_style_of(None, ctx);
                        out.push(Inline::Text(text, style));
                    }
                }
                _ => {}
            }
        }
    }

    /// Keeps the saved result of the innermost field when it is a style reference,
    /// placing the field at its first result run; true when `text` was taken.
    fn capture_style_ref(&self, text: &str, style: &Rc<RunStyle>, out: &mut Vec<Inline>) -> bool {
        let fields = self.fields.borrow();
        let Some((last, outer)) = fields.split_last() else {
            return false;
        };
        let Some(i) = last.style_ref.filter(|_| last.separated) else {
            return false;
        };
        if !outer.iter().all(|f| f.separated && !f.replaced) {
            return false;
        }
        if let Some(entry) = self.style_refs.borrow_mut().get_mut(i) {
            if entry.saved.is_empty() {
                out.push(Inline::Field(FieldKind::StyleRef(i), style.clone()));
            }
            entry.saved.push_str(text);
        }
        true
    }

    /// Inside the result of an equation object's field (EMBED Equation.3, MathType).
    fn in_equation_object(&self) -> bool {
        self.fields.borrow().iter().any(|f| {
            let mut words = f.instr.split_whitespace();
            words
                .next()
                .is_some_and(|w| w.eq_ignore_ascii_case("EMBED"))
                && words.next().is_some_and(|p| {
                    let p = p.to_ascii_lowercase();
                    p.starts_with("equation") || p.starts_with("mathtype")
                })
        })
    }

    /// Records the VML picture parts of an equation OLE object (`o:OLEObject` with an
    /// Equation or MathType ProgID).
    fn note_equation_object(&self, object: &Rc<XNode>, part: &str) {
        let equation = object
            .find("o:OLEObject")
            .and_then(|o| o.attr("ProgID"))
            .is_some_and(|p| {
                let p = p.to_ascii_lowercase();
                p.starts_with("equation") || p.starts_with("mathtype")
            });
        if !equation {
            return;
        }
        let mut images = Vec::new();
        object.find_all("v:imagedata", &mut images);
        for i in images {
            if let Some(t) = i
                .attr("r:id")
                .or_else(|| i.attr("r:pict"))
                .and_then(|id| self.rel_target(part, id))
            {
                self.equation_parts.borrow_mut().insert(t);
            }
        }
    }

    /// Picture parts that preview an equation object.
    pub fn equation_parts(&self) -> std::collections::HashSet<String> {
        self.equation_parts.borrow().clone()
    }

    /// The document's header and footer STYLEREF fields.
    pub fn style_references(&self) -> Vec<StyleRef> {
        self.style_refs.borrow().clone()
    }

    fn displaying(&self) -> bool {
        self.fields
            .borrow()
            .iter()
            .all(|f| f.separated && !f.replaced)
    }

    /// Inside the result of a table of contents field.
    fn in_toc(&self) -> bool {
        self.fields.borrow().iter().any(|f| {
            f.separated
                && f.instr
                    .split_whitespace()
                    .next()
                    .is_some_and(|w| w.eq_ignore_ascii_case("TOC"))
        })
    }

    fn run_style_of(&self, rpr: Option<&XNode>, ctx: &RunCtx) -> Rc<RunStyle> {
        let style = self.base_run_style_of(rpr, ctx);
        self.with_source(style, ctx.part)
    }

    fn base_run_style_of(&self, rpr: Option<&XNode>, ctx: &RunCtx) -> Rc<RunStyle> {
        let toc = self.in_toc();
        let key = format!(
            "{}|{}|{:?}|{toc}",
            ctx.para_style.unwrap_or(""),
            ctx.table.is_some(),
            rpr.map(node_hash)
        );
        if ctx.table.is_none()
            && let Some(hit) = self.styles_cache.borrow().get(&key)
        {
            return hit.clone();
        }
        let mut direct = rpr.map(RPr::parse).unwrap_or_default();
        // Word shows table of contents entries without the Hyperlink character style.
        if toc && direct.r_style.as_deref() == Some("Hyperlink") {
            direct.r_style = None;
        }
        let r = self
            .sheet
            .resolve_rpr(ctx.para_style, &direct, ctx.table.map(|t| &t.rpr), None);
        let style = Rc::new(self.sheet.run_style(&r));
        if ctx.table.is_none() {
            self.styles_cache.borrow_mut().insert(key, style.clone());
        }
        style
    }

    /// One shared allocation per distinct paragraph style.
    fn intern_para(&self, s: ParaStyle) -> Rc<ParaStyle> {
        self.para_pool
            .borrow_mut()
            .entry(format!("{s:?}"))
            .or_insert_with(|| Rc::new(s))
            .clone()
    }

    fn run(&self, r: &XNode, ctx: &RunCtx, out: &mut Vec<Inline>) {
        let base_style = self.base_run_style_of(r.child("w:rPr").map(|n| n.as_ref()), ctx);
        let hidden = base_style.hidden;
        for c in &r.children {
            let style = self.with_source(base_style.clone(), ctx.part);
            match c.name.as_str() {
                "w:fldChar" => {
                    let checkbox = c.find("w:checkBox").map(|b| {
                        let on =
                            |n: &str| b.child(n).map(|v| crate::office::docx::props::on_off(v));
                        on("w:checked").or(on("w:default")).unwrap_or(false)
                    });
                    let dropdown = c.find("w:ddList").and_then(|d| dropdown_entry(d));
                    // Page fields are live in headers and footers; in the body Word
                    // shows their saved result.
                    let live = ctx.part.contains("header") || ctx.part.contains("footer");
                    self.field_char(
                        c.attr("fldCharType").unwrap_or(""),
                        &style,
                        (checkbox, dropdown),
                        live,
                        out,
                    )
                }
                "w:instrText" => {
                    if let Some(f) = self.fields.borrow_mut().last_mut()
                        && !f.separated
                    {
                        f.instr.push_str(&c.text);
                    }
                }
                "w:t" if !hidden && self.capture_style_ref(&c.text, &style, out) => {}
                _ if hidden || !self.displaying() => {}
                "w:t" => push_text(out, &c.text, &style),
                "w:ruby" => {
                    if let Some(base) = c.child("w:rubyBase") {
                        let mut runs = Vec::new();
                        self.inline_nodes(&base.children, ctx, &mut runs);
                        if let (Some(pr), Some(rt)) = (c.child("w:rubyPr"), c.child("w:rt")) {
                            let mut guide = Vec::new();
                            self.inline_nodes(&rt.children, ctx, &mut guide);
                            if let Some(ruby) = ruby_group(&runs, &guide, pr) {
                                out.push(Inline::Ruby(Box::new(ruby)));
                                continue;
                            }
                        }
                        out.extend(runs);
                    }
                }
                "w:tab" | "w:ptab" => out.push(Inline::Tab(style.clone())),
                "w:br" => {
                    let kind = match c.attr("type") {
                        Some("page") => BreakKind::Page,
                        Some("column") => BreakKind::Column,
                        _ => BreakKind::Line,
                    };
                    out.push(Inline::Break(kind, style.clone()));
                }
                "w:cr" => out.push(Inline::Break(BreakKind::Line, style.clone())),
                "w:separator" => out.push(note_separator(&style, 144.0)),
                "w:continuationSeparator" => out.push(Inline::ContinuationSeparator(style.clone())),
                "w:noBreakHyphen" => push_text(out, "\u{2011}", &style),
                "w:softHyphen" => push_text(out, "\u{AD}", &style),
                "w:sym" => {
                    let ch = c
                        .attr("char")
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                        .and_then(char::from_u32);
                    if let Some(ch) = ch {
                        let font = c.attr("font").unwrap_or(&style.font).to_string();
                        let mapped = crate::office::pptx::fonts::map_symbol_char(&font, ch);
                        let mut s = (*style).clone();
                        s.symbol = Some((font.clone(), ch));
                        if mapped.is_none() {
                            s.font = font;
                        }
                        let text = mapped.unwrap_or(ch).to_string();
                        out.push(Inline::Text(text, Rc::new(s)));
                    }
                }
                "w:pgNum" => out.push(Inline::Field(FieldKind::Page(None), style.clone())),
                "w:footnoteReference" => {
                    let foot = c.name == "w:footnoteReference";
                    if c.attr_bool("customMarkFollows") != Some(true) && foot {
                        let mut n = self.footnotes.borrow_mut();
                        *n += 1;
                        // `w:footnotePr` of the section, else of the settings.
                        let sect = self.cur_sect.borrow();
                        let start = sect.note_start.or(self.settings.note_start).unwrap_or(1);
                        let fmt = sect
                            .note_fmt
                            .clone()
                            .or_else(|| self.settings.note_fmt.clone())
                            .unwrap_or_else(|| "decimal".into());
                        let shown = crate::office::docx::numbering::format_number(
                            &fmt,
                            *n as i64 + start - 1,
                        );
                        push_text(out, &shown, &style);
                        if let Some(id) = c.attr_i64("id") {
                            self.note_numbers.borrow_mut().insert(id, shown);
                        }
                    }
                    if let (true, Some(id)) = (foot, c.attr_i64("id")) {
                        out.push(Inline::Note(id));
                    }
                }
                "w:endnoteReference" => {
                    // Endnotes count apart from footnotes, in lower-case roman by default.
                    let mut order = self.endnote_order.borrow_mut();
                    let section = self.body_section.get();
                    let shown = if c.attr_bool("customMarkFollows") == Some(true) {
                        String::new()
                    } else {
                        let sect = self.cur_sect.borrow();
                        let start = sect
                            .endnote_start
                            .or(self.settings.endnote_start)
                            .unwrap_or(1);
                        let restart = sect
                            .endnote_restart
                            .as_deref()
                            .or(self.settings.endnote_restart.as_deref())
                            == Some("eachSect");
                        let number = match self.endnote_counter.get() {
                            Some((previous, number)) if previous == section || !restart => {
                                number.saturating_add(1)
                            }
                            _ => start,
                        };
                        self.endnote_counter.set(Some((section, number)));
                        let fmt = sect
                            .endnote_fmt
                            .as_deref()
                            .or(self.settings.endnote_fmt.as_deref())
                            .unwrap_or("lowerRoman");
                        crate::office::docx::numbering::format_number(fmt, number)
                    };
                    push_text(out, &shown, &style);
                    if let Some(id) = c.attr_i64("id") {
                        order.push((id, shown, section));
                    }
                }
                "w:footnoteRef" | "w:endnoteRef" => {
                    let n = self.current_note.borrow().clone();
                    if let Some(n) = n {
                        push_text(out, &n, &style);
                    }
                }
                "w:drawing" => {
                    for d in &c.children {
                        if let Some(i) = self.drawing(d, ctx.part, &style) {
                            out.push(i);
                        }
                    }
                }
                "w:pict" | "w:object" => {
                    if c.name == "w:object" {
                        self.note_equation_object(c, ctx.part);
                    }
                    if let Some(i) = self.vml(c, ctx.part, &style) {
                        out.push(i);
                    }
                }
                _ => {}
            }
        }
    }

    fn numbering_picture(&self, id: Option<i64>, style: &Rc<RunStyle>) -> Option<Picture> {
        let (definition, part) = {
            let numbering = self.numbering.borrow();
            (
                numbering.pictures.get(&id?)?.clone(),
                numbering.picture_part.clone()?,
            )
        };
        let inline = if let Some(pict) = definition.child("w:pict") {
            self.vml(pict, &part, style)
        } else {
            let drawing = definition.child("w:drawing")?;
            self.drawing(drawing.child("wp:inline")?, &part, style)
        }?;
        let Inline::Picture(picture, _) = inline else {
            return None;
        };
        if picture.part.is_none()
            || !picture.w.is_finite()
            || !picture.h.is_finite()
            || picture.w <= 0.0
            || picture.h <= 0.0
        {
            return None;
        }
        // Raster labels can retain their outline; deferred graphical payloads
        // keep the existing text-label fallback until their sizing is supported.
        if picture.shape.as_ref().is_some_and(|shape| {
            !shape.texts.is_empty()
                || !shape.graphics.is_empty()
                || shape
                    .items
                    .iter()
                    .any(|item| !matches!(item, crate::office::pptx::scene::Item::Path(_)))
        }) {
            return None;
        }
        Some(picture)
    }

    fn field_char(
        &self,
        kind: &str,
        style: &Rc<RunStyle>,
        (checkbox, dropdown): (Option<bool>, Option<String>),
        live: bool,
        out: &mut Vec<Inline>,
    ) {
        match kind {
            "begin" => {
                let mut fields = self.fields.borrow_mut();
                if let Some(parent) = fields.last_mut().filter(|field| !field.separated) {
                    parent.dynamic_instruction = true;
                }
                fields.push(OpenField {
                    source_id: self.next_source_link_id(),
                    instr: String::new(),
                    separated: false,
                    dynamic_instruction: false,
                    replaced: false,
                    checkbox,
                    dropdown,
                    style_ref: None,
                });
            }
            "separate" => {
                let displaying_outer = {
                    let f = self.fields.borrow();
                    let n = f.len();
                    n > 0 && f[..n - 1].iter().all(|o| o.separated && !o.replaced)
                };
                let mut fields = self.fields.borrow_mut();
                if let Some(f) = fields.last_mut() {
                    f.separated = true;
                    // A drop-down form field shows its selected entry, whatever its saved result.
                    if let Some(t) = &f.dropdown {
                        f.replaced = true;
                        if displaying_outer && !style.hidden {
                            push_text(out, t, style);
                        }
                    }
                    if let Some(k) = field_kind(&f.instr).filter(|_| live) {
                        f.replaced = true;
                        if displaying_outer && !style.hidden {
                            out.push(Inline::Field(k, style.clone()));
                        }
                    } else if let Some(v) =
                        live.then(|| self.doc_props.field_text(&f.instr)).flatten()
                    {
                        f.replaced = true;
                        if displaying_outer && !style.hidden {
                            push_text(out, &v, style);
                        }
                    } else if let Some((name, number)) = style_ref_target(&f.instr).filter(|_| live)
                    {
                        // Each page shows its own heading; the saved result stays for
                        // pages without one. The field takes its first result run's place.
                        if displaying_outer && !style.hidden {
                            let mut refs = self.style_refs.borrow_mut();
                            f.style_ref = Some(refs.len());
                            refs.push(StyleRef {
                                name,
                                number,
                                saved: String::new(),
                            });
                            f.replaced = true;
                        }
                    }
                }
            }
            "end" => {
                let popped = self.fields.borrow_mut().pop();
                if let Some(f) = popped {
                    // Word draws a legacy check box field as a box glyph.
                    if let (Some(on), true) = (f.checkbox, self.displaying() && !style.hidden) {
                        // Drawn in automatic colour without the run's shading.
                        let plain = Rc::new(RunStyle {
                            color: None,
                            shading: None,
                            highlight: None,
                            ..(**style).clone()
                        });
                        push_text(out, if on { "\u{2612}" } else { "\u{2610}" }, &plain);
                    }
                    // Without a result part, the entry goes at the field's end.
                    if let (Some(t), false, true) =
                        (&f.dropdown, f.separated, self.displaying() && !style.hidden)
                    {
                        push_text(out, t, style);
                    }
                    if !f.separated
                        && self.displaying()
                        && !style.hidden
                        && let Some(k) = field_kind(&f.instr)
                    {
                        out.push(Inline::Field(k, style.clone()));
                    }
                }
            }
            _ => {}
        }
    }

    /// Relationship target of `id` from `part`.
    fn rel_target(&self, part: &str, id: &str) -> Option<String> {
        if let Some(target) = self.imported_asset(part, id) {
            return Some(target);
        }
        self.pkg
            .rel_by_id(part, id)
            .filter(|r| !r.external)
            .map(|r| r.target)
    }

    /// `wp:inline` and `wp:anchor` drawings.
    fn drawing(&self, d: &XNode, part: &str, style: &Rc<RunStyle>) -> Option<Inline> {
        let inline = d.name == "wp:inline";
        if !inline && d.name != "wp:anchor" {
            return None;
        }
        let ext = d.child("wp:extent")?;
        let emu = |n: &XNode, k: &str| n.attr_f64(k).unwrap_or(0.0) / EMU_PER_PT;
        let (mut w, mut h) = (emu(ext, "cx"), emu(ext, "cy"));
        if inline {
            // An inline object's effect extent (shadows, glow) takes room in the line.
            if let Some(e) = d.child("wp:effectExtent") {
                w += emu(e, "l") + emu(e, "r");
                h += emu(e, "t") + emu(e, "b");
            }
        }
        // `wp14:sizeRelH`/`sizeRelV`: a size in thousandths of a percent of the page or a margin.
        if !inline {
            let sp = self.cur_sect.borrow();
            let rel = |name: &str, pct: &str| -> Option<(String, f64)> {
                let n = d.child(name)?;
                let v = n
                    .child(pct)?
                    .text
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|v| *v > 0.0)?;
                Some((
                    n.attr("relativeFrom").unwrap_or("page").to_string(),
                    v / 100_000.0,
                ))
            };
            if let Some((from, f)) = rel("wp14:sizeRelH", "wp14:pctWidth") {
                w = f * match from.as_str() {
                    "margin" => sp.page_w - sp.left - sp.right,
                    "leftMargin" | "insideMargin" => sp.left,
                    "rightMargin" | "outsideMargin" => sp.right,
                    _ => sp.page_w,
                };
            }
            // A shape that resizes to fit its text keeps its saved height.
            let autofit = d.find("a:spAutoFit").is_some();
            if let Some((from, f)) = rel("wp14:sizeRelV", "wp14:pctHeight").filter(|_| !autofit) {
                h = f * match from.as_str() {
                    "margin" => sp.page_h - sp.top - sp.bottom,
                    "topMargin" | "insideMargin" => sp.top,
                    "bottomMargin" | "outsideMargin" => sp.bottom,
                    _ => sp.page_h,
                };
            }
        }
        let data = d.path(&["a:graphic", "a:graphicData"]);
        let pic = data.and_then(|g| g.child("pic:pic"));
        let blip = pic.and_then(|p| p.find("a:blip"));
        let target = blip
            .and_then(|b| b.attr("r:embed").or_else(|| b.attr("r:link")))
            .and_then(|id| self.rel_target(part, id));
        if let Some(t) = target.as_ref().filter(|_| self.in_equation_object()) {
            self.equation_parts.borrow_mut().insert(t.clone());
        }
        let crop = pic
            .and_then(|p| p.find("a:srcRect"))
            .map(|s| {
                let f = |k: &str| s.attr_f64(k).unwrap_or(0.0) / 100_000.0;
                (f("l"), f("t"), f("r"), f("b"))
            })
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
        let alt = d
            .child("wp:docPr")
            .and_then(|p| p.attr("descr"))
            .map(str::to_string);
        // Shapes, groups and text boxes are drawn from their DrawingML.
        let drawn = data.and_then(|g| {
            g.children
                .iter()
                .find(|c| matches!(c.name.as_str(), "wps:wsp" | "wpg:wgp" | "wpc:wpc"))
        });
        let (sw, sh) = if inline {
            (emu(ext, "cx"), emu(ext, "cy"))
        } else {
            (w, h)
        };
        let shape = drawn.and_then(|n| self.shape(n, sw, sh, part)).or_else(|| {
            // A picture's outline is drawn over it as a one-item shape.
            let pic = pic?;
            let item = self.with_ctx(part, |ctx| {
                crate::office::docx::shapes::picture_outline(
                    pic,
                    crate::office::pptx::scene::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: sw,
                        h: sh,
                    },
                    ctx,
                )
            })?;
            Some(Rc::new(crate::office::docx::shapes::ShapeSpec {
                items: vec![item],
                texts: Vec::new(),
                ..Default::default()
            }))
        });
        // Recolouring and opacity of the picture's `a:blip`.
        let effects = blip
            .and_then(|b| self.with_ctx(part, |ctx| ctx.blip_image(b, None)))
            .map(|i| i.effects)
            .unwrap_or_default();
        let alpha = blip
            .and_then(|b| b.child("a:alphaModFix"))
            .and_then(|a| a.attr_f64("amt"))
            .map_or(1.0, |v| (v / 100_000.0).clamp(0.0, 1.0) as f32);
        // A picture turned or flipped by its own `a:xfrm` is drawn as a shape
        // item, so the turn applies to the image and its outline together.
        let (target, mut shape) = match pic.filter(|_| drawn.is_none()).and_then(|p| turn_of(p)) {
            Some(t) if target.is_some() => {
                let mut items = vec![crate::office::pptx::scene::Item::Image(
                    crate::office::pptx::scene::ImageItem {
                        image: crate::office::pptx::scene::ImageRef {
                            part: target.unwrap_or_default(),
                            effects: effects.clone(),
                        },
                        transform: tiny_skia::Transform::from_row(
                            sw as f32, 0.0, 0.0, sh as f32, 0.0, 0.0,
                        ),
                        crop,
                        alpha,
                        clip: None,
                        shape: None,
                        alt_text: alt.clone(),
                    },
                )];
                items.extend(shape.iter().flat_map(|s| s.items.iter().cloned()));
                let c = ((sw / 2.0) as f32, (sh / 2.0) as f32);
                let m = tiny_skia::Transform::from_translate(c.0, c.1)
                    .pre_rotate(t.0 as f32)
                    .pre_scale(if t.1 { -1.0 } else { 1.0 }, if t.2 { -1.0 } else { 1.0 })
                    .pre_translate(-c.0, -c.1);
                for it in items.iter_mut() {
                    crate::office::docx::layout::transform_item(it, m);
                }
                let texts = Vec::new();
                (
                    None,
                    Some(Rc::new(crate::office::docx::shapes::ShapeSpec {
                        raster_picture: true,
                        items,
                        texts,
                        ..Default::default()
                    })),
                )
            }
            _ => (target, shape),
        };
        // Inline quarter-turn sectors exchange layout dimensions around the
        // source center. Paint and deferred text share the resulting origin;
        // source effect extents remain attached to their layout sides.
        if inline {
            let angle = drawn.or(pic).and_then(|n| turn_of(n)).map_or(0.0, |t| t.0);
            if (45.0..135.0).contains(&angle.rem_euclid(180.0))
                && let Some(spec) = shape.as_mut()
            {
                let effect = d.child("wp:effectExtent");
                let left = effect.map_or(0.0, |e| emu(e, "l"));
                let top = effect.map_or(0.0, |e| emu(e, "t"));
                Rc::make_mut(spec).translate((sh - sw) / 2.0 + left, (sw - sh) / 2.0 + top);
                w += sh - sw;
                h += sw - sh;
            }
        }
        // Charts and diagrams are drawn later by `graphics.rs`.
        let graphic = data
            .filter(|g| {
                let uri = g.attr("uri").unwrap_or("");
                drawn.is_none()
                    && (uri.ends_with("/chart")
                        || uri.ends_with("/chartex")
                        || uri.ends_with("/diagram"))
            })
            .map(|g| (g.clone(), part.to_string()));
        let shadow = None;
        let picture = Picture {
            shadow,
            graphic,
            effects,
            alpha,
            part: target,
            w: w.max(0.0),
            h: h.max(0.0),
            crop,
            alt,
            shape,
        };
        if inline {
            return Some(Inline::Picture(picture, style.clone()));
        }
        let pos = |name: &str, off_name: &str| {
            let n = d.child(name);
            let rel = n
                .and_then(|n| n.attr("relativeFrom"))
                .unwrap_or("")
                .to_string();
            let off = n
                .and_then(|n| n.child(off_name))
                .and_then(|o| o.text.trim().parse::<f64>().ok())
                .map(|v| v / EMU_PER_PT);
            let align = n
                .and_then(|n| n.child("wp:align"))
                .map(|a| a.text.trim().to_string());
            (rel, off.ok_or(align.unwrap_or_default()))
        };
        let simple = d.attr_bool("simplePos") == Some(true);
        let (hrel, mut x) = pos("wp:positionH", "wp:posOffset");
        let (vrel, mut y) = pos("wp:positionV", "wp:posOffset");
        let (mut h_rel, mut v_rel) = (
            match hrel.as_str() {
                "page" => HRel::Page,
                "margin" | "leftMargin" | "rightMargin" | "insideMargin" | "outsideMargin" => {
                    HRel::Margin
                }
                "character" => HRel::Character,
                _ => HRel::Column,
            },
            match vrel.as_str() {
                "page" | "topMargin" | "bottomMargin" => VRel::Page,
                "margin" => VRel::Margin,
                "line" => VRel::Line,
                _ => VRel::Paragraph,
            },
        );
        // The left and right margin strips place the object from the page edge.
        if matches!(hrel.as_str(), "leftMargin" | "rightMargin") {
            let sp = self.cur_sect.borrow();
            let (x0, strip) = if hrel == "leftMargin" {
                (0.0, sp.left)
            } else {
                (sp.page_w - sp.right, sp.right)
            };
            x = Ok(match &x {
                Ok(off) => x0 + off,
                Err(a) if a == "right" || a == "outside" => x0 + strip - w,
                Err(a) if a == "center" => x0 + (strip - w) / 2.0,
                Err(_) => x0,
            });
            h_rel = HRel::Page;
        }
        if simple && let Some(sp) = d.child("wp:simplePos") {
            x = Ok(emu(sp, "x"));
            y = Ok(emu(sp, "y"));
            h_rel = HRel::Page;
            v_rel = VRel::Page;
        }
        let wrap = if d.child("wp:wrapTopAndBottom").is_some() {
            Wrap::TopAndBottom
        } else if d.child("wp:wrapSquare").is_some()
            || d.child("wp:wrapTight").is_some()
            || d.child("wp:wrapThrough").is_some()
        {
            Wrap::Square
        } else {
            Wrap::None
        };
        Some(Inline::Anchor(Box::new(Anchored {
            pic: picture,
            h_rel,
            x,
            v_rel,
            y,
            behind: d.attr_bool("behindDoc").unwrap_or(false),
            z: d.attr_i64("relativeHeight").unwrap_or(0),
            wrap,
            contour: ["wp:wrapTight", "wp:wrapThrough"].iter().find_map(|name| {
                let node = d.child(name)?.child("wp:wrapPolygon")?;
                let points = node
                    .children
                    .iter()
                    .filter_map(|p| {
                        Some((
                            p.attr_i64("x")? as f64 / 21600.0,
                            p.attr_i64("y")? as f64 / 21600.0,
                        ))
                    })
                    .collect();
                Some(crate::office::docx::wrap::Contour {
                    points,
                    through: *name == "wp:wrapThrough",
                })
            }),
            dist: drawing_wrap_distances(d),
            wrap_side: ["wp:wrapSquare", "wp:wrapTight", "wp:wrapThrough"]
                .iter()
                .find_map(|k| d.child(k).and_then(|w| w.attr("wrapText")))
                .unwrap_or("bothSides")
                .to_string(),
            text_items: Vec::new(),
            frame_children: Vec::new(),
            text_frame: false,
            turned: data
                .and_then(|g| {
                    g.children
                        .iter()
                        .find(|c| matches!(c.local(), "wsp" | "wgp" | "wpc" | "pic"))
                })
                .and_then(|n| {
                    n.children
                        .iter()
                        .find(|c| matches!(c.local(), "spPr" | "grpSpPr"))
                })
                .and_then(|s| s.child("a:xfrm"))
                .and_then(|x| x.attr_f64("rot"))
                .is_some_and(|r| quarter_turn(r / 60000.0)),
        })))
    }

    /// A DrawingML shape or group of `w`×`h` points, with text box contents.
    /// Runs `f` with the DrawingML style context of `part`.
    fn with_ctx<T>(
        &self,
        part: &str,
        f: impl FnOnce(&crate::office::pptx::style::StyleCtx) -> T,
    ) -> T {
        let default_theme;
        let (theme, theme_part) = match &self.theme {
            Some((t, p)) => (t, p.as_str()),
            None => {
                default_theme = crate::office::pptx::theme::Theme::default();
                (&default_theme, "")
            }
        };
        f(&crate::office::pptx::style::StyleCtx {
            theme,
            colors: &theme.colors,
            part,
            theme_part,
            pkg: self.pkg,
        })
    }

    fn shape(
        &self,
        n: &XNode,
        w: f64,
        h: f64,
        part: &str,
    ) -> Option<Rc<crate::office::docx::shapes::ShapeSpec>> {
        let default_theme;
        let (theme, theme_part) = match &self.theme {
            Some((t, p)) => (t, p.as_str()),
            None => {
                default_theme = crate::office::pptx::theme::Theme::default();
                (&default_theme, "")
            }
        };
        let ctx = crate::office::pptx::style::StyleCtx {
            theme,
            colors: &theme.colors,
            part,
            theme_part,
            pkg: self.pkg,
        };
        let text_of = |c: &XNode| self.blocks(&c.children, part, None);
        let spec = crate::office::docx::shapes::build(n, w, h, &ctx, &text_of);
        (!spec.items.is_empty() || !spec.texts.is_empty() || !spec.graphics.is_empty())
            .then(|| Rc::new(spec))
    }

    /// A VML shape's fill, outline and text box (`v:textbox`) in local points.
    #[expect(
        clippy::unwrap_in_result,
        reason = "the gradient arm matches only when `vfill` is present"
    )]
    fn vml_shape(
        &self,
        shape: &XNode,
        w: f64,
        h: f64,
        part: &str,
    ) -> Option<Rc<crate::office::docx::shapes::ShapeSpec>> {
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        if shape.name == "v:group" {
            return self.vml_group(shape, w, h, part);
        }
        if shape.child("v:imagedata").is_some() {
            return None;
        }
        let mut spec = crate::office::docx::shapes::ShapeSpec::default();
        let rect = crate::office::pptx::textlayout::rect_path(0.0, 0.0, w, h)?;
        let flag = |k: &str| shape.attr(k).and_then(crate::office::pptx::xml::parse_bool);
        let color = |k: &str| shape.attr(k).and_then(vml_color);
        let opacity = shape
            .child("v:fill")
            .and_then(|f| f.attr("opacity"))
            .and_then(vml_fraction)
            .unwrap_or(1.0);
        let fill_color = if flag("filled") == Some(false) {
            None
        } else {
            color("fillcolor").map(|mut c| {
                c.a *= opacity as f32;
                c
            })
        };
        // WordArt (`v:textpath`): the string stretched over the shape, filled.
        if let Some(tp) = shape.child("v:textpath") {
            let text = tp.attr("string").unwrap_or("").to_string();
            if text.trim().is_empty() {
                return None;
            }
            let css = parse_css(tp.attr("style").unwrap_or(""));
            let mut rpr = RPr::default();
            if let Some(f) = css.get("font-family") {
                rpr.ascii = Some(crate::office::docx::props::FontRef::Name(
                    f.trim_matches(|c| c == '"' || c == '\'').to_string(),
                ));
                rpr.h_ansi = rpr.ascii.clone();
                rpr.east_asia = rpr.ascii.clone();
                rpr.cs = rpr.ascii.clone();
            }
            rpr.sz = Some(36.0);
            rpr.b = css.get("font-weight").map(|v| v == "bold");
            rpr.i = css.get("font-style").map(|v| v == "italic");
            let mut rs = self.sheet.run_style(&rpr);
            rs.color = fill_color.or(Some(crate::office::pptx::color::Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            }));
            let rs = Rc::new(rs);
            let para = Paragraph {
                style: crate::office::docx::styles::ParaStyle::from_ppr(&PPr::default(), 12.0)
                    .into(),
                inlines: vec![Inline::Text(text, rs.clone())],
                mark: rs,
                num: None,
            };
            spec.texts.push(crate::office::docx::shapes::TextBox {
                rect: crate::office::pptx::scene::Rect {
                    x: 0.0,
                    y: 0.0,
                    w,
                    h,
                },
                inset: [0.0; 4],
                v_anchor: "t".into(),
                blocks: vec![Block::Para(para)],
                bg: None,
                font_color: None,
                transform: None,
                upright: false,
                fit: true,
                clip: false,
                grow: false,
                nowrap: false,
                vert: None,
            });
            return Some(Rc::new(spec));
        }
        let vfill = shape.child("v:fill");
        let fill = match (vfill.and_then(|f| f.attr("type")), fill_color) {
            // A two-colour gradient: `angle` turns it (0: top to bottom), `focus`
            // places `color2` (0: end, 100: start, ±50: middle).
            (Some("gradient"), Some(c1)) => {
                let f = vfill.unwrap();
                let mut c2 = f.attr("color2").and_then(vml_color).unwrap_or(
                    crate::office::pptx::color::Rgba {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                );
                c2.a *= opacity as f32;
                let focus = f
                    .attr("focus")
                    .and_then(|v| v.trim_end_matches('%').parse::<f64>().ok())
                    .unwrap_or(0.0);
                let colors = match focus.round() as i64 {
                    100 => vec![c2, c1],
                    50 => vec![c1, c2, c1],
                    -50 => vec![c2, c1, c2],
                    _ => vec![c1, c2],
                };
                let n = colors.len() as f32 - 1.0;
                let stops = colors
                    .into_iter()
                    .enumerate()
                    .map(|(i, color)| crate::office::pptx::scene::GradientStop {
                        pos: i as f32 / n,
                        color,
                    })
                    .collect();
                let a = f
                    .attr("angle")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0)
                    .to_radians();
                let (dx, dy) = (-a.sin() * w / 2.0, a.cos() * h / 2.0);
                Some(crate::office::pptx::scene::Paint::Linear {
                    start: (w / 2.0 - dx, h / 2.0 - dy),
                    end: (w / 2.0 + dx, h / 2.0 + dy),
                    stops,
                })
            }
            (_, c) => c.map(crate::office::pptx::scene::Paint::Solid),
        };
        // A picture fill (`type="frame"`, `tile`) is stretched over the shape.
        let picture_fill = vfill
            .filter(|f| matches!(f.attr("type"), Some("frame") | Some("tile")))
            .and_then(|f| f.attr("r:id"))
            .and_then(|id| self.rel_target(part, id));
        let fill = if picture_fill.is_some() { None } else { fill };
        if let Some(target) = picture_fill {
            spec.items.push(crate::office::pptx::scene::Item::Image(
                crate::office::pptx::scene::ImageItem {
                    image: crate::office::pptx::scene::ImageRef {
                        part: target,
                        effects: Vec::new(),
                    },
                    transform: tiny_skia::Transform::from_row(
                        w as f32, 0.0, 0.0, h as f32, 0.0, 0.0,
                    ),
                    crop: (0.0, 0.0, 0.0, 0.0),
                    alpha: opacity as f32,
                    clip: None,
                    shape: None,
                    alt_text: None,
                },
            ));
        }
        let stroke = (flag("stroked") != Some(false)).then(|| crate::office::pptx::scene::Stroke {
            paint: crate::office::pptx::scene::Paint::Solid(color("strokecolor").unwrap_or(
                crate::office::pptx::color::Rgba {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                },
            )),
            width: shape.attr("strokeweight").and_then(css_len).unwrap_or(0.75),
            dash: Vec::new(),
            cap: crate::office::pptx::scene::LineCap::Flat,
            join: crate::office::pptx::scene::LineJoin::Miter,
            head: None,
            tail: None,
        });
        if fill.is_some() || stroke.is_some() {
            // The outline of a VML preset (`v:oval`, `v:roundrect`, `o:spt` types), else a rectangle.
            let prst = vml_preset(shape);
            let custom = shape
                .attr("path")
                .or_else(|| shape.child("v:path").and_then(|p| p.attr("v")))
                .filter(|_| prst.is_none())
                .and_then(|p| vml_path(p, shape, w, h));
            let paths = match (prst, custom) {
                (Some(p), _) => crate::office::docx::shapes::preset_paths(p, w, h),
                (None, Some(c)) => vec![c],
                (None, None) => vec![(rect, true, true, false)],
            };
            for (path, f, s, curve) in paths {
                spec.items.push(crate::office::pptx::scene::Item::Path(
                    crate::office::pptx::scene::PathItem {
                        path,
                        fill: fill.clone().filter(|_| f),
                        stroke: stroke.clone().filter(|_| s),
                        has_curve: curve,
                        shape: None,
                        from_text: false,
                    },
                ));
            }
        }
        if let Some(tb) = shape.child("v:textbox")
            && let Some(content) = tb.find("w:txbxContent")
        {
            // `layout-flow` turns the text: vertical (down the box) or ideographic.
            let flow = parse_css(tb.attr("style").unwrap_or(""));
            let vml_flow = match (
                flow.get("layout-flow").map(String::as_str),
                flow.get("mso-layout-flow-alt").map(String::as_str),
            ) {
                (Some("vertical"), Some("bottom-to-top")) => Some("vert270".to_string()),
                (Some("vertical"), _) => Some("vert".to_string()),
                (Some("vertical-ideographic"), _) => Some("eaVert".to_string()),
                _ => None,
            };
            // `inset` lists left, top, right, bottom; VML's default is 0.1 in, 0.05 in.
            let ins: Vec<f64> = tb
                .attr("inset")
                .unwrap_or("")
                .split(',')
                .map(|v| css_len(v).unwrap_or(f64::NAN))
                .collect();
            let get = |i: usize, d: f64| ins.get(i).copied().filter(|v| v.is_finite()).unwrap_or(d);
            let blocks = self.blocks(&content.children, part, None);
            if !blocks.is_empty() {
                spec.texts.push(crate::office::docx::shapes::TextBox {
                    rect: crate::office::pptx::scene::Rect {
                        x: 0.0,
                        y: 0.0,
                        w,
                        h,
                    },
                    inset: [get(0, 7.2), get(1, 3.6), get(2, 7.2), get(3, 3.6)],
                    v_anchor: "t".into(),
                    blocks,
                    bg: match &fill {
                        Some(crate::office::pptx::scene::Paint::Solid(c)) => Some(*c),
                        _ => None,
                    },
                    font_color: None,
                    transform: None,
                    upright: false,
                    fit: false,
                    clip: false,
                    grow: tb
                        .attr("style")
                        .is_some_and(|s| s.replace(' ', "").contains("mso-fit-shape-to-text:t")),
                    nowrap: shape
                        .attr("style")
                        .unwrap_or("")
                        .replace(' ', "")
                        .contains("mso-wrap-style:none"),
                    vert: vml_flow,
                });
            }
        }
        (!spec.items.is_empty() || !spec.texts.is_empty()).then(|| Rc::new(spec))
    }

    /// A VML group: children placed in its `coordorigin`/`coordsize` space,
    /// scaled to the group's `w`×`h` points.
    fn vml_group(
        &self,
        group: &XNode,
        w: f64,
        h: f64,
        part: &str,
    ) -> Option<Rc<crate::office::docx::shapes::ShapeSpec>> {
        let pair = |k: &str, d: (f64, f64)| {
            group
                .attr(k)
                .and_then(|v| {
                    let mut it = v.split(',').map(|x| x.trim().parse::<f64>().ok());
                    Some((it.next()??, it.next().flatten().unwrap_or(d.1)))
                })
                .unwrap_or(d)
        };
        let (ox, oy) = pair("coordorigin", (0.0, 0.0));
        let (cw, ch) = pair("coordsize", (1000.0, 1000.0));
        let (sx, sy) = (w / cw.max(1.0), h / ch.max(1.0));
        let mut spec = crate::office::docx::shapes::ShapeSpec::default();
        for c in &group.children {
            if !matches!(
                c.name.as_str(),
                "v:shape" | "v:rect" | "v:roundrect" | "v:oval" | "v:group"
            ) {
                continue;
            }
            let css = parse_css(c.attr("style").unwrap_or(""));
            let num = |k: &str| {
                css.get(k)
                    .and_then(|v| v.trim().parse::<f64>().ok())
                    .unwrap_or(0.0)
            };
            let (cx, cy) = ((num("left") - ox) * sx, (num("top") - oy) * sy);
            let (cw2, ch2) = (num("width") * sx, num("height") * sy);
            // A picture in the group (`v:imagedata`) is drawn over its box.
            let image = c
                .child("v:imagedata")
                .and_then(|i| i.attr("r:id").or_else(|| i.attr("r:pict")))
                .and_then(|id| self.rel_target(part, id));
            if let Some(target) = image {
                if cw2 > 0.0 && ch2 > 0.0 {
                    spec.items.push(crate::office::pptx::scene::Item::Image(
                        crate::office::pptx::scene::ImageItem {
                            image: crate::office::pptx::scene::ImageRef {
                                part: target,
                                effects: Vec::new(),
                            },
                            transform: tiny_skia::Transform::from_row(
                                cw2 as f32, 0.0, 0.0, ch2 as f32, cx as f32, cy as f32,
                            ),
                            crop: (0.0, 0.0, 0.0, 0.0),
                            alpha: 1.0,
                            clip: None,
                            shape: None,
                            alt_text: c.attr("alt").map(str::to_string),
                        },
                    ));
                }
                continue;
            }
            let Some(child) = self.vml_shape(c, cw2, ch2, part) else {
                continue;
            };
            let t = tiny_skia::Transform::from_translate(cx as f32, cy as f32);
            for it in &child.items {
                let mut it = it.clone();
                crate::office::docx::layout::transform_item(&mut it, t);
                spec.items.push(it);
            }
            for tb in &child.texts {
                let mut tb = tb.clone();
                tb.rect.x += cx;
                tb.rect.y += cy;
                spec.texts.push(tb);
            }
        }
        (!spec.items.is_empty() || !spec.texts.is_empty()).then(|| Rc::new(spec))
    }

    /// Legacy VML pictures (`w:pict`, `w:object`): an image shape with a CSS
    /// style giving its size; absolutely positioned shapes float.
    fn vml(&self, c: &XNode, part: &str, style: &Rc<RunStyle>) -> Option<Inline> {
        let shape = c.children.iter().find(|n| {
            matches!(
                n.name.as_str(),
                "v:shape" | "v:rect" | "v:roundrect" | "v:oval" | "v:line" | "v:group"
            )
        })?;
        let mut css = parse_css(shape.attr("style").unwrap_or(""));
        // A line spans `from`..`to` (points from the anchor); its box starts at the nearer end.
        if shape.name == "v:line" {
            let pt = |k: &str, d: &str| -> (f64, f64) {
                let v = shape.attr(k).unwrap_or(d);
                let mut it = v.split(',').map(|x| css_len(x).unwrap_or(0.0));
                (it.next().unwrap_or(0.0), it.next().unwrap_or(0.0))
            };
            let (a, b) = (pt("from", "0,0"), pt("to", "10,10"));
            css.insert("margin-left".into(), format!("{}pt", a.0.min(b.0)));
            css.insert("margin-top".into(), format!("{}pt", a.1.min(b.1)));
            css.insert("width".into(), format!("{}pt", (b.0 - a.0).abs().max(0.01)));
            css.insert(
                "height".into(),
                format!("{}pt", (b.1 - a.1).abs().max(0.01)),
            );
            css.entry("position".into())
                .or_insert_with(|| "absolute".into());
            let mut flip = String::new();
            if (b.0 < a.0) != (b.1 < a.1) {
                flip.push('y');
            }
            css.insert("flip".into(), flip);
        }
        let w = css.get("width").and_then(|v| css_len(v)).unwrap_or(0.0);
        let mut h = css.get("height").and_then(|v| css_len(v)).unwrap_or(0.0);
        // A horizontal rule (`o:hr`) is drawn as a thin line.
        if shape
            .attr("o:hr")
            .and_then(crate::office::pptx::xml::parse_bool)
            == Some(true)
            && h <= 0.0
        {
            h = 1.5;
        }
        // A group's pictures are its children's; the group itself has none.
        let target = shape
            .child("v:imagedata")
            .and_then(|i| i.attr("r:id").or_else(|| i.attr("r:pict")))
            .and_then(|id| self.rel_target(part, id));
        let picture = Picture {
            part: target,
            w,
            h,
            crop: (0.0, 0.0, 0.0, 0.0),
            alt: shape.attr("alt").map(str::to_string),
            shape: self.vml_shape(shape, w, h, part),
            effects: shape
                .child("v:imagedata")
                .map(|i| {
                    let on = |k: &str| {
                        i.attr(k).and_then(crate::office::pptx::xml::parse_bool) == Some(true)
                    };
                    let mut e = Vec::new();
                    if on("grayscale") {
                        e.push(crate::office::pptx::scene::BlipEffect::Grayscale);
                    }
                    if on("bilevel") {
                        e.push(crate::office::pptx::scene::BlipEffect::BiLevel(0.5));
                    }
                    e
                })
                .unwrap_or_default(),
            alpha: 1.0,
            graphic: None,
            shadow: None,
        };
        // `flip` mirrors the shape's outline about its centre.
        let picture = match css.get("flip").filter(|f| !f.is_empty()) {
            Some(f) => {
                let mut picture = picture;
                if let Some(spec) = picture.shape.take() {
                    let mut spec = (*spec).clone();
                    let (sx, sy) = (
                        if f.contains('x') { -1.0 } else { 1.0 },
                        if f.contains('y') { -1.0 } else { 1.0 },
                    );
                    let t =
                        tiny_skia::Transform::from_translate((w / 2.0) as f32, (h / 2.0) as f32)
                            .pre_scale(sx, sy)
                            .pre_translate(-(w / 2.0) as f32, -(h / 2.0) as f32);
                    for it in spec.items.iter_mut() {
                        crate::office::docx::layout::transform_item(it, t);
                    }
                    picture.shape = Some(Rc::new(spec));
                }
                picture
            }
            None => picture,
        };
        // `rotation` (degrees) turns the shape and its text about the centre.
        let picture = match css
            .get("rotation")
            .and_then(|r| r.trim().parse::<f64>().ok())
        {
            Some(rot) if rot.rem_euclid(360.0) != 0.0 => {
                let mut picture = picture;
                if let Some(spec) = picture.shape.take() {
                    let mut spec = (*spec).clone();
                    let (cx, cy) = ((w / 2.0) as f32, (h / 2.0) as f32);
                    let t = tiny_skia::Transform::from_translate(cx, cy)
                        .pre_rotate(rot as f32)
                        .pre_translate(-cx, -cy);
                    for it in spec.items.iter_mut() {
                        crate::office::docx::layout::transform_item(it, t);
                    }
                    for tb in spec.texts.iter_mut() {
                        tb.transform = Some(tb.transform.map_or(t, |o| o.post_concat(t)));
                    }
                    picture.shape = Some(Rc::new(spec));
                }
                picture
            }
            _ => picture,
        };
        if css.get("position").map(String::as_str) == Some("absolute") {
            let off = |k: &str| css.get(k).and_then(|v| css_len(v)).unwrap_or(0.0);
            // `mso-position-horizontal`/`-vertical` align the shape instead of its offsets.
            let align = |k: &str, off: f64| -> Result<f64, String> {
                match css.get(k).map(String::as_str) {
                    Some(a) if a != "absolute" => Err(a.to_string()),
                    _ => Ok(off),
                }
            };
            let h_rel = match css
                .get("mso-position-horizontal-relative")
                .map(String::as_str)
            {
                Some("page") => HRel::Page,
                Some("margin") => HRel::Margin,
                Some("char") => HRel::Character,
                _ => HRel::Column,
            };
            let v_rel = match css
                .get("mso-position-vertical-relative")
                .map(String::as_str)
            {
                Some("page") => VRel::Page,
                Some("margin") => VRel::Margin,
                Some("line") => VRel::Line,
                _ => VRel::Paragraph,
            };
            let behind = css.get("z-index").is_some_and(|z| z.starts_with('-'));
            return Some(Inline::Anchor(Box::new(Anchored {
                pic: picture,
                h_rel,
                x: align("mso-position-horizontal", off("margin-left") + off("left")),
                v_rel,
                y: align("mso-position-vertical", off("margin-top") + off("top")),
                behind,
                z: css
                    .get("z-index")
                    .and_then(|z| z.trim().parse::<i64>().ok())
                    .map_or(0, i64::abs),
                wrap: match shape.find("x:wrap").and_then(|w| w.attr("type")) {
                    Some("topAndBottom") => Wrap::TopAndBottom,
                    Some("square") | Some("tight") | Some("through") => Wrap::Square,
                    _ => Wrap::None,
                },
                dist: [
                    off("mso-wrap-distance-top"),
                    off("mso-wrap-distance-bottom"),
                    css.get("mso-wrap-distance-left")
                        .and_then(|v| css_len(v))
                        .unwrap_or(9.0),
                    css.get("mso-wrap-distance-right")
                        .and_then(|v| css_len(v))
                        .unwrap_or(9.0),
                ],
                contour: None,
                wrap_side: "bothSides".into(),
                text_items: Vec::new(),
                frame_children: Vec::new(),
                text_frame: false,
                turned: css
                    .get("rotation")
                    .and_then(|r| r.trim().parse::<f64>().ok())
                    .is_some_and(quarter_turn),
            })));
        }
        if w <= 0.0 && h <= 0.0 {
            return None;
        }
        Some(Inline::Picture(picture, style.clone()))
    }

    pub fn sect_pr(&self, s: &XNode, part: &str) -> SectPr {
        let mut p = SectPr::default();
        if let Some(columns) = s.child("w15:footnoteColumns") {
            p.note_columns = match columns.attr("val").and_then(|v| v.parse::<u32>().ok()) {
                Some(0) => FootnoteColumns::Auto,
                Some(n) => FootnoteColumns::Count(n),
                None => FootnoteColumns::Invalid,
            };
        }
        if let Some(f) = s.child("w:footnotePr") {
            (p.note_start, p.note_fmt) = note_pr(f);
            p.note_position = match f.child("w:pos").and_then(|n| n.attr("val")) {
                Some("beneathText" | "sectEnd") => FootnotePosition::BeneathText,
                Some("docEnd") => FootnotePosition::DocumentEnd,
                _ => FootnotePosition::PageBottom,
            };
        }
        if let Some(f) = s.child("w:endnotePr") {
            (p.endnote_start, p.endnote_fmt) = note_pr(f);
            p.endnote_restart = f
                .child("w:numRestart")
                .and_then(|n| n.attr("val"))
                .map(str::to_string);
        }
        if let Some(sz) = s.child("w:pgSz") {
            p.page_w = tw_attr(sz, "w").unwrap_or(p.page_w).max(36.0);
            p.page_h = tw_attr(sz, "h").unwrap_or(p.page_h).max(36.0);
        }
        if let Some(m) = s.child("w:pgMar") {
            p.top = tw_attr(m, "top").unwrap_or(p.top);
            p.bottom = tw_attr(m, "bottom").unwrap_or(p.bottom);
            p.left = tw_attr(m, "left")
                .or_else(|| tw_attr(m, "start"))
                .unwrap_or(p.left);
            p.right = tw_attr(m, "right")
                .or_else(|| tw_attr(m, "end"))
                .unwrap_or(p.right);
            // An empty header/footer distance (w:header="") is 0 in Word, not unset.
            let empty = |k: &str| m.attr(k).is_some_and(|v| v.trim().is_empty());
            p.header = tw_attr(m, "header").unwrap_or(if empty("header") { 0.0 } else { p.header });
            p.footer = tw_attr(m, "footer").unwrap_or(if empty("footer") { 0.0 } else { p.footer });
            p.gutter = tw_attr(m, "gutter").unwrap_or(0.0).max(0.0);
        }
        if let Some(c) = s.child("w:cols") {
            p.cols = c.attr_i64("num").unwrap_or(1).clamp(1, 45) as usize;
            p.col_space = tw_attr(c, "space").unwrap_or(36.0);
            let equal = c.attr_bool("equalWidth").unwrap_or(true);
            let explicit: Vec<(f64, f64)> = c
                .children_named("w:col")
                .map(|col| {
                    (
                        tw_attr(col, "w").unwrap_or(0.0),
                        tw_attr(col, "space").unwrap_or(0.0),
                    )
                })
                .collect();
            // Unequal columns keep their own widths, a single one too (Word for Mac:
            // one 468 pt column on a 510 pt text width wraps at 468 pt).
            if !equal && !explicit.is_empty() && explicit.iter().all(|c| c.0 > 0.0) {
                p.cols = explicit.len();
                p.col_widths = explicit;
            }
        }
        p.start = match s.child("w:type").and_then(|t| t.attr("val")) {
            Some("continuous") => SectionStart::Continuous,
            Some("oddPage") => SectionStart::OddPage,
            Some("evenPage") => SectionStart::EvenPage,
            Some("nextColumn") => SectionStart::NextColumn,
            _ => SectionStart::NextPage,
        };
        p.title_pg = s
            .child("w:titlePg")
            .is_some_and(|t| crate::office::docx::props::on_off(t));
        for (name, refs) in [
            ("w:headerReference", &mut p.headers),
            ("w:footerReference", &mut p.footers),
        ] {
            for r in s.children_named(name) {
                let Some(target) = r.attr("r:id").and_then(|id| self.rel_target(part, id)) else {
                    continue;
                };
                match r.attr("type").unwrap_or("default") {
                    "first" => refs.first = Some(target),
                    "even" => refs.even = Some(target),
                    _ => refs.default = Some(target),
                }
            }
        }
        if let Some(n) = s.child("w:pgNumType") {
            p.pg_start = n.attr_i64("start");
            p.pg_fmt = n.attr("fmt").map(str::to_string);
        }
        if let Some(g) = s.child("w:docGrid") {
            p.grid_type = g.attr("type").map(str::to_string);
            p.grid_pitch = tw_attr(g, "linePitch");
            p.grid_char_space = g.attr("charSpace").and_then(character_grid_space);
        }
        p.line_numbers = s
            .child("w:lnNumType")
            .filter(|n| n.attr_i64("countBy").unwrap_or(0) > 0)
            .map(|n| {
                let rpr = RPr {
                    r_style: Some("LineNumber".into()),
                    ..Default::default()
                };
                crate::office::docx::decor::LineNumbers {
                    count_by: n.attr_i64("countBy").unwrap_or(1).max(1),
                    start: n.attr_i64("start").unwrap_or(0).max(0),
                    distance: tw_attr(n, "distance").filter(|d| *d > 0.0),
                    restart: n.attr("restart").unwrap_or("newPage").to_string(),
                    style: Rc::new(
                        self.sheet
                            .run_style(&self.sheet.resolve_rpr(None, &rpr, None, None)),
                    ),
                }
            });
        p.pg_borders = s
            .child("w:pgBorders")
            .map(|b| crate::office::docx::decor::PageBorders {
                borders: self.sheet.theme_colors.borders(&Borders::parse(b)),
                from_page: b.attr("offsetFrom") == Some("page"),
                display: b.attr("display").unwrap_or("allPages").to_string(),
            });
        p.v_align = s
            .child("w:vAlign")
            .and_then(|v| v.attr("val"))
            .map(str::to_string);
        p
    }

    pub fn table(&self, t: &XNode, part: &str) -> Option<Table> {
        let source_table = self.source_table_id(t, part);
        let view = self.materialized_table(t, part);
        if let Some(view) = &view {
            self.retain_import_tree(view.clone());
        }
        let t = view.as_deref().unwrap_or(t);
        let merged = h_merged(t);
        if let Some(m) = &merged {
            self.retain_import_tree(m.clone());
        }
        let t = merged.as_deref().unwrap_or(t);
        let mut row_nodes = Vec::new();
        collect_named(&t.children, "w:tr", &mut row_nodes);
        let row_grids = self.grid_budget.borrow_mut().table(t, &row_nodes)?;
        let source_text_only = source::text_only_table(t);
        let tblpr = t.child("w:tblPr");
        let style_id = tblpr
            .and_then(|p| p.child("w:tblStyle"))
            .and_then(|s| s.attr("val"))
            .filter(|s| self.sheet.styles.contains_key(*s))
            .map(str::to_string)
            .or_else(|| self.sheet.default_table.clone());
        let chain: Vec<&crate::office::docx::styles::Style> = style_id
            .as_deref()
            .map(|s| self.sheet.chain(s))
            .unwrap_or_default();
        // Table properties: the style chain, then the table's own.
        let mut pr_nodes: Vec<&XNode> = chain.iter().filter_map(|s| s.tblpr.as_deref()).collect();
        if let Some(p) = tblpr {
            pr_nodes.push(p);
        }
        let mut borders = Borders::default();
        let mut margins = [0.0, 5.4, 0.0, 5.4];
        let mut jc = String::from("left");
        let mut indent = 0.0;
        // Word reads the layout algorithm from direct table properties.
        let layout = match tblpr
            .and_then(|p| p.child("w:tblLayout"))
            .and_then(|p| p.attr("type"))
        {
            Some("fixed") => TableLayout::Fixed,
            _ => TableLayout::AutoFit,
        };
        let mut cell_spacing = 0.0;
        let mut bidi_visual = false;
        for n in &pr_nodes {
            if let Some(b) = n.child("w:tblBorders") {
                borders.apply(&Borders::parse(b));
            }
            if let Some(m) = n.child("w:tblCellMar") {
                apply_margins(&mut margins, m);
            }
            if let Some(j) = n.child("w:jc").and_then(|j| j.attr("val")) {
                jc = j.to_string();
            }
            if let Some(c) = n.child("w:tblCellSpacing") {
                cell_spacing = tw_attr(c, "w").unwrap_or(0.0).max(0.0);
            }
            if let Some(b) = n.child("w:bidiVisual") {
                bidi_visual = crate::office::docx::props::on_off(b);
            }
            if let Some(i) = n.child("w:tblInd") {
                indent = tw_attr(i, "w").unwrap_or(indent);
            }
        }
        let source_vertical_merges = source::vertical_merges(&row_nodes, &row_grids);
        // Word selects a first-row width exception for the whole table.
        // Table-style widths and later-row width exceptions do not participate.
        let width_node = row_nodes
            .first()
            .and_then(|row| row.child("w:tblPrEx"))
            .and_then(|pr| pr.child("w:tblW"))
            .or_else(|| tblpr.and_then(|pr| pr.child("w:tblW")));
        let preferred_width = parse_preferred_width(width_node);
        let mut text = TableText::default();
        for s in &chain {
            text.ppr.apply(&s.ppr);
            text.rpr.apply(&s.rpr);
        }
        let look = tblpr.and_then(|p| p.child("w:tblLook"));
        let flag = |name: &str, bit: u32, default: bool| {
            look.and_then(|l| l.attr_bool(name))
                .or_else(|| {
                    look.and_then(|l| l.attr("val"))
                        .and_then(|v| u32::from_str_radix(v, 16).ok())
                        .map(|v| v & bit != 0)
                })
                .unwrap_or(default)
        };
        let first_row = flag("firstRow", 0x0020, true);
        let cond = |kind: &str| -> Option<crate::office::docx::styles::CondFormat> {
            let mut out: Option<crate::office::docx::styles::CondFormat> = None;
            for s in &chain {
                if let Some(c) = s.conditional.get(kind) {
                    let o = out.get_or_insert_with(Default::default);
                    o.ppr.apply(&c.ppr);
                    o.rpr.apply(&c.rpr);
                    if c.tcpr.is_some() {
                        o.tcpr = c.tcpr.clone();
                    }
                }
            }
            out
        };
        let last_row = flag("lastRow", 0x0040, false);
        let first_col = flag("firstColumn", 0x0080, true);
        let last_col = flag("lastColumn", 0x0100, false);
        let h_band = !flag("noHBand", 0x0200, false);
        let v_band = !flag("noVBand", 0x0400, true);
        let band_size = |name: &str| {
            pr_nodes
                .iter()
                .rev()
                .find_map(|n| n.child(name).and_then(|b| b.attr_i64("val")))
                .unwrap_or(1)
                .max(1) as usize
        };
        let (row_band, col_band) = (
            band_size("w:tblStyleRowBandSize"),
            band_size("w:tblStyleColBandSize"),
        );
        let kinds = [
            "band1Vert",
            "band2Vert",
            "band1Horz",
            "band2Horz",
            "firstCol",
            "lastCol",
            "firstRow",
            "lastRow",
            "nwCell",
            "neCell",
            "swCell",
            "seCell",
        ];
        let conds: HashMap<&str, crate::office::docx::styles::CondFormat> = kinds
            .iter()
            .filter_map(|k| cond(k).map(|c| (*k, c)))
            .collect();
        let grid: Vec<f64> = t
            .child("w:tblGrid")
            .map(|g| {
                g.children_named("w:gridCol")
                    .map(|c| tw_attr(c, "w").unwrap_or(0.0).max(0.0))
                    .collect()
            })
            .unwrap_or_default();
        let mut rows = Vec::new();
        let style_tcpr: Option<&XNode> = chain.iter().rev().find_map(|s| s.tcpr.as_deref());
        let n_rows = row_nodes.len();
        let n_cols = grid.len();
        for (ri, tr) in row_nodes.iter().enumerate() {
            let trpr = tr.child("w:trPr");
            let mut height = 0.0;
            let mut height_rule = HeightRule::Auto;
            if let Some(h) = trpr.and_then(|p| p.child("w:trHeight")) {
                height = tw_attr(h, "val").unwrap_or(0.0);
                height_rule = match h.attr("hRule") {
                    Some("exact") => HeightRule::Exact,
                    Some("auto") => HeightRule::Auto,
                    _ => HeightRule::AtLeast,
                };
            }
            let tr_flag = |name: &str| {
                trpr.and_then(|p| p.child(name))
                    .is_some_and(|c| crate::office::docx::props::on_off(c))
            };
            let grid_before = row_grids[ri].before;
            let grid_after = row_grids[ri].after;
            // `w:tblPrEx`: table borders and cell margins overridden for this row.
            let prex = tr.child("w:tblPrEx");
            let mut row_margins = margins;
            if let Some(m) = prex.and_then(|p| p.child("w:tblCellMar")) {
                apply_margins(&mut row_margins, m);
            }
            let row_borders = prex
                .and_then(|p| p.child("w:tblBorders"))
                .map(|b| self.sheet.theme_colors.borders(&Borders::parse(b)));
            let mut cell_nodes = Vec::new();
            collect_named(&tr.children, "w:tc", &mut cell_nodes);
            let mut cells = Vec::new();
            let mut active_regions = [false; 12];
            let n_cells = cell_nodes.len();
            let cell_spans = &row_grids[ri].spans;
            let mut col = grid_before;
            for (ci, tc) in cell_nodes.iter().enumerate() {
                let tcpr = tc.child("w:tcPr");
                let span = cell_spans[ci];
                // Table style regions of this cell, lowest priority first.
                let mut regions: Vec<&crate::office::docx::styles::CondFormat> = Vec::new();
                {
                    let is_first_row = first_row && ri == 0;
                    let is_last_row = last_row && ri + 1 == n_rows && n_rows > 1;
                    let is_first_col = first_col && col == 0;
                    let is_last_col = last_col
                        && (col + span >= n_cols || ci + 1 == n_cells)
                        && !(col == 0 && first_col);
                    let mut push = |k: &str| {
                        if let Some(index) = kinds.iter().position(|kind| *kind == k) {
                            active_regions[index] = true;
                        }
                        if let Some(c) = conds.get(k) {
                            regions.push(c);
                        }
                    };
                    if v_band && !is_first_col && !is_last_col {
                        let c = col.saturating_sub(first_col as usize) / col_band;
                        push(if c % 2 == 0 { "band1Vert" } else { "band2Vert" });
                    }
                    if h_band && !is_first_row && !is_last_row {
                        let r = ri.saturating_sub(first_row as usize) / row_band;
                        push(if r % 2 == 0 { "band1Horz" } else { "band2Horz" });
                    }
                    if is_first_col {
                        push("firstCol");
                    }
                    if is_last_col {
                        push("lastCol");
                    }
                    if is_first_row {
                        push("firstRow");
                    }
                    if is_last_row {
                        push("lastRow");
                    }
                    match (is_first_row, is_last_row, is_first_col, is_last_col) {
                        (true, _, true, _) => push("nwCell"),
                        (true, _, _, true) => push("neCell"),
                        (_, true, true, _) => push("swCell"),
                        (_, true, _, true) => push("seCell"),
                        _ => {}
                    }
                }
                col += span;
                let vmerge = match tcpr.and_then(|p| p.child("w:vMerge")) {
                    Some(v) if v.attr("val") == Some("restart") => VMerge::Restart,
                    Some(_) => VMerge::Continue,
                    None => VMerge::None,
                };
                let mut cell_borders = Borders::default();
                let mut diagonals: [Option<crate::office::docx::props::Border>; 2] = [None, None];
                let mut cell_margins = row_margins;
                let mut shading = None;
                let mut v_align = String::from("top");
                let mut text_dir: Option<String> = None;
                let mut hide_mark = false;
                let mut tc_nodes: Vec<&XNode> = style_tcpr.into_iter().collect();
                tc_nodes.extend(regions.iter().filter_map(|c| c.tcpr.as_deref()));
                if let Some(p) = tcpr {
                    tc_nodes.push(p);
                }
                for n in tc_nodes {
                    if let Some(b) = n.child("w:tcBorders") {
                        cell_borders.apply(&Borders::parse(b));
                        for (k, name) in ["w:tl2br", "w:tr2bl"].iter().enumerate() {
                            if let Some(d) = b
                                .child(name)
                                .and_then(|d| crate::office::docx::props::Border::parse(d))
                            {
                                diagonals[k] = Some(d).filter(|d| d.visible());
                            }
                        }
                    }
                    if let Some(m) = n.child("w:tcMar") {
                        apply_margins(&mut cell_margins, m);
                    }
                    if let Some(s) = n
                        .child("w:shd")
                        .and_then(|s| crate::office::docx::props::Shading::parse(s))
                    {
                        shading = self.sheet.theme_colors.shading(&s);
                    }
                    if let Some(v) = n.child("w:vAlign").and_then(|v| v.attr("val")) {
                        v_align = v.to_string();
                    }
                    if let Some(h) = n.child("w:hideMark") {
                        hide_mark = !matches!(h.attr("val"), Some("0") | Some("false"));
                    }
                    if let Some(d) = n.child("w:textDirection").and_then(|v| v.attr("val")) {
                        text_dir = Some(d.to_string()).filter(|d| {
                            matches!(
                                d.as_str(),
                                "btLr" | "tbRl" | "tbRlV" | "tbLrV" | "rl" | "lr"
                            )
                        });
                    }
                }
                let mut cell_text = text.clone();
                for c in &regions {
                    cell_text.ppr.apply(&c.ppr);
                    cell_text.rpr.apply(&c.rpr);
                }
                self.source
                    .borrow_mut()
                    .cells
                    .push(crate::office::pptx::scene::SourceCell {
                        table: source_table,
                        row: ri,
                        cell: ci,
                        span,
                        cell_count: n_cells,
                        cell_spans: cell_spans.clone(),
                        grid_before,
                        grid_after,
                        vertical_merges: source_vertical_merges[ri].clone(),
                        fragment: None,
                        text_only: source_text_only,
                    });
                let blocks = self.blocks(&tc.children, part, Some(&cell_text));
                self.source.borrow_mut().cells.pop();
                cells.push(Cell {
                    preferred_width: parse_preferred_width(tcpr.and_then(|p| p.child("w:tcW"))),
                    no_wrap: false,
                    diagonals: diagonals.map(|b| b.map(|b| self.sheet.theme_colors.border(&b))),
                    blocks,
                    span,
                    vmerge,
                    borders: self.sheet.theme_colors.borders(&cell_borders),
                    margins: cell_margins,
                    shading,
                    v_align,
                    text_dir,
                    hide_mark,
                });
            }
            // Word applies conditional noWrap to the entire affected row.
            // Direct cell values remain the final override, including false.
            let base_no_wrap = chain
                .iter()
                .filter_map(|style| {
                    style
                        .tcpr
                        .as_deref()?
                        .child("w:noWrap")
                        .map(|value| crate::office::docx::props::on_off(value))
                })
                .next_back()
                .unwrap_or(false);
            let row_no_wrap = kinds
                .iter()
                .enumerate()
                .filter(|(index, _)| active_regions[*index])
                .flat_map(|(_, kind)| {
                    chain.iter().filter_map(|style| {
                        style
                            .conditional
                            .get(*kind)?
                            .tcpr
                            .as_deref()?
                            .child("w:noWrap")
                            .map(|value| crate::office::docx::props::on_off(value))
                    })
                })
                .next_back();
            for (cell, source) in cells.iter_mut().zip(&cell_nodes) {
                cell.no_wrap = source
                    .child("w:tcPr")
                    .and_then(|p| p.child("w:noWrap"))
                    .map(|value| crate::office::docx::props::on_off(value))
                    .or(row_no_wrap)
                    .unwrap_or(base_no_wrap);
            }
            cells.shrink_to_fit();
            let row_layout = match prex
                .and_then(|p| p.child("w:tblLayout"))
                .and_then(|p| p.attr("type"))
            {
                Some("fixed") => TableLayout::Fixed,
                Some("autofit") => TableLayout::AutoFit,
                _ => layout,
            };
            rows.push(Row {
                layout: row_layout,
                cells,
                height,
                height_rule,
                cant_split: tr_flag("w:cantSplit"),
                header: tr_flag("w:tblHeader"),
                grid_before,
                borders: row_borders,
            });
        }
        // Floating table position; the anchors default to the column
        // horizontally and the margin vertically (ECMA-376 17.4.58).
        let float = tblpr.and_then(|p| p.child("w:tblpPr")).map(|f| TableFloat {
            h_anchor: f.attr("horzAnchor").unwrap_or("text").to_string(),
            x: f.attr("tblpXSpec")
                .map(|s| Err(s.to_string()))
                .unwrap_or_else(|| Ok(tw_attr(f, "tblpX").unwrap_or(0.0))),
            v_anchor: f.attr("vertAnchor").unwrap_or("margin").to_string(),
            y: f.attr("tblpYSpec")
                // Text anchors use the numeric offset; relative vertical alignment
                // is ignored for this anchor (ECMA-376, tblpPr/tblpYSpec).
                .filter(|_| f.attr("vertAnchor") != Some("text"))
                .map(|s| Err(s.to_string()))
                .unwrap_or_else(|| Ok(tw_attr(f, "tblpY").unwrap_or(0.0))),
            no_overlap: tblpr
                .and_then(|p| p.child("w:tblOverlap"))
                .and_then(|o| o.attr("val"))
                == Some("never"),
        });
        Some(Table {
            bookmarks_before: Vec::new(),
            bookmarks_after: Vec::new(),
            layout,
            grid,
            rows,
            jc,
            indent,
            borders: self.sheet.theme_colors.borders(&borders),
            float,
            preferred_width,
            cell_spacing,
            bidi_visual,
        })
    }
}

struct RunCtx<'c> {
    part: &'c str,
    para_style: Option<&'c str>,
    table: Option<&'c TableText>,
}

/// Section properties of a body in document order: those ending paragraphs, then the body's own.
fn collect_sect_prs(nodes: &[Rc<XNode>], out: &mut Vec<Rc<XNode>>) {
    for n in nodes {
        match n.name.as_str() {
            "w:p" => {
                if let Some(sp) = n.child("w:pPr").and_then(|p| p.child("w:sectPr")) {
                    out.push(sp.clone());
                }
            }
            "w:sdt" => {
                if let Some(c) = n.child("w:sdtContent") {
                    collect_sect_prs(&c.children, out);
                }
            }
            "w:customXml" | "w:ins" | "w:moveTo" => collect_sect_prs(&n.children, out),
            "w:sectPr" => out.push(n.clone()),
            _ => {}
        }
    }
}

/// A content hash of an element (name, attributes, children): equal run
/// properties share a cached style. Parts are dropped after parsing, so a node's
/// address can be reused by another part's node and cannot be the key.
fn node_hash(n: &XNode) -> u64 {
    use std::hash::{Hash, Hasher};
    fn feed(n: &XNode, h: &mut std::collections::hash_map::DefaultHasher) {
        n.name.hash(h);
        n.attrs.hash(h);
        n.text.hash(h);
        n.children.len().hash(h);
        for c in &n.children {
            feed(c, h);
        }
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    feed(n, &mut h);
    h.finish()
}

/// Rows and cells may sit inside `w:sdt`/`w:customXml` wrappers.
/// The table with legacy horizontal merges (`w:hMerge`) folded into grid spans: a
/// restart cell takes the spans of the continuation cells after it, which are
/// dropped. `None` when no row uses them.
fn h_merged(t: &XNode) -> Option<Rc<XNode>> {
    let merge = |c: &XNode| {
        c.child("w:tcPr")
            .and_then(|p| p.child("w:hMerge"))
            .map(|h| h.attr("val") == Some("restart"))
    };
    let uses = t.children.iter().filter(|r| r.name == "w:tr").any(|r| {
        r.children
            .iter()
            .any(|c| c.name == "w:tc" && merge(c).is_some())
    });
    if !uses {
        return None;
    }
    let pr_child = |c: &XNode, name: &str| c.child("w:tcPr").and_then(|p| p.child(name)).cloned();
    let span_of = |c: &XNode| {
        pr_child(c, "w:gridSpan")
            .and_then(|g| g.attr("val").and_then(|v| v.parse::<usize>().ok()))
            .unwrap_or(1)
    };
    // Preferred width in twips when given as dxa.
    let width_of = |c: &XNode| {
        pr_child(c, "w:tcW")
            .filter(|w| w.attr("type").is_none_or(|t| t == "dxa"))
            .and_then(|w| w.attr("w").and_then(|v| v.parse::<i64>().ok()))
    };
    let leaf = |name: &str, attrs: Vec<(String, String)>| {
        Rc::new(XNode {
            name: name.into(),
            attrs,
            children: Vec::new(),
            text: String::new(),
        })
    };
    // (cell index in `out`, total span, total dxa width when every member has one)
    type Open = (usize, usize, Option<i64>);
    let close = |out: &mut Vec<Rc<XNode>>, (at, span, width): Open| {
        let mut cell = (*out[at]).clone();
        let mut pr = cell
            .child("w:tcPr")
            .map(|n| (**n).clone())
            .unwrap_or(XNode {
                name: "w:tcPr".into(),
                attrs: Vec::new(),
                children: Vec::new(),
                text: String::new(),
            });
        pr.children
            .retain(|n| !matches!(n.name.as_str(), "w:gridSpan" | "w:hMerge" | "w:tcW"));
        pr.children
            .push(leaf("w:gridSpan", vec![("val".into(), span.to_string())]));
        if let Some(w) = width {
            pr.children.push(leaf(
                "w:tcW",
                vec![("w".into(), w.to_string()), ("type".into(), "dxa".into())],
            ));
        }
        cell.children.retain(|n| n.name != "w:tcPr");
        cell.children.insert(0, Rc::new(pr));
        out[at] = Rc::new(cell);
    };
    let mut table = t.clone();
    for row in table.children.iter_mut().filter(|r| r.name == "w:tr") {
        let mut out: Vec<Rc<XNode>> = Vec::new();
        let mut open: Option<Open> = None;
        for c in &row.children {
            match (c.name == "w:tc", merge(c), open) {
                (true, Some(false), Some((at, span, width))) => {
                    let w = width.zip(width_of(c)).map(|(a, b)| a + b);
                    open = Some((at, span + span_of(c), w));
                }
                (true, m, _) => {
                    if let Some(o) = open.take() {
                        close(&mut out, o);
                    }
                    if m == Some(true) {
                        open = Some((out.len(), span_of(c), width_of(c)));
                    }
                    out.push(c.clone());
                }
                _ => out.push(c.clone()),
            }
        }
        if let Some(o) = open {
            close(&mut out, o);
        }
        let mut r = (**row).clone();
        r.children = out;
        *row = Rc::new(r);
    }
    Some(Rc::new(table))
}

fn collect_named<'n>(nodes: &'n [Rc<XNode>], name: &str, out: &mut Vec<&'n Rc<XNode>>) {
    for n in nodes {
        if n.name == name {
            out.push(n);
        } else if n.name == "w:sdt" {
            if let Some(c) = n.child("w:sdtContent") {
                collect_named(&c.children, name, out);
            }
        } else if n.name == "w:customXml" {
            collect_named(&n.children, name, out);
        }
    }
}

/// `w:footnotePr`: first number and number format.
pub fn note_pr(f: &XNode) -> (Option<i64>, Option<String>) {
    (
        f.child("w:numStart").and_then(|n| n.attr_i64("val")),
        f.child("w:numFmt")
            .and_then(|n| n.attr("val"))
            .map(str::to_string),
    )
}

fn apply_margins(m: &mut [f64; 4], n: &XNode) {
    let get = |names: &[&str]| {
        names
            .iter()
            .find_map(|k| n.child(&format!("w:{k}")))
            .filter(|c| {
                // Word ignores nil top/bottom cell exceptions, preserving inheritance.
                !(n.name == "w:tcMar"
                    && matches!(c.name.as_str(), "w:top" | "w:bottom")
                    && c.attr("type") == Some("nil"))
            })
            .and_then(|c| tw_attr(c, "w"))
    };
    for (i, names) in [
        &["top"][..],
        &["left", "start"][..],
        &["bottom"][..],
        &["right", "end"][..],
    ]
    .iter()
    .enumerate()
    {
        if let Some(v) = get(names) {
            m[i] = v.max(0.0);
        }
    }
}

fn push_text(out: &mut Vec<Inline>, text: &str, style: &Rc<RunStyle>) {
    if text.is_empty() {
        return;
    }
    if let Some(Inline::Text(prev, s)) = out.last_mut()
        && Rc::ptr_eq(s, style)
    {
        prev.push_str(text);
        return;
    }
    out.push(Inline::Text(text.to_string(), style.clone()));
}

/// A header or footer STYLEREF field: the style it names, whether `\\n` asks for the
/// paragraph's number instead of its text, and its saved result.
#[derive(Debug, Clone)]
pub struct StyleRef {
    pub name: String,
    pub number: bool,
    pub saved: String,
}

/// The style a `STYLEREF "name" [switches]` field names, and whether it has `\\n`.
fn style_ref_target(instr: &str) -> Option<(String, bool)> {
    let rest = instr.trim_start();
    let word = rest.split_whitespace().next()?;
    if !word.eq_ignore_ascii_case("STYLEREF") {
        return None;
    }
    let rest = rest[word.len()..].trim_start();
    let (name, switches) = if let Some(quoted) = rest.strip_prefix('"') {
        let end = quoted.find('"')?;
        (&quoted[..end], &quoted[end + 1..])
    } else {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        (&rest[..end], &rest[end..])
    };
    let number = switches
        .split_whitespace()
        .any(|w| w == "\\n" || w == "\\r" || w == "\\w");
    (!name.is_empty()).then(|| (name.to_string(), number))
}

pub fn field_kind(instr: &str) -> Option<FieldKind> {
    let mut words = instr.split_whitespace();
    let word = words.next()?.to_ascii_uppercase();
    // `\* Format`: the number format, case-sensitive for letters and roman numerals.
    let mut fmt = None;
    while let Some(w) = words.next() {
        if w == "\\*" {
            fmt = words.next().and_then(field_format).or(fmt);
        }
    }
    match word.as_str() {
        "PAGE" => Some(FieldKind::Page(fmt)),
        "NUMPAGES" => Some(FieldKind::NumPages(fmt)),
        "SECTIONPAGES" => Some(FieldKind::SectionPages(fmt)),
        _ => None,
    }
}

/// A field's `\\*` number format as a `format_number` name.
fn field_format(sw: &str) -> Option<&'static str> {
    Some(match sw {
        "roman" => "lowerRoman",
        "Roman" | "ROMAN" => "upperRoman",
        "alphabetic" => "lowerLetter",
        "Alphabetic" | "ALPHABETIC" => "upperLetter",
        "ArabicDash" => "numberInDash",
        "Ordinal" => "ordinal",
        "CardText" | "cardtext" => "cardinalTextLower",
        "OrdText" | "ordtext" => "ordinalTextLower",
        _ if sw.eq_ignore_ascii_case("arabic") => "decimal",
        _ => return None,
    })
}

/// Sections without a header or footer reference of a kind inherit the
/// previous section's.
fn inherit_hdr_ftr(sections: &mut [Section]) {
    for i in 1..sections.len() {
        let (prev, rest) = sections.split_at_mut(i);
        let prev = &prev[i - 1].props;
        let cur = &mut rest[0].props;
        for (mine, theirs) in [
            (&mut cur.headers, &prev.headers),
            (&mut cur.footers, &prev.footers),
        ] {
            if mine.default.is_none() {
                mine.default = theirs.default.clone();
            }
            if mine.first.is_none() {
                mine.first = theirs.first.clone();
            }
            if mine.even.is_none() {
                mine.even = theirs.even.clone();
            }
        }
    }
}

fn parse_css(style: &str) -> HashMap<String, String> {
    style
        .split(';')
        .filter_map(|kv| {
            let (k, v) = kv.split_once(':')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect()
}

/// The DrawingML preset matching a VML shape: by element, or by the shape
/// type number (`#_x0000_tNN`, `o:spt`), which follows the same catalogue.
fn vml_preset(shape: &XNode) -> Option<&'static str> {
    match shape.name.as_str() {
        "v:oval" => return Some("ellipse"),
        "v:roundrect" => return Some("roundRect"),
        "v:line" => return Some("line"),
        _ => {}
    }
    let t = shape
        .attr("type")?
        .trim_start_matches('#')
        .strip_prefix("_x0000_t")?;
    Some(match t.parse::<u32>().ok()? {
        2 => "roundRect",
        3 => "ellipse",
        4 => "diamond",
        5 => "triangle",
        6 => "rtTriangle",
        7 => "parallelogram",
        8 => "trapezoid",
        9 => "hexagon",
        10 => "octagon",
        11 => "plus",
        12 => "star5",
        13 => "rightArrow",
        15 => "homePlate",
        16 => "cube",
        20 | 32 => "line",
        55 => "chevron",
        56 => "pentagon",
        61 => "wedgeRectCallout",
        62 => "wedgeRoundRectCallout",
        63 => "wedgeEllipseCallout",
        66 => "leftArrow",
        67 => "downArrow",
        68 => "upArrow",
        69 => "leftRightArrow",
        70 => "upDownArrow",
        84 => "bevel",
        85 => "leftBracket",
        86 => "rightBracket",
        87 => "leftBrace",
        88 => "rightBrace",
        109 => "flowChartProcess",
        110 => "flowChartDecision",
        111 => "flowChartInputOutput",
        116 => "flowChartTerminator",
        120 => "flowChartConnector",
        176 => "flowChartOffpageConnector",
        183 => "sun",
        184 => "moon",
        185 => "bracketPair",
        186 => "bracePair",
        _ => return None,
    })
}

/// A VML path (`m`, `l`, `c`, `x`, `e` and their relative forms `t`, `r`,
/// `v`) in the shape's `coordorigin`/`coordsize` space, scaled to `w`×`h`
/// points; `nf`/`ns` turn fill and stroke off for the subpath.
fn vml_path(p: &str, shape: &XNode, w: f64, h: f64) -> Option<(tiny_skia::Path, bool, bool, bool)> {
    let pair = |k: &str, d: (f64, f64)| {
        shape
            .attr(k)
            .and_then(|v| {
                let mut it = v.split(',').map(|x| x.trim().parse::<f64>().ok());
                Some((it.next()??, it.next().flatten().unwrap_or(d.1)))
            })
            .unwrap_or(d)
    };
    let (ox, oy) = pair("coordorigin", (0.0, 0.0));
    let (cw, ch) = pair("coordsize", (21600.0, 21600.0));
    let (sx, sy) = (w / cw.max(1.0), h / ch.max(1.0));
    let map = |x: f64, y: f64| (((x - ox) * sx) as f32, ((y - oy) * sy) as f32);
    // Tokens: command letters and numbers (commas and spaces separate; empty numbers are 0).
    let mut toks: Vec<String> = Vec::new();
    let mut num = String::new();
    let flush = |num: &mut String, toks: &mut Vec<String>| {
        if !num.is_empty() {
            toks.push(std::mem::take(num));
        }
    };
    for c in p.chars() {
        if c.is_ascii_alphabetic() {
            flush(&mut num, &mut toks);
            toks.push(c.to_string());
        } else if c == '-' && !num.is_empty() {
            flush(&mut num, &mut toks);
            num.push(c);
        } else if c == ',' || c.is_whitespace() {
            flush(&mut num, &mut toks);
        } else {
            num.push(c);
        }
    }
    flush(&mut num, &mut toks);
    let mut pb = tiny_skia::PathBuilder::new();
    let (mut cx, mut cy) = (0.0f64, 0.0f64);
    let mut i = 0;
    let mut cmd = String::new();
    let (mut fill, mut stroke, mut curve) = (true, true, false);
    let n = |i: usize| toks.get(i).and_then(|t| t.parse::<f64>().ok());
    while i < toks.len() {
        if toks[i].chars().all(|c| c.is_ascii_alphabetic()) {
            cmd = toks[i].clone();
            i += 1;
            match cmd.as_str() {
                "x" => pb.close(),
                "nf" => fill = false,
                "ns" => stroke = false,
                _ => {}
            }
            if !matches!(cmd.as_str(), "m" | "l" | "c" | "t" | "r" | "v") {
                continue;
            }
        }
        let rel = matches!(cmd.as_str(), "t" | "r" | "v");
        let (bx, by) = if rel { (cx, cy) } else { (0.0, 0.0) };
        match cmd.as_str() {
            "m" | "t" | "l" | "r" => {
                let (Some(x), Some(y)) = (n(i), n(i + 1)) else {
                    break;
                };
                i += 2;
                (cx, cy) = (bx + x, by + y);
                let (px, py) = map(cx, cy);
                if cmd == "m" || cmd == "t" {
                    pb.move_to(px, py);
                } else {
                    pb.line_to(px, py);
                }
            }
            "c" | "v" => {
                let v: Vec<f64> = (0..6).filter_map(|k| n(i + k)).collect();
                if v.len() < 6 {
                    break;
                }
                i += 6;
                let (a, b) = map(bx + v[0], by + v[1]);
                let (c2, d) = map(bx + v[2], by + v[3]);
                (cx, cy) = (bx + v[4], by + v[5]);
                let (e, f) = map(cx, cy);
                pb.cubic_to(a, b, c2, d, e, f);
                curve = true;
            }
            _ => i += 1,
        }
    }
    Some((pb.finish()?, fill, stroke, curve))
}

/// A VML colour: `#rrggbb`, `#rgb` or a named colour, with an optional
/// `[index]` suffix.
fn vml_color(c: &str) -> Option<crate::office::pptx::color::Rgba> {
    let c = c.split_whitespace().next()?.trim();
    if let Some(hex) = c.strip_prefix('#') {
        let hex: String = if hex.len() == 3 {
            hex.chars().flat_map(|ch| [ch, ch]).collect()
        } else {
            hex.get(..6)?.to_string()
        };
        return crate::office::pptx::color::Rgba::from_hex(&hex);
    }
    let hex = match c.to_ascii_lowercase().as_str() {
        "black" => "000000",
        "white" => "FFFFFF",
        "silver" => "C0C0C0",
        "gray" | "grey" => "808080",
        "red" => "FF0000",
        "maroon" => "800000",
        "yellow" => "FFFF00",
        "olive" => "808000",
        "lime" => "00FF00",
        "green" => "008000",
        "aqua" | "cyan" => "00FFFF",
        "teal" => "008080",
        "blue" => "0000FF",
        "navy" => "000080",
        "fuchsia" | "magenta" => "FF00FF",
        "purple" => "800080",
        "orange" => "FFA500",
        _ => return crate::office::pptx::color::Rgba::from_hex(c.get(..6)?),
    };
    crate::office::pptx::color::Rgba::from_hex(hex)
}

/// A VML fraction: `.5`, `50%` or `32768f` (65536ths).
fn vml_fraction(v: &str) -> Option<f64> {
    let v = v.trim();
    let f = if let Some(p) = v.strip_suffix('%') {
        p.parse::<f64>().ok()? / 100.0
    } else if let Some(x) = v.strip_suffix('f') {
        x.parse::<f64>().ok()? / 65536.0
    } else {
        v.parse::<f64>().ok()?
    };
    Some(f.clamp(0.0, 1.0))
}

/// A CSS length in points (`pt`, `in`, `cm`, `mm`, `px`; bare numbers are EMU
/// in VML coordinates but points in styles, so bare means points here).
fn css_len(v: &str) -> Option<f64> {
    let v = v.trim();
    let split = v.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(v.len());
    let (num, unit) = v.split_at(split);
    let n: f64 = num.trim().parse().ok()?;
    Some(match unit {
        "in" => n * 72.0,
        "cm" => n * 72.0 / 2.54,
        "mm" => n * 72.0 / 25.4,
        "px" => n * 0.75,
        "emu" => n / EMU_PER_PT,
        _ => n,
    })
}

/// A drawing element's own turn from its `a:xfrm`: (degrees, flipH, flipV);
/// `None` when it is drawn upright and unflipped.
fn turn_of(n: &XNode) -> Option<(f64, bool, bool)> {
    let x = n
        .children
        .iter()
        .find(|c| matches!(c.local(), "spPr" | "grpSpPr"))?
        .child("a:xfrm")?;
    let rot = x.attr_f64("rot").unwrap_or(0.0) / 60000.0;
    let flip = |k: &str| x.attr_bool(k).unwrap_or(false);
    let t = (rot, flip("flipH"), flip("flipV"));
    (rot.rem_euclid(360.0) != 0.0 || t.1 || t.2).then_some(t)
}

/// A note rule of the requested width in points, 0.75 pt thick, in the
/// run's colour (automatic: black), about mid-line (0.6 of the font size above
/// the baseline).
pub(crate) fn note_separator(style: &Rc<RunStyle>, w: f64) -> Inline {
    let h = style.size * 0.6;
    let color = style
        .color
        .unwrap_or(crate::office::pptx::color::Rgba::BLACK);
    let items = tiny_skia::Rect::from_xywh(0.0, 0.0, w as f32, 0.75)
        .map(|r| {
            vec![crate::office::pptx::scene::Item::Path(
                crate::office::pptx::scene::PathItem {
                    path: tiny_skia::PathBuilder::from_rect(r),
                    fill: Some(crate::office::pptx::scene::Paint::Solid(color)),
                    stroke: None,
                    has_curve: false,
                    shape: None,
                    from_text: false,
                },
            )]
        })
        .unwrap_or_default();
    let spec = crate::office::docx::shapes::ShapeSpec {
        items,
        texts: Vec::new(),
        ..Default::default()
    };
    let pic = Picture {
        part: None,
        w,
        h,
        crop: (0.0, 0.0, 0.0, 0.0),
        alt: None,
        shape: Some(Rc::new(spec)),
        effects: Vec::new(),
        alpha: 1.0,
        graphic: None,
        shadow: None,
    };
    Inline::Picture(pic, style.clone())
}

/// The entry a drop-down form field shows: `w:result`, else `w:default`, else the first.
fn dropdown_entry(dd: &XNode) -> Option<String> {
    let idx = |n: &str| dd.child(n).and_then(|v| v.attr_i64("val"));
    let k = idx("w:result").or_else(|| idx("w:default")).unwrap_or(0);
    let entries: Vec<&str> = dd
        .children
        .iter()
        .filter(|c| c.name == "w:listEntry")
        .filter_map(|c| c.attr("val"))
        .collect();
    let k = usize::try_from(k)
        .ok()
        .filter(|k| *k < entries.len())
        .unwrap_or(0);
    entries.get(k).map(|s| (*s).to_string())
}

/// Whether a rotation (degrees) is nearer a quarter turn than a half turn.
fn quarter_turn(deg: f64) -> bool {
    (45.0..135.0).contains(&deg.rem_euclid(180.0))
}

/// Preserve unsupported ruby content through the ordinary base-text path.
fn ruby_group(base: &[Inline], guide: &[Inline], pr: &XNode) -> Option<Ruby> {
    let text_runs = |runs: &[Inline]| -> Option<Vec<(String, Rc<RunStyle>)>> {
        runs.iter()
            .map(|i| match i {
                Inline::Text(t, s) => Some((t.clone(), s.clone())),
                _ => None,
            })
            .collect()
    };
    let base = text_runs(base)?;
    let mut guide = text_runs(guide)?;
    if base.is_empty() || guide.is_empty() {
        return None;
    }
    let align = pr
        .child("w:rubyAlign")
        .and_then(|n| n.attr("val"))
        .unwrap_or("center");
    if !matches!(
        align,
        "center" | "left" | "right" | "distributeLetter" | "distributeSpace"
    ) {
        return None;
    }
    let half_points = |name: &str| {
        pr.child(name)?
            .attr("val")?
            .parse::<i16>()
            .ok()
            .map(|n| n as f64 / 2.0)
    };
    let size = half_points("w:hps")?;
    if size <= 0.0 {
        return None;
    }
    for (_, style) in &mut guide {
        Rc::make_mut(style).size = size;
    }
    Some(Ruby {
        base,
        guide,
        align: align.to_string(),
        raise: half_points("w:hpsRaise")?,
    })
}

/// A wrapping element's explicit distance overrides the anchor on that edge.
/// Missing edges inherit independently, including when another edge is zero.
fn drawing_wrap_distances(anchor: &XNode) -> [f64; 4] {
    let wrap = anchor.children.iter().find(|c| {
        matches!(
            c.name.as_str(),
            "wp:wrapSquare" | "wp:wrapTight" | "wp:wrapThrough" | "wp:wrapTopAndBottom"
        )
    });
    ["distT", "distB", "distL", "distR"].map(|edge| {
        wrap.and_then(|w| w.attr(edge))
            .or_else(|| anchor.attr(edge))
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
            / EMU_PER_PT
    })
}

#[cfg(test)]
mod tests;
