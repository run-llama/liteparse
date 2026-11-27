//! The extract request: opens the document, lays it out into pages, renders
//! each page and writes the protocol files (`index.json`, `pages.jsonl`,
//! rasters) and the optional screenshot PDF. Same contract as `pptx-read`
//! (`rust/crates/pptx-read/src/extract.rs`), with `sourceFormat: "docx"`.

use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use crate::office::pptx::config::{Config, PdfSource, parse_target_pages};
use crate::office::pptx::document::Document;
use crate::office::pptx::error::{DocError, DocResult};
pub use crate::office::pptx::extract::Extracted;
use crate::office::pptx::fonts::FontSystem;
use crate::office::pptx::package::Package;
use crate::office::pptx::render::{self, ImageCache, RenderTarget};
use crate::office::pptx::scene::{Rect, Scene};
use crate::office::pptx::xml::{self, XNode};
use serde_json::{Map, Value, json};

use crate::office::docx::model::{Block, Section, Settings};
use crate::office::docx::paginate::{HdrFtr, OutPage, Paginator};
use crate::office::docx::parse::Parser;
use crate::office::docx::styles::StyleSheet;
use crate::office::docx::text::TextEnv;

mod bookmarks;
mod source;
mod special_notes;
pub use special_notes::{SpecialNoteKind, SpecialNoteStories, SpecialNoteStory};

/// A document laid out into pages.
pub struct Laid {
    pub pages: Vec<OutPage>,
    pub sections: usize,
    /// Graphics diagnostics with source-part context, before pages are assigned.
    pub warnings: Vec<(String, String)>,
}

/// The parsed package: main part, styles, settings and the body.
pub struct Opened {
    pub bookmarks: Vec<Arc<crate::office::pptx::scene::SourceBookmark>>,
    /// Picture parts previewing equation objects, whose metafile text is not extracted.
    pub equation_parts: std::collections::HashSet<String>,
    /// Header and footer STYLEREF fields, with the id of the style each names.
    pub style_refs: Vec<(crate::office::docx::parse::StyleRef, Option<String>)>,
    pub import_warnings: Vec<(String, String)>,
    pub pkg: Arc<Package>,
    pub main: String,
    pub settings: Settings,
    pub sections: Vec<Section>,
    pub hdr_ftr: HashMap<String, Vec<Block>>,
    /// Footnotes by id, and the separator drawn above them.
    pub notes: HashMap<i64, Vec<Block>>,
    pub note_separator: Vec<Block>,
    pub note_continuation_separator: Option<Vec<Block>>,
    pub note_continuation_notice: Option<Vec<Block>>,
    pub footnote_specials: SpecialNoteStories,
    pub endnote_specials: SpecialNoteStories,
}

pub(crate) fn read_xml(pkg: &Package, part: &str) -> Option<Rc<XNode>> {
    pkg.read_string(part)
        .ok()
        .filter(|s| xml_depth(s) <= MAX_XML_DEPTH)
        .and_then(|s| xml::parse_word(&s).ok())
}

/// Deepest element nesting accepted in a part. Parsing, layout and dropping
/// the tree recurse once per level; real documents stay far below this (three
/// levels per nested table).
const MAX_XML_DEPTH: usize = 3_000;

/// The element nesting depth of an XML text, from its tags (comments,
/// declarations and processing instructions skipped; `>` in attribute values
/// may end a tag early, which only lowers the count).
pub fn xml_depth(text: &str) -> usize {
    let b = text.as_bytes();
    let (mut depth, mut max, mut i) = (0usize, 0usize, 0usize);
    while let Some(k) = b[i..].iter().position(|c| *c == b'<') {
        let start = i + k;
        // Comments and CDATA sections end at their own terminators.
        let close: &[u8] = if b[start..].starts_with(b"<!--") {
            b"-->"
        } else if b[start..].starts_with(b"<![CDATA[") {
            b"]]>"
        } else {
            b">"
        };
        let end = match b[start..].windows(close.len()).position(|w| w == close) {
            Some(e) if close.len() > 1 => {
                i = start + e + close.len();
                continue;
            }
            Some(e) => start + e,
            None => break,
        };
        match b.get(start + 1) {
            Some(b'/') => depth = depth.saturating_sub(1),
            Some(b'?') | Some(b'!') => {}
            _ if b[end - 1] == b'/' => {}
            _ => {
                depth += 1;
                max = max.max(depth);
            }
        }
        i = end + 1;
    }
    max
}

/// Signature of an OLE compound file: binary Word (.doc), or an encrypted .docx.
const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// A binary Word file (a compound file with a `WordDocument` stream) becomes a
/// .docx through doc-read; any other input is returned unchanged.
fn docx_bytes(bytes: Vec<u8>) -> DocResult<Vec<u8>> {
    if !bytes.starts_with(&CFB_MAGIC) {
        return Ok(bytes);
    }
    let is_doc = crate::office::doc::compound::Compound::open(&bytes)
        .is_ok_and(|c| c.stream("WordDocument").is_ok());
    if !is_doc {
        return Ok(bytes);
    }
    match crate::office::doc::docx_export::convert(&bytes) {
        Ok(converted) => Ok(converted.docx),
        Err(crate::office::doc::Error::Unsupported("encrypted binary Word")) => {
            Err(DocError::Protected)
        }
        Err(e) => Err(DocError::Broken(format!("binary Word: {e}"))),
    }
}

pub fn open(bytes: Vec<u8>, cfg: &Config) -> DocResult<Opened> {
    let pkg = Arc::new(Package::open(docx_bytes(bytes)?, &cfg.limits)?);
    let main = pkg
        .rel_by_kind("", "officeDocument")
        .map(|r| r.target)
        .filter(|p| pkg.exists(p))
        .unwrap_or_else(|| "word/document.xml".to_string());
    let text = pkg
        .read_string(&main)
        .map_err(|_| DocError::Broken("no document part".into()))?;
    if xml_depth(&text) > MAX_XML_DEPTH {
        return Err(DocError::LimitExceeded("document nesting too deep".into()));
    }
    let root =
        xml::parse_word(&text).map_err(|e| DocError::Broken(format!("document.xml: {e}")))?;
    if root.name != "w:document" {
        return Err(DocError::Broken(format!(
            "root element {} is not a document",
            root.name
        )));
    }
    let part_of = |kind: &str| pkg.rel_by_kind(&main, kind).map(|r| r.target);
    let settings_x = part_of("settings").and_then(|p| read_xml(&pkg, &p));
    let settings = parse_settings(settings_x.as_deref());
    let theme_x = part_of("theme").and_then(|p| read_xml(&pkg, &p));
    let styles_x = part_of("styles").and_then(|p| read_xml(&pkg, &p));
    let numbering_x = part_of("numbering").and_then(|p| read_xml(&pkg, &p));
    // Tests that read the font table directly hold this lock across their reads.
    #[cfg(test)]
    let _font_table_guard = crate::office::docx::metrics::FONT_TABLE_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    crate::office::docx::metrics::set_font_table(
        part_of("fontTable")
            .and_then(|p| read_xml(&pkg, &p))
            .as_deref(),
    );
    // The east asian language picks the theme's script font when its `a:ea` is empty.
    let lang_ea = settings.lang_ea.clone().or_else(|| {
        styles_x
            .as_ref()
            .and_then(|s| {
                s.path(&["w:docDefaults", "w:rPrDefault", "w:rPr", "w:lang"])
                    .cloned()
            })
            .and_then(|l| l.attr("eastAsia").map(str::to_string))
    });
    crate::office::docx::metrics::set_doc_lang_ea(lang_ea.as_deref());
    let lang_bidi = settings.lang_bidi.clone().or_else(|| {
        styles_x
            .as_ref()
            .and_then(|s| {
                s.path(&["w:docDefaults", "w:rPrDefault", "w:rPr", "w:lang"])
                    .cloned()
            })
            .and_then(|l| l.attr("bidi").map(str::to_string))
    });
    let theme = crate::office::docx::styles::theme_fonts(
        theme_x.as_deref(),
        lang_ea.as_deref(),
        lang_bidi.as_deref(),
    );
    let mut sheet = StyleSheet::parse(styles_x.as_deref(), theme);
    sheet.compat_mode = settings.compat_mode;
    sheet.theme_colors = crate::office::docx::theme_colors::ThemeColors::parse(
        theme_x.as_deref(),
        settings_x.as_deref(),
    );
    let numbering = crate::office::docx::numbering::Numbering::parse_in_part(
        numbering_x.as_deref(),
        part_of("numbering"),
    );
    let mut parser = Parser::new(&pkg, &sheet, &settings, numbering);
    if let (Some(t), Some(p)) = (theme_x.as_ref(), part_of("theme")) {
        parser.theme = Some((crate::office::pptx::theme::Theme::parse(t), p));
    }
    let body = root
        .child("w:body")
        .ok_or_else(|| DocError::Broken("document has no body".into()))?;
    let mut sections = parser.body(body, &main);
    let mut hdr_ftr = HashMap::new();
    for s in &sections {
        parser.set_section(&s.props);
        for refs in [&s.props.headers, &s.props.footers] {
            for part in [&refs.default, &refs.first, &refs.even]
                .into_iter()
                .flatten()
            {
                if hdr_ftr.contains_key(part) {
                    continue;
                }
                let blocks = read_xml(&pkg, part)
                    .map(|x| parser.blocks(&x.children, part, None))
                    .unwrap_or_default();
                hdr_ftr.insert(part.clone(), blocks);
            }
        }
    }
    // A page background (`w:background`): Word for Mac adds an empty line under the
    // header, and gives a section without one an empty header, lowering the body
    // (a page with no header part starts its text two lines below the header edge).
    if root.child("w:background").is_some() {
        // Its lines are single-spaced without paragraph spacing, as Word's built-in
        // Header style (12 pt Calibri lines 14.9 pt apart), in that style if defined.
        let spacing = r#"<w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/>"#;
        let style = if parser.sheet.styles.contains_key("Header") {
            format!(r#"<w:pPr><w:pStyle w:val="Header"/>{spacing}</w:pPr>"#)
        } else {
            format!("<w:pPr>{spacing}</w:pPr>")
        };
        let empty = crate::office::pptx::xml::parse_word(&format!(
            r#"<w:p xmlns:w="{}">{style}</w:p>"#,
            "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
        ))
        .map(|p| parser.blocks(std::slice::from_ref(&p), &main, None))
        .unwrap_or_default();
        let headers: std::collections::HashSet<String> = sections
            .iter()
            .flat_map(|s| {
                let h = &s.props.headers;
                [h.default.clone(), h.first.clone(), h.even.clone()]
            })
            .flatten()
            .collect();
        for part in headers {
            if let Some(blocks) = hdr_ftr.get_mut(&part) {
                blocks.extend(empty.iter().cloned());
            }
        }
        let implicit = "\u{1}background-header".to_string();
        let mut used = false;
        for s in &mut sections {
            if s.props.headers.default.is_none() {
                s.props.headers.default = Some(implicit.clone());
                used = true;
            }
        }
        if used {
            hdr_ftr.insert(implicit, empty.iter().chain(&empty).cloned().collect());
        }
    }
    let mut notes = HashMap::new();
    let mut note_separator = Vec::new();
    let mut footnote_specials =
        SpecialNoteStories::read(None, "", settings_x.as_deref(), "footnote");
    let mut endnote_specials = SpecialNoteStories::read(None, "", settings_x.as_deref(), "endnote");
    if let Some(fp) = part_of("footnotes") {
        let source = read_xml(&pkg, &fp);
        footnote_specials =
            SpecialNoteStories::read(source.as_deref(), &fp, settings_x.as_deref(), "footnote");
        if let Some(x) = source {
            for f in x.children_named("w:footnote") {
                match f.attr("type") {
                    Some("separator") => note_separator = parser.blocks(&f.children, &fp, None),
                    None | Some("normal") => {
                        let Some(id) = f.attr_i64("id") else { continue };
                        let n = parser.note_numbers.borrow().get(&id).cloned();
                        *parser.current_note.borrow_mut() = n;
                        notes.insert(id, parser.blocks(&f.children, &fp, None));
                    }
                    Some(_) => {}
                }
            }
        }
    }
    // Each destination section gets one endnote area and its separator.
    if let Some(ep) = part_of("endnotes") {
        let source = read_xml(&pkg, &ep);
        endnote_specials =
            SpecialNoteStories::read(source.as_deref(), &ep, settings_x.as_deref(), "endnote");
        if let Some(x) = source {
            let order = parser.endnote_order.borrow().clone();
            let mut tails = vec![Vec::new(); sections.len()];
            let mut started = vec![false; sections.len()];
            let at_section_end = settings.endnote_pos.as_deref() == Some("sectEnd");
            for (id, shown, source_section) in order {
                let target = if at_section_end {
                    source_section
                } else {
                    sections.len() - 1
                };
                let target = target.min(sections.len() - 1);
                parser.set_section(&sections[target].props);
                let tail = &mut tails[target];
                if !started[target] {
                    if let Some(sep) = x
                        .children_named("w:endnote")
                        .find(|e| e.attr("type") == Some("separator"))
                    {
                        tail.extend(parser.blocks(&sep.children, &ep, None));
                    }
                    started[target] = true;
                }
                let Some(e) = x.children_named("w:endnote").find(|e| {
                    e.attr_i64("id") == Some(id) && matches!(e.attr("type"), None | Some("normal"))
                }) else {
                    continue;
                };
                *parser.current_note.borrow_mut() = Some(shown);
                tail.extend(parser.blocks(&e.children, &ep, None));
            }
            for (section, tail) in sections.iter_mut().zip(tails) {
                section.blocks.extend(tail);
            }
        }
    }
    // Selected special stories get isolated parser state and relationship context.
    let parse_special = |kind| -> DocResult<Option<Vec<Block>>> {
        let story = footnote_specials.references.iter().find_map(|id| {
            footnote_specials
                .definitions
                .iter()
                .find(|story| story.id == *id && story.kind == kind)
        });
        let Some(story) = story else {
            return Ok(None);
        };
        let mut special = Parser::new(
            &pkg,
            &sheet,
            &settings,
            crate::office::docx::numbering::Numbering::parse_in_part(
                numbering_x.as_deref(),
                part_of("numbering"),
            ),
        );
        special.share_grid_budget(&parser);
        special.share_bookmarks(&parser);
        special.share_imports(&parser);
        special.theme = parser.theme.clone();
        if let Some(section) = sections.first() {
            special.set_section(&section.props);
        }
        let blocks = special.blocks(&story.source.children, &story.part, None);
        special.check_grid_budget()?;
        Ok(Some(blocks))
    };
    let note_continuation_separator = parse_special(SpecialNoteKind::ContinuationSeparator)?;
    let note_continuation_notice = parse_special(SpecialNoteKind::ContinuationNotice)?;
    parser.check_grid_budget()?;
    let import_warnings = parser.import_warnings();
    let bookmarks = parser.bookmark_definitions();
    let style_refs = resolve_style_refs(&pkg, parser.style_references());
    let equation_parts = parser.equation_parts();
    drop(parser);
    Ok(Opened {
        bookmarks,
        equation_parts,
        style_refs,
        import_warnings,
        pkg,
        main,
        settings,
        sections,
        hdr_ftr,
        notes,
        note_separator,
        note_continuation_separator,
        note_continuation_notice,
        footnote_specials,
        endnote_specials,
    })
}

fn style_ref_inputs(opened: &Opened) -> Vec<(Option<String>, bool, String)> {
    opened
        .style_refs
        .iter()
        .map(|(r, id)| (id.clone(), r.number, r.saved.clone()))
        .collect()
}

/// The style id each STYLEREF names: the paragraph style whose name, or one of its
/// comma-separated aliases, matches the field's name without regard to case.
fn resolve_style_refs(
    pkg: &Package,
    refs: Vec<crate::office::docx::parse::StyleRef>,
) -> Vec<(crate::office::docx::parse::StyleRef, Option<String>)> {
    if refs.is_empty() {
        return Vec::new();
    }
    let styles = pkg
        .rel_by_kind("word/document.xml", "styles")
        .map(|r| r.target)
        .unwrap_or_else(|| "word/styles.xml".into());
    let mut names: Vec<(Vec<String>, String)> = Vec::new();
    if let Some(root) = read_xml(pkg, &styles) {
        for st in root.children.iter().filter(|c| c.name == "w:style") {
            let (Some(id), Some(name)) = (
                st.attr("styleId"),
                st.child("w:name").and_then(|n| n.attr("val")),
            ) else {
                continue;
            };
            let mut all = vec![name.to_lowercase()];
            all.extend(name.split(',').map(|a| a.trim().to_lowercase()));
            names.push((all, id.to_string()));
        }
    }
    refs.into_iter()
        .map(|r| {
            let wanted = r.name.to_lowercase();
            let id = names
                .iter()
                .find(|(all, _)| all.contains(&wanted))
                .map(|(_, id)| id.clone());
            (r, id)
        })
        .collect()
}

fn parse_settings(x: Option<&XNode>) -> Settings {
    let mut s = Settings {
        default_tab: 36.0,
        compat_mode: 12,
        ..Default::default()
    };
    s.math_justification = x
        .and_then(|x| x.path(&["m:mathPr", "m:defJc"]))
        .and_then(|n| n.attr("val"))
        .map(str::to_owned);
    if let Some(f) = x.and_then(|x| x.child("w:footnotePr")) {
        (s.note_start, s.note_fmt) = crate::office::docx::parse::note_pr(f);
    }
    if let Some(f) = x.and_then(|x| x.child("w:endnotePr")) {
        (s.endnote_start, s.endnote_fmt) = crate::office::docx::parse::note_pr(f);
        s.endnote_restart = f
            .child("w:numRestart")
            .and_then(|n| n.attr("val"))
            .map(str::to_string);
        s.endnote_pos = f
            .child("w:pos")
            .and_then(|n| n.attr("val"))
            .map(str::to_string);
    }
    crate::office::docx::metrics::set_balance_spaces(
        x.and_then(|x| x.child("w:compat"))
            .and_then(|c| c.child("w:balanceSingleByteDoubleByteWidth"))
            .is_some_and(|n| crate::office::docx::props::on_off(n)),
    );
    crate::office::docx::metrics::set_no_tab_hang_ind(
        x.and_then(|x| x.child("w:compat"))
            .and_then(|c| c.child("w:noTabHangInd"))
            .is_some_and(|n| crate::office::docx::props::on_off(n)),
    );
    crate::office::docx::metrics::set_compress_punct(
        x.and_then(|x| x.child("w:characterSpacingControl"))
            .and_then(|c| c.attr("val")),
    );
    crate::office::docx::hyph::set_settings(
        x.and_then(|x| x.child("w:autoHyphenation"))
            .is_some_and(|n| crate::office::docx::props::on_off(n)),
        x.and_then(|x| x.child("w:hyphenationZone"))
            .and_then(|z| z.attr_f64("val")),
        x.and_then(|x| x.child("w:consecutiveHyphenLimit"))
            .and_then(|z| z.attr_i64("val")),
        x.and_then(|x| x.child("w:doNotHyphenateCaps"))
            .is_some_and(|n| crate::office::docx::props::on_off(n)),
    );
    let Some(x) = x else {
        return s;
    };
    if let Some(t) = x.child("w:defaultTabStop").and_then(|t| t.attr_f64("val"))
        && t > 0.0
    {
        s.default_tab = t / 20.0;
    }
    s.even_and_odd = x
        .child("w:evenAndOddHeaders")
        .is_some_and(|n| crate::office::docx::props::on_off(n));
    if let Some(c) = x.child("w:compat") {
        s.do_not_expand_shift_return = c.child("w:doNotExpandShiftReturn").is_some();
        s.grid_in_table = c.child("w:adjustLineHeightInTable").is_some();
        s.no_leading = c.child("w:noLeading").is_some();
        s.grow_autofit = c
            .child("w:growAutofit")
            .is_some_and(|n| crate::office::docx::props::on_off(n));
        s.no_html_spacing = c
            .child("w:doNotUseHTMLParagraphAutoSpacing")
            .is_some_and(|n| crate::office::docx::props::on_off(n));
        for cs in c.children_named("w:compatSetting") {
            if cs.attr("name") == Some("compatibilityMode")
                && let Some(v) = cs.attr_i64("val")
            {
                s.compat_mode = v as u32;
            }
        }
    }
    s.lang_ea = x
        .child("w:themeFontLang")
        .and_then(|l| l.attr("eastAsia"))
        .map(str::to_string);
    s.lang_bidi = x
        .child("w:themeFontLang")
        .and_then(|l| l.attr("bidi"))
        .map(str::to_string);
    s
}

pub fn layout(opened: &Opened, fonts: &FontSystem) -> Laid {
    layout_with_image_limit(
        opened,
        fonts,
        crate::office::pptx::config::Config::default()
            .limits
            .max_image_pixels,
    )
}

fn layout_with_image_limit(opened: &Opened, fonts: &FontSystem, max_image_pixels: u64) -> Laid {
    let s = &opened.settings;
    let grid = opened.sections.first().and_then(|s| s.props.line_grid());
    let env = TextEnv {
        fonts,
        default_tab: s.default_tab,
        fields: Default::default(),
        grid,
        char_grid: Default::default(),
        expand_shift_return: !s.do_not_expand_shift_return,
        mark_in_last_line: true,
        html_spacing: !s.no_html_spacing,
        grow_autofit: s.grow_autofit,
        in_cell: Default::default(),
        in_header: Default::default(),
        grid_in_table: s.grid_in_table,
        no_leading: s.no_leading,
        compat_mode: s.compat_mode,
    };
    let pager = Paginator::new(
        &env,
        s.compat_mode,
        s.even_and_odd,
        HdrFtr {
            blocks: &opened.hdr_ftr,
            notes: &opened.notes,
            separator: &opened.note_separator,
            continuation_separator: opened.note_continuation_separator.as_ref(),
            continuation_notice: opened.note_continuation_notice.as_ref(),
        },
    )
    .with_style_refs(style_ref_inputs(opened));
    // Charts and diagrams are drawn now that fonts are known (on copies).
    let has_graphics = opened
        .sections
        .iter()
        .any(|s| crate::office::docx::graphics::any(&s.blocks))
        || opened
            .hdr_ftr
            .values()
            .any(|b| crate::office::docx::graphics::any(b))
        || opened
            .notes
            .values()
            .any(|b| crate::office::docx::graphics::has_picture_labels(b))
        || crate::office::docx::graphics::has_picture_labels(&opened.note_separator)
        || opened
            .note_continuation_separator
            .as_ref()
            .is_some_and(|b| crate::office::docx::graphics::any(b))
        || opened
            .note_continuation_notice
            .as_ref()
            .is_some_and(|b| crate::office::docx::graphics::any(b));
    let mut warnings = Vec::new();
    let pages = if has_graphics {
        let g = crate::office::docx::graphics::Graphics::with_image_limit(
            opened.pkg.clone(),
            &opened.main,
            fonts,
            max_image_pixels,
        );
        let mut sections = opened.sections.clone();
        g.sections(&mut sections);
        let mut hdr_ftr = opened.hdr_ftr.clone();
        for b in hdr_ftr.values_mut() {
            g.blocks(b);
        }
        let mut notes = opened.notes.clone();
        for blocks in notes.values_mut() {
            g.picture_labels(blocks);
        }
        let mut separator = opened.note_separator.clone();
        g.picture_labels(&mut separator);
        let mut continuation_separator = opened.note_continuation_separator.clone();
        let mut continuation_notice = opened.note_continuation_notice.clone();
        for blocks in [&mut continuation_separator, &mut continuation_notice]
            .into_iter()
            .flatten()
        {
            g.blocks(blocks);
        }
        let pager = Paginator::new(
            &env,
            s.compat_mode,
            s.even_and_odd,
            HdrFtr {
                blocks: &hdr_ftr,
                notes: &notes,
                separator: &separator,
                continuation_separator: continuation_separator.as_ref(),
                continuation_notice: continuation_notice.as_ref(),
            },
        )
        .with_style_refs(style_ref_inputs(opened));
        warnings = g.take_warnings();
        let mut pages = pager.run(&sections);
        g.mark_picture_tables(&mut pages);
        pages
    } else {
        pager.run(&opened.sections)
    };
    warnings.extend(opened.import_warnings.iter().cloned());
    for page in &pages {
        for detail in &page.info.note_diagnostics {
            let warning = ("FOOTNOTE_AREA_UNSUPPORTED".to_string(), detail.clone());
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
    }
    if let Some(warning) = offpage_text_warning(&pages) {
        warnings.push(warning);
    }
    Laid {
        pages,
        sections: opened.sections.len(),
        warnings,
    }
}

/// Decode `bytes` with the request in `config_json`: the index object and
/// the page records as JSON lines, in memory. Screenshot and image files still
/// go to the directories the request names. A document-level error (protected,
/// broken, timed out) is reported in the index, as in the file protocol.
pub fn read(config_json: &str, bytes: Vec<u8>) -> Result<Extracted, String> {
    let cfg: Config = serde_json::from_str(config_json).map_err(|e| format!("bad config: {e}"))?;
    let started = Instant::now();
    let opened = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| open(bytes, &cfg)))
    {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Ok(error_index(&e)),
        Err(p) => {
            let e = DocError::Broken(format!("panic: {}", panic_message(&p)));
            return Ok(error_index(&e));
        }
    };
    let fonts = FontSystem::new(
        &cfg.render.font_dirs,
        !cfg.render.no_system_fonts,
        &cfg.render.font_substitutions,
    );
    let font_warnings =
        crate::office::docx::embedded_fonts::register(&opened.pkg, &opened.main, &fonts);
    crate::office::docx::metrics::set_word_metrics(cfg.render.word_metrics);
    // Layout stops at the document budget (the rest of it is left to rendering).
    crate::office::docx::deadline::start(Some(std::time::Duration::from_millis(
        cfg.render
            .document_budget_ms
            .saturating_sub(started.elapsed().as_millis() as u64),
    )));
    let laid = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        layout_with_image_limit(&opened, &fonts, cfg.limits.max_image_pixels)
    }));
    crate::office::docx::deadline::start(None);
    let laid = match laid {
        Ok(l) => l,
        Err(p) if crate::office::docx::deadline::expired(&p) => {
            return Ok(error_index(&DocError::Timeout));
        }
        Err(p) => {
            let e = DocError::Broken(format!("layout panic: {}", panic_message(&p)));
            return Ok(error_index(&e));
        }
    };
    if laid.pages.len() > cfg.limits.max_slides as usize {
        let e = DocError::LimitExceeded(format!("{} pages", laid.pages.len()));
        return Ok(error_index(&e));
    }
    let (w0, h0) = laid
        .pages
        .first()
        .map_or((612.0, 792.0), |p| (p.info.w, p.info.h));
    let doc = Document::from_package(
        opened.pkg.clone(),
        (w0 * crate::office::docx::parse::EMU_PER_PT) as i64,
        (h0 * crate::office::docx::parse::EMU_PER_PT) as i64,
    );
    let images = ImageCache::new(cfg.limits.max_image_pixels, &fonts);
    let layers = crate::office::pptx::layers::LayerCache::default();
    let bookmark_index = bookmarks::Index::new(&opened.bookmarks, &laid.pages);
    let total = laid.pages.len();
    let mut selected: Vec<u32> = match cfg.target_pages.as_deref().filter(|s| !s.trim().is_empty())
    {
        Some(spec) => parse_target_pages(spec)
            .into_iter()
            .filter(|p| (*p as usize) < total)
            .collect(),
        None => (0..total as u32).collect(),
    };
    if cfg.max_pages > 0 {
        selected.truncate(cfg.max_pages as usize);
    }
    let mut out = PageWriter {
        file: Vec::new(),
        offset: 0,
        index: Vec::new(),
        pdf_pages: Vec::new(),
        warnings: laid
            .warnings
            .iter()
            .chain(font_warnings.iter())
            .map(|(code, detail)| json!({ "code": code, "detail": detail }))
            .collect(),
    };
    let workers = parallel::worker_count(cfg.workers, selected.len());
    let finished = if workers > 1 {
        let job = parallel::Job {
            pkg: &opened.pkg,
            size: (doc.slide_w, doc.slide_h),
            fonts: &fonts,
            images: &images,
            layers: &layers,
            cfg: &cfg,
            pages: &laid.pages,
            bookmarks: &bookmark_index,
            equations: &opened.equation_parts,
            selected: &selected,
            started,
        };
        parallel::run(&job, workers, &mut out)?
    } else {
        std::thread::scope(|scope| -> Result<bool, String> {
            let worker = crate::office::pptx::shots::ShotWorker::spawn(scope);
            let mut queue: std::collections::VecDeque<Queued> = Default::default();
            for page in &selected {
                if started.elapsed().as_millis() as u64 > cfg.render.document_budget_ms {
                    return Ok(false);
                }
                let p = &laid.pages[*page as usize];
                let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    page_record(
                        &doc,
                        &fonts,
                        &images,
                        &layers,
                        &cfg,
                        *page,
                        (p, &bookmark_index, &opened.equation_parts),
                    )
                }))
                .map_err(|p| DocError::Broken(format!("panic: {}", panic_message(&p))));
                let built = built.map(|(record, pending)| {
                    let shot = match pending.map(|s| worker.submit(s)) {
                        None => Shot::None,
                        Some(Ok(())) => Shot::Submitted,
                        Some(Err(s)) => Shot::Done(Ok(s.finish())),
                    };
                    (record, shot)
                });
                queue.push_back((*page, (p.info.w, p.info.h), built));
                if queue.len() > 1
                    && let Some(q) = queue.pop_front()
                {
                    out.commit(q, &worker)?;
                }
            }
            while let Some(q) = queue.pop_front() {
                out.commit(q, &worker)?;
            }
            Ok(true)
        })?
    };
    if !finished {
        return Ok(error_index(&DocError::Timeout));
    }
    let PageWriter {
        file: pages_file,
        index,
        pdf_pages,
        mut warnings,
        ..
    } = out;
    for (from, to) in fonts.substituted.borrow().iter() {
        warnings.push(json!({ "code": "FONT_SUBSTITUTED", "detail": format!("{from} → {to}") }));
    }
    for (part, why) in images.store().failures() {
        warnings.push(json!({ "code": "IMAGE_UNDECODABLE", "detail": format!("{part}: {why}") }));
    }
    let mut pdf_path = None;
    if cfg.pdf.enabled
        && let Some(path) = &cfg.pdf.output_path
    {
        let pages = if cfg.pdf.source == PdfSource::Render || pdf_pages.len() != selected.len() {
            render_pdf_pages(&doc, &fonts, &images, &cfg, &laid, &selected, path)?
        } else {
            pdf_pages
        };
        crate::office::pptx::pdf::write_pdf(path, &pages).map_err(|e| format!("pdf: {e}"))?;
        pdf_path = Some(path.clone());
    }
    let mut idx = Map::new();
    idx.insert("totalPages".into(), json!(total));
    if let Some(app) = &doc.meta.app {
        idx.insert("creator".into(), json!(app));
    }
    if let Some(v) = &doc.meta.app_version {
        idx.insert("producer".into(), json!(v));
    }
    idx.insert("index".into(), Value::Array(index));
    idx.insert("sourceFormat".into(), json!("docx"));
    if cfg.annotation.enabled {
        idx.insert(
            "document".into(),
            json!({ "comments": crate::office::docx::comments::read(&opened.pkg, &opened.main) }),
        );
    }
    idx.insert("pageSize".into(), json!({ "w": w0, "h": h0 }));
    idx.insert("sections".into(), json!(laid.sections));
    idx.insert(
        "docMeta".into(),
        serde_json::to_value(&doc.meta).unwrap_or(Value::Null),
    );
    if let Some(p) = pdf_path {
        idx.insert("pdfPath".into(), json!(p));
    }
    idx.insert("warnings".into(), Value::Array(warnings));
    idx.insert(
        "layoutMs".into(),
        json!(started.elapsed().as_millis() as u64),
    );
    if let Some(rss) = crate::office::docx::memory::peak_rss_bytes() {
        idx.insert("peakRssBytes".into(), json!(rss));
    }
    Ok(Extracted {
        index: Value::Object(idx),
        pages: pages_file,
    })
}

type Queued = (u32, (f64, f64), DocResult<(Value, Shot)>);

enum Shot {
    None,
    Submitted,
    Done(crate::office::pptx::shots::Finished),
}

struct PageWriter {
    /// The page records, one JSON line each.
    file: Vec<u8>,
    offset: u64,
    index: Vec<Value>,
    pdf_pages: Vec<crate::office::pptx::pdf::PdfPage>,
    warnings: Vec<Value>,
}

impl PageWriter {
    fn commit(
        &mut self,
        q: Queued,
        worker: &crate::office::pptx::shots::ShotWorker,
    ) -> Result<(), String> {
        let ready = finish_record(q, Some(worker))?;
        self.write(ready)
    }

    /// Appends a finished record to the pages file.
    fn write(&mut self, r: Ready) -> Result<(), String> {
        if let Some(w) = r.warning {
            self.warnings.push(w);
        }
        self.index
            .push(json!({ "page": r.page, "offset": self.offset }));
        self.file.write_all(&r.line).map_err(|e| e.to_string())?;
        self.file.write_all(b"\n").map_err(|e| e.to_string())?;
        self.offset += r.line.len() as u64 + 1;
        if let Some(p) = r.pdf {
            self.pdf_pages.push(p);
        }
        Ok(())
    }
}

/// A page record serialized and ready to write: render threads finish their
/// pages into these, so only bytes wait for their turn, not JSON trees.
struct Ready {
    page: u32,
    line: Vec<u8>,
    pdf: Option<crate::office::pptx::pdf::PdfPage>,
    warning: Option<Value>,
}

/// Completes a page record with its screenshot (from `worker` when it was
/// submitted there) and serializes it.
fn finish_record(
    (page, (w, h), built): Queued,
    worker: Option<&crate::office::pptx::shots::ShotWorker>,
) -> Result<Ready, String> {
    let built = built.and_then(|(mut record, shot)| {
        let finished = match shot {
            Shot::None => return Ok((record, None)),
            Shot::Submitted => worker
                .and_then(|w| w.next())
                .unwrap_or_else(|| Err(Box::new("screenshot worker stopped"))),
            Shot::Done(f) => f,
        };
        match finished {
            Ok(Some((shot, path))) => {
                record["screenshot"] = shot;
                Ok((record, Some(path)))
            }
            Ok(None) => {
                if let Some(obj) = record.as_object_mut() {
                    obj.shift_remove("screenshot");
                }
                Ok((record, None))
            }
            Err(p) => Err(DocError::Broken(format!("panic: {}", panic_message(&p)))),
        }
    });
    let (record, shot, warning) = match built {
        Ok((r, s)) => (r, s, None),
        Err(e) => (
            json!({ "page": page, "error": "PDF_PAGE_ERROR" }),
            None,
            Some(json!({ "code": "PAGE_RENDER_FAILED", "page": page, "detail": e.to_string() })),
        ),
    };
    let line = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
    Ok(Ready {
        page,
        line,
        pdf: shot.map(|path| crate::office::pptx::pdf::PdfPage {
            width_pt: w,
            height_pt: h,
            image_path: path,
        }),
        warning,
    })
}

/// Reports source text subject to the physical-page text-export clip.
/// Generated furniture, transparent runs and whitespace are excluded;
/// the aggregate deliberately contains counts only, without document text.
fn offpage_text_warning(pages: &[OutPage]) -> Option<(String, String)> {
    let mut runs = 0;
    let mut affected_pages = 0;
    for page in pages {
        let count = page
            .items
            .iter()
            .filter(|item| {
                let crate::office::pptx::scene::Item::Text(run) = item else {
                    return false;
                };
                if run.text.trim().is_empty()
                    || ((run.paragraph == 0
                        || run.paragraph == crate::office::docx::text::FOOTER_KEY)
                        && run.source.is_none())
                    || (run.color.a == 0.0 && run.outline.is_none_or(|(c, _)| c.a == 0.0))
                {
                    return false;
                }
                let Some(bbox) = crate::office::pptx::textlayout::transformed_bbox(
                    &run.transform,
                    0.0,
                    -run.ascent,
                    run.width,
                    run.ascent + run.descent,
                ) else {
                    return false;
                };
                bbox.x >= page.info.w
                    || bbox.y >= page.info.h
                    || bbox.x + bbox.w <= 0.0
                    || bbox.y + bbox.h <= 0.0
            })
            .count();
        runs += count;
        affected_pages += usize::from(count > 0);
    }
    (runs > 0).then(|| (
        "TEXT_OUTSIDE_PAGE".to_string(),
        format!("{runs} nonempty source text runs wholly outside {affected_pages} physical pages in the full laid-out document would be omitted by page text export; generated furniture, transparent and whitespace-only runs are excluded"),
    ))
}

fn scene_of(p: &OutPage) -> Scene {
    Scene {
        width: p.info.w,
        height: p.info.h,
        items: p.items.clone(),
        ..Default::default()
    }
}

/// `scene` plus the text drawn inside its metafile pictures (EMF/WMF diagrams), as
/// runs placed over each picture and not drawn again: Word's PDF keeps that text,
/// except for equation objects' previews.
fn with_metafile_text(
    scene: &Scene,
    doc: &Document,
    images: &ImageCache,
    fonts: &FontSystem,
    equations: &std::collections::HashSet<String>,
) -> Scene {
    let mut runs = Vec::new();
    for item in &scene.items {
        if let crate::office::pptx::scene::Item::Image(i) = item {
            let part = i.image.part.to_ascii_lowercase();
            if part.ends_with(".emf") && !equations.contains(&i.image.part) {
                runs.extend(crate::office::pptx::slide::metafile_text_runs(
                    images,
                    doc,
                    fonts,
                    &i.image.part,
                    i.transform,
                    i.crop,
                    None,
                ));
            }
        }
    }
    let mut items = scene.items.clone();
    items.extend(runs.into_iter().map(crate::office::pptx::scene::Item::Text));
    Scene {
        width: scene.width,
        height: scene.height,
        items,
        ..Default::default()
    }
}

fn page_record(
    doc: &Document,
    fonts: &FontSystem,
    images: &ImageCache,
    layers: &crate::office::pptx::layers::LayerCache,
    cfg: &Config,
    page: u32,
    (p, bookmarks, equations): (
        &OutPage,
        &bookmarks::Index,
        &std::collections::HashSet<String>,
    ),
) -> (Value, Option<crate::office::pptx::output::PendingShot>) {
    let scene = scene_of(p);
    let frame = Rect {
        x: 0.0,
        y: 0.0,
        w: p.info.w,
        h: p.info.h,
    };
    // `DOCX_IMG_LOG=1`: one stderr line per drawn picture (page, part, box), for evaluation.
    if std::env::var("DOCX_IMG_LOG").is_ok_and(|v| v == "1") {
        for it in &scene.items {
            if let crate::office::pptx::scene::Item::Image(i) = it {
                let t = i.transform;
                eprintln!(
                    "IMG\t{page}\t{}\t{:.1}\t{:.1}\t{:.1}\t{:.1}",
                    i.image.part, t.tx, t.ty, t.sx, t.sy
                );
            }
        }
    }
    let mut rec = Map::new();
    rec.insert("page".into(), json!(page));
    rec.insert(
        "viewport".into(),
        json!({ "x": 0.0, "y": 0.0, "w": frame.w, "h": frame.h }),
    );
    if cfg.annotation.enabled {
        rec.insert("annotations".into(), Value::Array(Vec::new()));
    }
    if cfg.image.enabled {
        let (imgs, errors) =
            crate::office::pptx::output::images_json(doc, images, cfg, &scene, page, 0.0, 0.0);
        rec.insert("images".into(), Value::Array(imgs));
        if errors > 0 {
            rec.insert("imageErrorCount".into(), json!(errors));
        }
    }
    if cfg.path.enabled {
        let raw = crate::office::pptx::output::paths_json(
            &scene,
            cfg.path.max_per_page as usize,
            0.0,
            0.0,
        );
        let v: Value = serde_json::from_slice(&raw).unwrap_or(Value::Array(Vec::new()));
        rec.insert("paths".into(), v);
    }
    let mut pending = None;
    if cfg.screenshot.enabled {
        pending = crate::office::pptx::output::render_screenshot(
            doc, fonts, images, cfg, &scene, &frame, page, layers,
        );
        if pending.is_some() {
            rec.insert("screenshot".into(), Value::Null);
        }
    }
    if cfg.text.enabled || cfg.structtree.enabled {
        let scene = with_metafile_text(&scene, doc, images, fonts, equations);
        let mut text = dedupe_text(crate::office::pptx::output::source_text_json(
            fonts,
            &reading_scene(&scene),
            cfg.verbose,
            (0.0, 0.0),
            Some((frame.w, frame.h)),
        ));
        bookmarks.resolve_text(&mut text);
        if cfg.structtree.enabled {
            let mut tree = source::structure(&mut text);
            bookmarks.decorate(&mut tree, page);
            tree["sourcePart"] = json!(
                doc.pkg
                    .rel_by_kind("", "officeDocument")
                    .map(|rel| rel.target)
            );
            rec.insert("structtree".into(), tree);
        } else {
            // Text-only requests use the same deterministic native content IDs.
            source::structure(&mut text);
        }
        if cfg.text.enabled {
            rec.insert("text".into(), text);
        }
    }
    rec.insert(
        "document".into(),
        json!({ "section": p.info.section, "pageNumber": p.info.number }),
    );
    (Value::Object(rec), pending)
}

/// The page's text runs in reading order (stable sort on the reading key that
/// `paginate` stores in `GlyphRun::paragraph`); drawing keeps z order.
fn reading_scene(scene: &Scene) -> Scene {
    let mut items: Vec<crate::office::pptx::scene::Item> = scene
        .items
        .iter()
        .filter(|it| {
            matches!(
                it,
                crate::office::pptx::scene::Item::Text(_)
                    | crate::office::pptx::scene::Item::Bookmark(_)
            )
        })
        .cloned()
        .collect();
    {
        items.sort_by_key(|it| match it {
            crate::office::pptx::scene::Item::Text(r) => r.paragraph,
            crate::office::pptx::scene::Item::Bookmark(marker) => marker.reading_key,
            _ => 0,
        });
    }
    Scene {
        width: scene.width,
        height: scene.height,
        items,
        ..Default::default()
    }
}

/// Drops coincident text copies from distinct painted runs within half a point.
/// Logical ownership fragments of one painted run remain distinct, including
/// condensed punctuation. The page image keeps every painted copy.
fn dedupe_text(mut text: Value) -> Value {
    let Some(items) = text.get_mut("items").and_then(Value::as_array_mut) else {
        return text;
    };
    let key = |it: &Value| -> Option<(String, i64, i64)> {
        // Exported whitespace has an explicit source position, including coincident slots.
        if it.get("unicode")?.as_str()?.trim().is_empty() {
            return None;
        }
        let c = it.get("coords")?;
        let q = |k: &str| (c.get(k).and_then(Value::as_f64).unwrap_or(0.0) * 2.0).round() as i64;
        Some((it.get("unicode")?.as_str()?.to_string(), q("x"), q("y")))
    };
    let mut seen = std::collections::HashMap::new();
    let mut retained: Vec<Value> = Vec::with_capacity(items.len());
    let mut boundaries = Vec::with_capacity(items.len() + 1);
    for mut item in items.drain(..) {
        boundaries.push(retained.len());
        let paint_group = item
            .as_object_mut()
            .and_then(|item| item.remove("nativePaintGroup"))
            .and_then(|value| value.as_u64());
        if let Some(key) = key(&item) {
            if let Some((index, _)) = seen
                .get(&key)
                .copied()
                .filter(|(_, previous)| paint_group.is_none() || paint_group != *previous)
            {
                if let Some(markers) = item
                    .as_object_mut()
                    .and_then(|item| item.remove("nativeBookmarks"))
                {
                    let previous: &mut Value = &mut retained[index];
                    if previous.get("nativeBookmarks").is_none() {
                        previous["nativeBookmarks"] = json!([]);
                    }
                    previous["nativeBookmarks"]
                        .as_array_mut()
                        .unwrap()
                        .extend(markers.as_array().into_iter().flatten().cloned());
                }
                continue;
            }
            seen.entry(key).or_insert((retained.len(), paint_group));
        }
        retained.push(item);
    }
    boundaries.push(retained.len());
    *items = retained;
    if let Some(markers) = text
        .get_mut("nativeStandaloneBookmarks")
        .and_then(Value::as_array_mut)
    {
        for marker in markers {
            let old = marker["textItemBoundary"].as_u64().unwrap_or(0) as usize;
            marker["textItemBoundary"] = json!(boundaries[old.min(boundaries.len() - 1)]);
        }
    }
    text
}

fn render_pdf_pages(
    doc: &Document,
    fonts: &FontSystem,
    images: &ImageCache,
    cfg: &Config,
    laid: &Laid,
    selected: &[u32],
    out: &str,
) -> Result<Vec<crate::office::pptx::pdf::PdfPage>, String> {
    let dir = std::path::Path::new(out)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(format!(".docx-pdf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut pages = Vec::new();
    for page in selected {
        let p = &laid.pages[*page as usize];
        let scene = scene_of(p);
        let frame = Rect {
            x: 0.0,
            y: 0.0,
            w: p.info.w,
            h: p.info.h,
        };
        let scale = cfg.pdf.dpi.max(36.0) as f64 / 72.0;
        let target = RenderTarget {
            region: frame,
            width: (frame.w * scale).round() as u32,
            height: (frame.h * scale).round() as u32,
        };
        let pm = render::render(&scene, &target, doc, fonts, images).ok_or("render failed")?;
        let path = dir.join(format!("page_{}.jpg", page + 1));
        let bgr = render::to_bgr(&pm);
        crate::office::pptx::output::write_jpeg(
            &path,
            &bgr,
            pm.width() as usize,
            pm.height() as usize,
            cfg.pdf.dpi as i32,
            cfg.jpeg_quality(),
        );
        pages.push(crate::office::pptx::pdf::PdfPage {
            width_pt: frame.w,
            height_pt: frame.h,
            image_path: path.to_string_lossy().into_owned(),
        });
    }
    Ok(pages)
}

fn panic_message(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown".into())
}

fn error_index(e: &DocError) -> Extracted {
    Extracted {
        index: json!({ "error": e.code(), "errorDetail": e.to_string(), "totalPages": 0, "index": [] }),
        pages: Vec::new(),
    }
}

/// The file-protocol form of [`read`], for the tests: reads `input`, writes
/// the index and the page lines to the given paths.
#[cfg(test)]
pub fn run_read(
    config_json: &str,
    input: &str,
    index_path: &str,
    pages_path: &str,
) -> Result<(), String> {
    let bytes = std::fs::read(input).map_err(|e| format!("cannot read {input}: {e}"))?;
    let out = read(config_json, bytes)?;
    std::fs::write(pages_path, &out.pages)
        .map_err(|e| format!("cannot write {pages_path}: {e}"))?;
    let index = serde_json::to_vec(&out.index).map_err(|e| e.to_string())?;
    std::fs::write(index_path, index).map_err(|e| format!("cannot write {index_path}: {e}"))
}

mod parallel;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod endnote_tests;

#[cfg(test)]
mod picture_bullet_tests;
