use super::*;
use unicode_bidi::{BidiClass, BidiInfo, Level};

/// Analysis uses separate byte offsets so Word's neutral-character overrides
/// leave the logical document text and shaping input unchanged.
pub(super) struct Analysis {
    text: String,
    original: String,
    units: Vec<(char, bool)>,
    source_to_analysis: Vec<usize>,
}

impl Analysis {
    pub(super) fn label(text: &str, style: &Rc<RunStyle>) -> Self {
        let mut out = Self {
            text: String::new(),
            original: String::new(),
            units: Vec::new(),
            source_to_analysis: Vec::new(),
        };
        out.push(text, Some(style));
        out.finish();
        out
    }

    pub(super) fn new(p: &Paragraph, env: &TextEnv) -> Self {
        let mut out = Self {
            text: String::new(),
            original: String::new(),
            units: Vec::new(),
            source_to_analysis: Vec::new(),
        };
        for inline in &p.inlines {
            match inline {
                Inline::Bookmark(_) => {}
                Inline::Text(t, s) => out.push(t, Some(s)),
                Inline::Ruby(r) => {
                    for (t, s) in &r.base {
                        out.push(t, Some(s));
                    }
                }
                Inline::Field(k, s) => {
                    let t = match k {
                        FieldKind::Page(None) => env.fields.page.clone(),
                        FieldKind::Page(Some(f)) => {
                            crate::office::docx::numbering::format_number(f, env.fields.page_no)
                        }
                        FieldKind::NumPages(f) => crate::office::docx::numbering::format_number(
                            f.unwrap_or("decimal"),
                            env.fields.num_pages as i64,
                        ),
                        FieldKind::SectionPages(f) => {
                            crate::office::docx::numbering::format_number(
                                f.unwrap_or("decimal"),
                                env.fields.section_pages as i64,
                            )
                        }
                        FieldKind::StyleRef(i) => super::style_ref_text(*i),
                    };
                    out.push(&t, Some(s));
                }
                Inline::Tab(_) => out.push("\t", None),
                Inline::Break(_, _) => out.push("\n", None),
                Inline::Picture(_, _) | Inline::ContinuationSeparator(_) => {
                    out.push("\u{fffc}", None)
                }
                Inline::Anchor(_) | Inline::Note(_) => out.push("\u{200b}", None),
            }
        }
        out.finish();
        out
    }

    fn push(&mut self, text: &str, style: Option<&Rc<RunStyle>>) {
        let text = if style.is_some_and(|s| s.caps && s.symbol.is_none()) {
            text.to_uppercase()
        } else {
            text.into()
        };
        let override_r = style.is_some_and(|s| s.rtl == Some(true));
        self.original.push_str(&text);
        self.units.extend(text.chars().map(|ch| (ch, override_r)));
    }

    fn finish(&mut self) {
        let hebrew = self
            .units
            .iter()
            .any(|(ch, _)| ('\u{0590}'..='\u{05ff}').contains(ch));
        for (i, &(source, rtl)) in self.units.iter().enumerate() {
            self.source_to_analysis
                .extend(std::iter::repeat_n(self.text.len(), source.len_utf8()));
            // A Word line break retains paragraph context. The line separator
            // gets L1 whitespace treatment without starting a bidi paragraph.
            let ch = if source == '\n' { '\u{2028}' } else { source };
            let class = unicode_bidi::bidi_class(ch);
            let numeric = |j: usize| {
                self.units.get(j).is_some_and(|(c, _)| {
                    matches!(unicode_bidi::bidi_class(*c), BidiClass::EN | BidiClass::AN)
                })
            };
            let numeric_es = hebrew && i > 0 && numeric(i - 1) && numeric(i + 1);
            // A separator in a right-to-left run reads right to left too: Word shows
            // "02" rtl"/" "03" rtl"/" "2025" in an Arabic paragraph as 2025/03/02.
            let override_r = rtl
                && (matches!(class, BidiClass::WS | BidiClass::ON | BidiClass::NSM)
                    || (matches!(class, BidiClass::ES | BidiClass::CS) && !numeric_es));
            self.text.push(if override_r { '\u{200f}' } else { ch });
        }
        self.source_to_analysis.push(self.text.len());
    }

    pub(super) fn resolve(&self, rtl: bool) -> Context<'_> {
        Context {
            info: BidiInfo::new(
                &self.text,
                Some(if rtl { Level::rtl() } else { Level::ltr() }),
            ),
            offsets: &self.source_to_analysis,
            original: &self.original,
        }
    }
}

pub(super) struct Context<'a> {
    info: BidiInfo<'a>,
    offsets: &'a [usize],
    original: &'a str,
}

impl Context<'_> {
    fn offset(&self, source: usize) -> usize {
        self.offsets
            .get(source)
            .copied()
            .unwrap_or(self.info.text.len())
    }

    pub(super) fn level(&self, source: usize) -> Level {
        self.info
            .levels
            .get(self.offset(source))
            .copied()
            .unwrap_or(Level::ltr())
    }

    pub(super) fn place(&self, b: &mut LineBuild) {
        let Some(first) = b.atoms.iter().position(|a| a.bidi_range.is_some()) else {
            return;
        };
        let last = b
            .atoms
            .iter()
            .rposition(|a| a.bidi_range.is_some())
            .unwrap()
            + 1;
        let start = self.offset(b.atoms[first].bidi_range.as_ref().unwrap().start);
        let end = self.offset(b.atoms[last - 1].bidi_range.as_ref().unwrap().end);
        let Some(para) = self
            .info
            .paragraphs
            .iter()
            .find(|p| p.range.start <= start && start < p.range.end)
        else {
            return;
        };
        if end <= start || end > para.range.end {
            return;
        }
        if self.info.levels[start..end].iter().all(|l| l.is_ltr()) {
            return;
        }
        let reset = self.info.reordered_levels(para, start..end);
        let mut levels: Vec<_> = b.atoms[first..last]
            .iter()
            .map(|a| {
                a.bidi_range
                    .as_ref()
                    .map(|r| reset[self.offset(r.start)])
                    .unwrap_or(para.level)
            })
            .collect();
        // L1 uses original character classes, including whitespace whose run
        // direction was represented by a strong mark during analysis.
        let mut trailing = true;
        for i in (first..last).rev() {
            let atom = &b.atoms[i];
            let source = atom
                .bidi_range
                .as_ref()
                .and_then(|r| self.original.get(r.clone()))
                .unwrap_or(&atom.text);
            let separator = source
                .chars()
                .any(|c| matches!(unicode_bidi::bidi_class(c), BidiClass::B | BidiClass::S));
            let whitespace = !source.is_empty()
                && source.chars().all(|c| {
                    matches!(
                        unicode_bidi::bidi_class(c),
                        BidiClass::WS
                            | BidiClass::FSI
                            | BidiClass::LRI
                            | BidiClass::RLI
                            | BidiClass::PDI
                    )
                });
            if separator || (trailing && whitespace) {
                levels[i - first] = para.level;
                trailing = true;
            } else {
                trailing = false;
            }
        }
        for (atom, level) in b.atoms[first..last].iter_mut().zip(&levels) {
            atom.bidi_level = level.number();
        }
        if levels.iter().all(|l| l.is_ltr()) {
            return;
        }
        let order = BidiInfo::reorder_visual(&levels);
        // Gaps include distributed justification. Atom and glyph internals stay
        // in logical cluster order; only each cluster's physical origin changes.
        let advances: Vec<_> = (first..last)
            .map(|i| {
                if i + 1 < last {
                    b.xs[i + 1] - b.xs[i]
                } else {
                    b.atoms[i].width
                }
            })
            .collect();
        let mut x = b.xs[first];
        // L1 trailing whitespace occupies the visual left edge in RTL lines.
        // Tab advances belong to that whitespace when keeping body ink in place.
        if para.level.is_rtl() {
            x -= b.atoms[first..last]
                .iter()
                .enumerate()
                .rev()
                .take_while(|(_, a)| matches!(a.kind, Kind::Space | Kind::Tab | Kind::Break(_)))
                .map(|(i, _)| advances[i])
                .sum::<f64>();
        }
        for i in order {
            b.xs[first + i] = x;
            b.atoms[first + i].bidi_level = levels[i].number();
            x += advances[i];
        }
    }
}

pub(super) fn shape(
    text: &str,
    style: &Rc<RunStyle>,
    env: &TextEnv,
    atoms: &mut Vec<Atom>,
    full: &mut String,
    offsets: &mut Vec<usize>,
    context: Option<&Context<'_>>,
) {
    let Some(ctx) = context else {
        shape_text(text, style, env, atoms, full, offsets);
        return;
    };
    let text = if style.caps && style.symbol.is_none() {
        text.to_uppercase()
    } else {
        text.into()
    };
    let base = full.len();
    let mut from = 0;
    while from < text.len() {
        let level = ctx.level(base + from);
        let end = text[from..]
            .char_indices()
            .skip(1)
            .find(|(i, _)| ctx.level(base + from + i) != level)
            .map(|(i, _)| from + i)
            .unwrap_or(text.len());
        shape_direction(
            &text[from..end],
            style,
            env,
            atoms,
            full,
            offsets,
            ShapeContext {
                direction: Some(if level.is_rtl() {
                    rustybuzz::Direction::RightToLeft
                } else {
                    rustybuzz::Direction::LeftToRight
                }),
                context: Some((ctx.original, base + from)),
            },
        );
        from = end;
    }
}

#[cfg(test)]
mod tests;
