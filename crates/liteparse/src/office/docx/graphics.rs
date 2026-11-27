//! Charts, chartEx and SmartArt in Word drawings, drawn by `pptx-read`'s
//! renderers: each graphic is wrapped in a one-frame synthetic slide whose part
//! is the Word part holding it (so its relationships resolve), and the
//! resulting scene becomes the picture's shape items, text included.

use std::cell::RefCell;
use std::rc::Rc;

use crate::office::pptx::document::{Document, SlideChain};
use crate::office::pptx::fonts::FontSystem;
use crate::office::pptx::render::ImageCache;
use crate::office::pptx::slide::SlideBuilder;
use crate::office::pptx::xml::XNode;

use crate::office::docx::model::{Block, Inline, Picture, Section};
use crate::office::docx::parse::EMU_PER_PT;
use crate::office::docx::shapes::ShapeSpec;

fn node(name: &str, attrs: &[(&str, String)], children: Vec<Rc<XNode>>) -> Rc<XNode> {
    Rc::new(XNode {
        name: name.into(),
        attrs: attrs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect(),
        children,
        text: String::new(),
    })
}

// Relationship IDs distinguish missing components within the same source part.
fn graphic_relationships(node: &XNode, out: &mut Vec<String>) {
    for (key, value) in &node.attrs {
        if key.starts_with("r:") {
            out.push(format!("{key}={value}"));
        }
    }
    for child in &node.children {
        graphic_relationships(child, out);
    }
}

/// Draws chart and diagram pictures of the document into their shapes.
pub struct Graphics<'a> {
    doc: Document,
    fonts: &'a FontSystem,
    images: ImageCache<'a>,
    cfg: crate::office::pptx::config::Config,
    theme_part: String,
    warnings: RefCell<Vec<(String, String)>>,
    picture_tables: RefCell<std::collections::HashSet<(String, usize)>>,
}

impl<'a> Graphics<'a> {
    pub fn new(
        pkg: std::sync::Arc<crate::office::pptx::package::Package>,
        main: &str,
        fonts: &'a FontSystem,
    ) -> Self {
        Self::with_image_limit(
            pkg,
            main,
            fonts,
            crate::office::pptx::config::Config::default()
                .limits
                .max_image_pixels,
        )
    }

    pub fn with_image_limit(
        pkg: std::sync::Arc<crate::office::pptx::package::Package>,
        main: &str,
        fonts: &'a FontSystem,
        max_image_pixels: u64,
    ) -> Self {
        let theme_part = pkg
            .rel_by_kind(main, "theme")
            .map(|r| r.target)
            .unwrap_or_default();
        let mut cfg = crate::office::pptx::config::Config::default();
        cfg.limits.max_image_pixels = max_image_pixels;
        Graphics {
            doc: Document::from_package(pkg, 914_400 * 8, 914_400 * 11),
            fonts,
            images: ImageCache::new(cfg.limits.max_image_pixels, fonts),
            cfg,
            theme_part,
            warnings: RefCell::new(Vec::new()),
            picture_tables: RefCell::new(Default::default()),
        }
    }

    /// The drawing of a `a:graphicData` (`uri` chart, chartex or diagram) of
    /// `w`×`h` points from `part`; `None` when nothing was drawn.
    pub fn render(&self, data: &Rc<XNode>, part: &str, w: f64, h: f64) -> Option<ShapeSpec> {
        let emu = |v: f64| ((v * EMU_PER_PT).round() as i64).to_string();
        let frame = node(
            "p:graphicFrame",
            &[],
            vec![
                node(
                    "p:nvGraphicFramePr",
                    &[],
                    vec![node(
                        "p:cNvPr",
                        &[("id", "2".into()), ("name", "Graphic".into())],
                        vec![],
                    )],
                ),
                node(
                    "p:xfrm",
                    &[],
                    vec![
                        node("a:off", &[("x", "0".into()), ("y", "0".into())], vec![]),
                        node("a:ext", &[("cx", emu(w)), ("cy", emu(h))], vec![]),
                    ],
                ),
                node("a:graphic", &[], vec![data.clone()]),
            ],
        );
        let slide = node(
            "p:sld",
            &[],
            vec![node(
                "p:cSld",
                &[],
                vec![node("p:spTree", &[], vec![frame])],
            )],
        );
        let theme = self.doc.theme(&self.theme_part);
        // Word's default colour mapping (w:clrSchemeMapping) is PowerPoint's.
        let map: Vec<(&str, String)> = [
            ("bg1", "lt1"),
            ("tx1", "dk1"),
            ("bg2", "lt2"),
            ("tx2", "dk2"),
            ("accent1", "accent1"),
            ("accent2", "accent2"),
            ("accent3", "accent3"),
            ("accent4", "accent4"),
            ("accent5", "accent5"),
            ("accent6", "accent6"),
            ("hlink", "hlink"),
            ("folHlink", "folHlink"),
        ]
        .iter()
        .map(|(k, v)| (*k, (*v).to_string()))
        .collect();
        let clr_map = node("p:clrMap", &map, vec![]);
        let chain = SlideChain {
            slide_part: part.to_string(),
            slide,
            layout_part: None,
            layout: None,
            master_part: None,
            master: None,
            colors: theme.colors.with_map(Some(&clr_map)),
            theme,
            theme_part: self.theme_part.clone(),
        };
        let scene =
            SlideBuilder::new(&self.doc, self.fonts, &self.images, &self.cfg, &chain, 1).build();
        if !scene.warnings.is_empty() {
            let mut refs = Vec::new();
            graphic_relationships(data, &mut refs);
            refs.sort();
            refs.dedup();
            let context = if refs.is_empty() {
                part.to_string()
            } else {
                format!("{part} [{}]", refs.join(", "))
            };
            self.warnings.borrow_mut().extend(
                scene
                    .warnings
                    .into_iter()
                    .map(|(code, detail)| (code, format!("{context}: {detail}"))),
            );
        }
        (!scene.items.is_empty()).then(|| ShapeSpec {
            items: scene.items,
            texts: Vec::new(),
            ..Default::default()
        })
    }

    /// Stable diagnostics from source graphics, independent of header traversal order.
    pub fn take_warnings(&self) -> Vec<(String, String)> {
        let mut warnings = std::mem::take(&mut *self.warnings.borrow_mut());
        warnings.sort();
        warnings.dedup();
        warnings
    }

    fn picture(&self, p: &mut Picture) {
        if let Some((data, part)) = p.graphic.take()
            && p.shape.is_none()
        {
            p.shape = self.render(&data, &part, p.w, p.h).map(|mut shape| {
                // Preserve direct-chart ownership after the pending graphic is consumed.
                // Grouped graphics keep their enclosing shape's placement policy.
                shape.direct_chart = data.attr("uri")
                    == Some("http://schemas.openxmlformats.org/drawingml/2006/chart");
                Rc::new(shape)
            });
        }
        if let Some(shape) = &mut p.shape
            && !shape.graphics.is_empty()
        {
            let shape = Rc::make_mut(shape);
            let pending = std::mem::take(&mut shape.graphics);
            let original = std::mem::take(&mut shape.items);
            let mut records = pending.into_iter().peekable();
            // Equal insertion indices retain XML order; existing shape items stay in place.
            for (index, item) in original
                .into_iter()
                .map(Some)
                .chain(std::iter::once(None))
                .enumerate()
            {
                while records.peek().is_some_and(|g| g.insertion == index) {
                    let graphic = records.next().unwrap();
                    if let Some(rendered) =
                        self.render(&graphic.data, &graphic.part, graphic.w, graphic.h)
                    {
                        shape
                            .items
                            .extend(rendered.items.into_iter().map(|mut item| {
                                crate::office::docx::layout::transform_item(
                                    &mut item,
                                    graphic.transform,
                                );
                                item
                            }));
                    }
                }
                if let Some(item) = item {
                    shape.items.push(item);
                }
            }
            debug_assert!(
                records.next().is_none(),
                "graphic insertion must reference an existing item boundary"
            );
        }
        {
            if let Some(shape) = &mut p.shape {
                for text in &mut Rc::make_mut(shape).texts {
                    self.blocks(&mut text.blocks);
                }
            }
        }
    }

    pub fn blocks(&self, blocks: &mut [Block]) {
        for b in blocks {
            match b {
                Block::Para(p) => {
                    self.picture_label(p);
                    for i in p.inlines.iter_mut() {
                        match i {
                            Inline::Picture(pic, _) => self.picture(pic),
                            Inline::Anchor(a) => self.picture(&mut a.pic),
                            _ => {}
                        }
                    }
                }
                Block::Table(t) => {
                    for r in t.rows.iter_mut() {
                        for c in r.cells.iter_mut() {
                            self.blocks(&mut c.blocks);
                        }
                    }
                }
            }
        }
    }

    fn picture_label(&self, paragraph: &mut crate::office::docx::model::Paragraph) {
        let Some(label) = &mut paragraph.num else {
            return;
        };
        if label.picture.is_none() {
            return;
        }
        let valid = label.picture.as_ref().is_some_and(|picture| {
            let height = label.style.size * 0.8;
            let width = picture.w * (height / picture.h);
            height.is_finite()
                && height > 0.0
                && width.is_finite()
                && width > 0.0
                && picture
                    .part
                    .as_ref()
                    .is_some_and(|part| self.images.get_part(&self.doc, part).is_some())
        });
        if !valid {
            label.picture = None;
        } else if label.picture_count > 0
            && let Some(table) = &label.picture_table
        {
            self.picture_tables.borrow_mut().insert(table.clone());
        }
    }

    /// Select picture labels only after the normal bounded image decoder accepts their assets.
    pub fn picture_labels(&self, blocks: &mut [Block]) {
        for block in blocks {
            match block {
                Block::Para(paragraph) => {
                    self.picture_label(paragraph);
                    for inline in &mut paragraph.inlines {
                        let picture = match inline {
                            Inline::Picture(picture, _) => picture,
                            Inline::Anchor(anchor) => &mut anchor.pic,
                            _ => continue,
                        };
                        if let Some(shape) = &mut picture.shape {
                            for text in &mut Rc::make_mut(shape).texts {
                                self.picture_labels(&mut text.blocks);
                            }
                        }
                    }
                }
                Block::Table(table) => {
                    for row in &mut table.rows {
                        for cell in &mut row.cells {
                            self.picture_labels(&mut cell.blocks);
                        }
                    }
                }
            }
        }
    }

    /// Apply validated image-table eligibility to accepted source metadata after layout.
    pub fn mark_picture_tables(&self, pages: &mut [crate::office::docx::paginate::OutPage]) {
        let tables = self.picture_tables.borrow();
        if tables.is_empty() {
            return;
        }
        let update =
            |source: &mut Option<std::sync::Arc<crate::office::pptx::scene::TextSource>>| {
                let Some(source) = source else {
                    return;
                };
                if !source
                    .cells
                    .iter()
                    .any(|cell| tables.contains(&(source.part.clone(), cell.table)))
                {
                    return;
                }
                let source = std::sync::Arc::make_mut(source);
                for cell in &mut source.cells {
                    if tables.contains(&(source.part.clone(), cell.table)) {
                        cell.text_only = false;
                        if let Some(fragment) = &mut cell.fragment {
                            fragment.text_only = false;
                        }
                    }
                }
            };
        for page in pages {
            for item in &mut page.items {
                if let crate::office::pptx::scene::Item::Text(run) = item {
                    update(&mut run.source);
                    if let Some(bookmarks) = &mut run.bookmarks {
                        for span in &mut bookmarks.source_spans {
                            update(&mut span.source);
                        }
                    }
                }
            }
        }
    }

    pub fn sections(&self, sections: &mut [Section]) {
        for s in sections {
            self.blocks(&mut s.blocks);
        }
    }
}

/// Whether blocks require deferred graphics rendering or picture-label validation.
pub fn any(blocks: &[Block]) -> bool {
    blocks.iter().any(|b| match b {
        Block::Para(p) => {
            p.num.as_ref().is_some_and(|label| label.picture.is_some())
                || p.inlines.iter().any(|i| match i {
                    Inline::Picture(pic, _) => picture_has_graphic(pic),
                    Inline::Anchor(a) => picture_has_graphic(&a.pic),
                    _ => false,
                })
        }
        Block::Table(t) => t
            .rows
            .iter()
            .any(|r| r.cells.iter().any(|c| any(&c.blocks))),
    })
}

pub fn has_picture_labels(blocks: &[Block]) -> bool {
    blocks.iter().any(|block| match block {
        Block::Para(paragraph) => {
            paragraph
                .num
                .as_ref()
                .is_some_and(|label| label.picture.is_some())
                || paragraph.inlines.iter().any(|inline| {
                    let picture = match inline {
                        Inline::Picture(picture, _) => picture,
                        Inline::Anchor(anchor) => &anchor.pic,
                        _ => return false,
                    };
                    picture.shape.as_ref().is_some_and(|shape| {
                        shape
                            .texts
                            .iter()
                            .any(|text| has_picture_labels(&text.blocks))
                    })
                })
        }
        Block::Table(table) => table.rows.iter().any(|row| {
            row.cells
                .iter()
                .any(|cell| has_picture_labels(&cell.blocks))
        }),
    })
}

// Nested shape text retains deferred graphics until fonts are available.
// Detection and rendering traverse the same blocks so nested-only documents run the pass.
fn picture_has_graphic(picture: &Picture) -> bool {
    picture.graphic.is_some()
        || picture
            .shape
            .as_ref()
            .is_some_and(|s| !s.graphics.is_empty())
        || picture
            .shape
            .as_ref()
            .is_some_and(|shape| shape.texts.iter().any(|text| any(&text.blocks)))
}

#[cfg(test)]
mod tests;
