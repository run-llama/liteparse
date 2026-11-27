//! The presentation as a whole: the slide list (with hidden flags and section
//! names), slide size, document metadata, and cached access to every slide's
//! layout → master → theme chain.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use crate::office::pptx::color::ColorScheme;
use crate::office::pptx::config::Limits;
use crate::office::pptx::error::{DocError, DocResult};
use crate::office::pptx::package::Package;
use crate::office::pptx::theme::Theme;
use crate::office::pptx::xml::{self, XNode};

pub const EMU_PER_PT: f64 = 12_700.0;

/// Folders of parts that belong to one slide; their trees are dropped after it.
pub const SLIDE_SCOPED: [&str; 5] = [
    "ppt/slides/",
    "ppt/charts/",
    "ppt/diagrams/",
    "ppt/notesSlides/",
    "ppt/drawings/",
];

#[derive(Debug, Clone)]
pub struct SlideRef {
    /// 0-based position in presentation order (hidden slides included).
    pub index: u32,
    pub slide_id: u32,
    pub part: String,
    pub hidden: bool,
    pub section_name: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
}

/// One slide with everything it inherits from.
pub struct SlideChain {
    pub slide_part: String,
    pub slide: Rc<XNode>,
    pub layout_part: Option<String>,
    pub layout: Option<Rc<XNode>>,
    pub master_part: Option<String>,
    pub master: Option<Rc<XNode>>,
    pub theme: Rc<Theme>,
    /// Theme part; picture fills in the theme's style lists resolve against it.
    pub theme_part: String,
    /// Theme colors with the effective `clrMap` (master, overridden by layout and slide).
    pub colors: ColorScheme,
}

pub struct Document {
    pub pkg: Arc<Package>,
    pub pres_part: String,
    pub pres: Rc<XNode>,
    /// Slide size in EMU.
    pub slide_w: i64,
    pub slide_h: i64,
    pub slides: Vec<SlideRef>,
    pub meta: DocMeta,
    parts: RefCell<HashMap<String, Option<Rc<XNode>>>>,
    themes: RefCell<HashMap<String, Rc<Theme>>>,
}

/// A [`Document`] without its thread-bound tree caches; see [`Document::seed`].
pub struct DocumentSeed {
    pkg: Arc<Package>,
    pres_part: String,
    slide_w: i64,
    slide_h: i64,
    slides: Vec<SlideRef>,
    meta: DocMeta,
}

impl DocumentSeed {
    /// Opens the document on the calling thread (re-parses `presentation.xml`).
    pub fn open(self) -> DocResult<Document> {
        let text = self
            .pkg
            .read_string(&self.pres_part)
            .map_err(|_| DocError::Broken("no presentation part".into()))?;
        let pres =
            xml::parse(&text).map_err(|e| DocError::Broken(format!("presentation.xml: {e}")))?;
        Ok(Document {
            pkg: self.pkg,
            pres_part: self.pres_part,
            pres,
            slide_w: self.slide_w,
            slide_h: self.slide_h,
            slides: self.slides,
            meta: self.meta,
            parts: RefCell::new(HashMap::new()),
            themes: RefCell::new(HashMap::new()),
        })
    }
}

/// Signature of an OLE compound file: binary PowerPoint (.ppt), or an encrypted .pptx.
const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// A binary PowerPoint file (a compound file with a `PowerPoint Document`
/// stream) becomes a .pptx through office::ppt; any other input is returned unchanged.
fn pptx_bytes(bytes: Vec<u8>) -> DocResult<Vec<u8>> {
    if !bytes.starts_with(&CFB_MAGIC) {
        return Ok(bytes);
    }
    let is_ppt = crate::office::doc::compound::Compound::open(&bytes)
        .is_ok_and(|c| c.stream("PowerPoint Document").is_ok());
    if !is_ppt {
        return Ok(bytes);
    }
    match crate::office::ppt::convert(&bytes) {
        Ok(converted) => Ok(converted.pptx),
        Err(crate::office::ppt::Error::Encrypted) => Err(DocError::Protected),
        Err(e) => Err(DocError::Broken(format!("binary PowerPoint: {e}"))),
    }
}

impl Document {
    pub fn open(bytes: Vec<u8>, limits: &Limits) -> DocResult<Document> {
        let pkg = Arc::new(Package::open(pptx_bytes(bytes)?, limits)?);
        let pres_part = pkg
            .rel_by_kind("", "officeDocument")
            .map(|r| r.target)
            .filter(|p| pkg.exists(p))
            .unwrap_or_else(|| "ppt/presentation.xml".to_string());
        let pres_xml = pkg
            .read_string(&pres_part)
            .map_err(|_| DocError::Broken("no presentation part".into()))?;
        let pres = xml::parse(&pres_xml)
            .map_err(|e| DocError::Broken(format!("presentation.xml: {e}")))?;
        if pres.name != "p:presentation" {
            return Err(DocError::Broken(format!(
                "root element {} is not a presentation",
                pres.name
            )));
        }
        let (slide_w, slide_h) = pres
            .child("p:sldSz")
            .map(|s| {
                (
                    s.attr_i64("cx").unwrap_or(9_144_000),
                    s.attr_i64("cy").unwrap_or(6_858_000),
                )
            })
            .unwrap_or((9_144_000, 6_858_000));
        let slide_w = if slide_w > 0 { slide_w } else { 9_144_000 };
        let slide_h = if slide_h > 0 { slide_h } else { 6_858_000 };

        let sections = section_map(&pres);
        let mut slides = Vec::new();
        if let Some(list) = pres.child("p:sldIdLst") {
            for entry in list.children_named("p:sldId") {
                let Some(rid) = entry.attr("r:id") else {
                    continue;
                };
                let Some(rel) = pkg.rel_by_id(&pres_part, rid) else {
                    continue;
                };
                let slide_id = entry.attr_i64("id").unwrap_or(0) as u32;
                slides.push(SlideRef {
                    index: slides.len() as u32,
                    slide_id,
                    part: rel.target,
                    hidden: false,
                    section_name: sections.get(&slide_id).cloned(),
                });
            }
        }
        if slides.len() > limits.max_slides as usize {
            return Err(DocError::LimitExceeded(format!("{} slides", slides.len())));
        }
        let doc = Document {
            meta: read_meta(&pkg),
            pkg,
            pres_part,
            pres,
            slide_w,
            slide_h,
            slides,
            parts: RefCell::new(HashMap::new()),
            themes: RefCell::new(HashMap::new()),
        };
        // The hidden flag lives on the slide root (`<p:sld show="0">`). Read it without
        // building or caching the tree: slide trees are built one page at a time.
        let mut slides = doc.slides.clone();
        for s in &mut slides {
            if let Ok(text) = doc.pkg.read_string(&s.part) {
                s.hidden = xml::root_attr(&text, "show")
                    .as_deref()
                    .and_then(xml::parse_bool)
                    == Some(false);
            }
        }
        Ok(Document { slides, ..doc })
    }

    /// A document shell around an already opened package, for decoders that
    /// reuse the renderer and outputs (which read only `pkg` and the size): no
    /// slides, an empty presentation node, metadata from `docProps`.
    pub fn from_package(pkg: Arc<Package>, w_emu: i64, h_emu: i64) -> Document {
        Document {
            meta: read_meta(&pkg),
            pkg,
            pres_part: String::new(),
            pres: Rc::new(XNode {
                name: "p:presentation".into(),
                attrs: Vec::new(),
                children: Vec::new(),
                text: String::new(),
            }),
            slide_w: w_emu,
            slide_h: h_emu,
            slides: Vec::new(),
            parts: RefCell::new(HashMap::new()),
            themes: RefCell::new(HashMap::new()),
        }
    }

    pub fn slide_w_pt(&self) -> f64 {
        self.slide_w as f64 / EMU_PER_PT
    }

    pub fn slide_h_pt(&self) -> f64 {
        self.slide_h as f64 / EMU_PER_PT
    }

    /// Parsed XML for a part, cached. `None` when missing or malformed.
    pub fn xml(&self, part: &str) -> Option<Rc<XNode>> {
        if let Some(hit) = self.parts.borrow().get(part) {
            return hit.clone();
        }
        let parsed = self
            .pkg
            .read_string(part)
            .ok()
            .and_then(|s| xml::parse(&s).ok());
        self.parts
            .borrow_mut()
            .insert(part.to_string(), parsed.clone());
        parsed
    }

    /// Drops cached trees of parts that belong to one slide (the slide itself, its
    /// charts, diagrams, notes and VML drawings), so a deck's slide XML is never all
    /// resident at once. Layouts, masters and themes stay; a dropped part is re-read
    /// if asked for again.
    pub fn release_slide_parts(&self) {
        self.parts
            .borrow_mut()
            .retain(|k, _| !SLIDE_SCOPED.iter().any(|p| k.starts_with(p)));
    }

    /// What another thread needs to open its own view of this document: the
    /// package is shared, the parsed trees and themes are per thread.
    pub fn seed(&self) -> DocumentSeed {
        DocumentSeed {
            pkg: self.pkg.clone(),
            pres_part: self.pres_part.clone(),
            slide_w: self.slide_w,
            slide_h: self.slide_h,
            slides: self.slides.clone(),
            meta: self.meta.clone(),
        }
    }

    pub fn theme(&self, part: &str) -> Rc<Theme> {
        if let Some(t) = self.themes.borrow().get(part) {
            return t.clone();
        }
        let theme = Rc::new(self.xml(part).map(|x| Theme::parse(&x)).unwrap_or_default());
        self.themes
            .borrow_mut()
            .insert(part.to_string(), theme.clone());
        theme
    }

    pub fn default_text_style(&self) -> Option<&Rc<XNode>> {
        self.pres.child("p:defaultTextStyle")
    }

    pub fn chain(&self, slide_part: &str) -> DocResult<SlideChain> {
        let slide = self
            .xml(slide_part)
            .ok_or_else(|| DocError::MissingPart(slide_part.to_string()))?;
        self.chain_for(slide_part, slide)
    }

    /// Builds the inheritance chain for any slide-like part (slide, layout,
    /// master, notes slide).
    pub fn chain_for(&self, slide_part: &str, slide: Rc<XNode>) -> DocResult<SlideChain> {
        let (layout_part, master_part) = match slide.name.as_str() {
            "p:sldMaster" => (None, Some(slide_part.to_string())),
            "p:sldLayout" => (
                Some(slide_part.to_string()),
                self.pkg
                    .rel_by_kind(slide_part, "slideMaster")
                    .map(|r| r.target),
            ),
            _ => {
                let layout = self
                    .pkg
                    .rel_by_kind(slide_part, "slideLayout")
                    .map(|r| r.target);
                let master = layout
                    .as_deref()
                    .and_then(|l| self.pkg.rel_by_kind(l, "slideMaster"))
                    .map(|r| r.target);
                (layout, master)
            }
        };
        let layout = layout_part.as_deref().and_then(|p| self.xml(p));
        let master = master_part.as_deref().and_then(|p| self.xml(p));
        let theme_part = master_part
            .as_deref()
            .and_then(|m| self.pkg.rel_by_kind(m, "theme"))
            .map(|r| r.target)
            .or_else(|| {
                self.pkg
                    .rel_by_kind(&self.pres_part, "theme")
                    .map(|r| r.target)
            });
        let theme = theme_part
            .as_deref()
            .map(|t| self.theme(t))
            .unwrap_or_default();
        let theme_part = theme_part.unwrap_or_default();

        let mut colors = theme.colors.with_map(
            master
                .as_ref()
                .and_then(|m| m.child("p:clrMap"))
                .map(|n| n.as_ref()),
        );
        for part in [layout.as_ref(), Some(&slide)].into_iter().flatten() {
            if let Some(ovr) = part.path(&["p:clrMapOvr", "a:overrideClrMapping"]) {
                colors = colors.with_map(Some(ovr));
            }
        }
        Ok(SlideChain {
            slide_part: slide_part.to_string(),
            slide,
            layout_part,
            layout,
            master_part,
            master,
            theme,
            theme_part,
            colors,
        })
    }
}

/// Slide id → section name from `p14:sectionLst` (inside the presentation extLst).
fn section_map(pres: &XNode) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let Some(list) = pres.find("p14:sectionLst") else {
        return out;
    };
    for section in list.children_named("p14:section") {
        let name = section.attr("name").unwrap_or("").to_string();
        if let Some(ids) = section.child("p14:sldIdLst") {
            for id in ids.children_named("p14:sldId") {
                if let Some(v) = id.attr_i64("id") {
                    out.insert(v as u32, name.clone());
                }
            }
        }
    }
    out
}

fn read_meta(pkg: &Package) -> DocMeta {
    let mut meta = DocMeta::default();
    let text = |root: &XNode, name: &str| {
        root.child(name)
            .map(|n| n.text.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    if let Some(core) = pkg
        .read_string("docProps/core.xml")
        .ok()
        .and_then(|s| xml::parse(&s).ok())
    {
        meta.title = text(&core, "dc:title");
        meta.author = text(&core, "dc:creator");
        meta.created = text(&core, "dcterms:created");
        meta.modified = text(&core, "dcterms:modified");
    }
    if let Some(app) = pkg
        .read_string("docProps/app.xml")
        .ok()
        .and_then(|s| xml::parse(&s).ok())
    {
        meta.app = text(&app, "ep:Application");
        meta.app_version = text(&app, "ep:AppVersion");
    }
    meta
}
