//! Lower supported HTML content into retained Word nodes for the shared source/layout paths.
use super::super::*;
use super::{dom, style, word};
use base64::Engine;
use std::collections::BTreeSet;

pub(super) struct Output {
    pub nodes: Vec<Rc<XNode>>,
    pub warnings: BTreeSet<String>,
}

#[derive(Default)]
struct InlineState {
    pending: bool,
    content: bool,
    pending_style: Option<style::Computed>,
    last_space: bool,
}

struct Lower<'a, 'b, 'c> {
    parser: &'a Parser<'b>,
    part: &'a str,
    host: &'a str,
    cascade: style::Cascade<'c>,
    remaining: usize,
    warnings: BTreeSet<String>,
    list: Vec<(bool, i64)>,
}

pub(super) fn lower(
    parser: &Parser<'_>,
    root: &dom::Handle,
    part: &str,
    host: &str,
    limit: usize,
) -> Result<Output, &'static str> {
    fn styles(node: &dom::Handle, out: &mut String) {
        if node.tag() == "style" && &*node.name.ns == "http://www.w3.org/1999/xhtml" {
            for child in node.children.borrow().iter() {
                out.push_str(&child.text.borrow());
            }
            out.push('\n');
        } else if node.tag() != "template" {
            for child in node.children.borrow().iter() {
                styles(child, out);
            }
        }
    }
    let mut css = String::new();
    styles(root, &mut css);
    let mut lower = Lower {
        parser,
        part,
        host,
        cascade: style::Cascade::new(&css),
        remaining: limit,
        warnings: BTreeSet::new(),
        list: Vec::new(),
    };
    let mut initial = style::Computed::default();
    initial.0.insert("font-size".into(), "12pt".into());
    initial
        .0
        .insert("font-family".into(), "Times New Roman".into());
    let nodes = lower.blocks(root, &initial)?;
    lower.warnings.extend(lower.cascade.warnings);
    Ok(Output {
        nodes,
        warnings: lower.warnings,
    })
}

fn block(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "main"
            | "aside"
            | "blockquote"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "pre"
            | "li"
            | "ul"
            | "ol"
            | "table"
            | "hr"
    )
}

impl Lower<'_, '_, '_> {
    fn charge(&mut self, amount: usize) -> Result<(), &'static str> {
        self.remaining = self
            .remaining
            .checked_sub(amount)
            .ok_or("HTML projection budget exceeded")?;
        Ok(())
    }
    fn computed(
        &mut self,
        node: &dom::Handle,
        parent: &style::Computed,
    ) -> Result<Option<style::Computed>, &'static str> {
        self.charge(32)?;
        if node.tag().starts_with('#') {
            return Ok(Some(parent.clone()));
        }
        if &*node.name.ns != "http://www.w3.org/1999/xhtml" {
            self.warnings
                .insert("unsupported foreign HTML content".into());
            return Ok(None);
        }
        if matches!(
            node.tag(),
            "script" | "style" | "template" | "head" | "meta" | "title" | "link" | "base"
        ) {
            if matches!(node.tag(), "link" | "base") {
                self.warnings
                    .insert("external stylesheet or base URL is unsupported".into());
            }
            return Ok(None);
        }
        let computed = self.cascade.compute(node, parent)?;
        if computed.get("border-collapse").is_some() {
            self.warnings
                .insert("CSS border-collapse negotiation is unsupported".into());
        }
        if computed.get("vertical-align").is_some() && !matches!(node.tag(), "td" | "th") {
            self.warnings
                .insert("CSS vertical-align outside table cells is unsupported".into());
        }

        if computed.get("display") == Some("none") || node.attr("hidden").is_some() {
            return Ok(None);
        }
        if let Some(display) = computed.get("display")
            && !matches!(
                display,
                "block"
                    | "inline"
                    | "table"
                    | "table-row"
                    | "table-cell"
                    | "list-item"
                    | "inline-block"
            )
        {
            self.warnings
                .insert(format!("unsupported CSS display {display}"));
        }
        Ok(Some(computed))
    }

    fn blocks(
        &mut self,
        container: &dom::Handle,
        style: &style::Computed,
    ) -> Result<Vec<Rc<XNode>>, &'static str> {
        let mut result = Vec::new();
        let mut runs = Vec::new();
        let mut space = InlineState::default();
        for child in container.children.borrow().iter() {
            let Some(computed) = self.computed(child, style)? else {
                continue;
            };
            if computed.get("display").map_or(block(child.tag()), |v| {
                matches!(v, "block" | "table" | "list-item")
            }) {
                if !runs.is_empty() {
                    result.push(word::paragraph(style, std::mem::take(&mut runs)));
                }
                space = InlineState::default();
                match child.tag() {
                    "table" => result.push(self.table(child, &computed)?),
                    "ul" | "ol" => {
                        let start = child
                            .attr("start")
                            .and_then(|n| n.parse().ok())
                            .unwrap_or(1);
                        self.list.push((child.tag() == "ol", start));
                        result.extend(self.blocks(child, &computed)?);
                        self.list.pop();
                    }
                    "li" => {
                        let mut children = self.blocks(child, &computed)?;
                        if children.is_empty() {
                            children.push(word::paragraph(&computed, Vec::new()));
                        }
                        if let Some((ordered, index)) = self.list.last_mut() {
                            if let Some(value) =
                                child.attr("value").and_then(|v| v.parse::<i64>().ok())
                            {
                                *index = value;
                            }
                            let marker = match computed.get("list-style-type") {
                                Some("none") => String::new(),
                                Some("disc") => "• ".into(),
                                Some("decimal") => format!("{}. ", *index),
                                _ if *ordered => format!("{}. ", *index),
                                _ => "• ".into(),
                            };
                            *index = index.saturating_add(1);
                            if children[0].name == "w:p" {
                                Rc::make_mut(&mut children[0])
                                    .children
                                    .insert(1, word::text(marker, &computed));
                            }
                        }
                        result.extend(children);
                    }
                    "hr" => {
                        self.warnings
                            .insert("horizontal rules are unsupported".into());
                    }
                    _ => {
                        let children = self.blocks(child, &computed)?;
                        if children.is_empty() {
                            result.push(word::paragraph(&computed, Vec::new()));
                        } else {
                            result.extend(children);
                        }
                    }
                }
            } else if matches!(child.tag(), "html" | "body") {
                result.extend(self.blocks(child, &computed)?);
            } else {
                self.inline(child, &computed, &mut runs, &mut space)?;
            }
        }
        if !runs.is_empty() {
            result.push(word::paragraph(style, runs));
        }
        Ok(result)
    }

    fn inline(
        &mut self,
        node: &dom::Handle,
        style: &style::Computed,
        out: &mut Vec<Rc<XNode>>,
        space: &mut InlineState,
    ) -> Result<(), &'static str> {
        if node.tag() == "#text" {
            let source = node.text.borrow();
            let preserve = matches!(
                style.get("white-space"),
                Some("pre" | "pre-wrap" | "break-spaces")
            );
            let mut text = String::new();
            for c in source.chars() {
                if preserve && matches!(c, '\n' | '\r' | '\t') {
                    if !text.is_empty() {
                        self.charge(32)?;
                        out.push(word::text(std::mem::take(&mut text), style));
                    }
                    self.charge(3)?;
                    out.push(word::element(
                        "w:r",
                        &[],
                        vec![word::element(
                            if c == '\t' { "w:tab" } else { "w:br" },
                            &[],
                            Vec::new(),
                        )],
                    ));
                    space.pending = false;
                } else if !preserve && matches!(c, ' ' | '\n' | '\r' | '\t' | '\u{c}') {
                    space.pending = true;
                    space.pending_style = Some(style.clone());
                } else {
                    if space.pending && !space.last_space && (space.content || !text.is_empty()) {
                        text.push(' ');
                    }
                    space.pending = false;
                    text.push(c);
                    space.content = true;
                    space.last_space = false;
                }
            }
            if !text.is_empty() {
                self.charge(32)?;
                out.push(word::text(text, style));
            }
            return Ok(());
        }
        if space.pending
            && !space.last_space
            && space.content
            && !matches!(node.tag(), "#comment" | "br")
        {
            self.charge(32)?;
            out.push(word::text(
                " ".into(),
                space.pending_style.as_ref().unwrap_or(style),
            ));
            space.pending = false;
            space.last_space = true;
        }
        match node.tag() {
            "#comment" => return Ok(()),
            "br" => {
                out.push(word::element(
                    "w:r",
                    &[],
                    vec![word::element("w:br", &[], Vec::new())],
                ));
                space.pending = false;
                space.content = false;
                return Ok(());
            }
            "img" => {
                if space.pending && !space.last_space && space.content {
                    out.push(word::text(" ".into(), style));
                }
                space.pending = false;
                if let Some(image) = self.image(node, style)? {
                    out.push(image);
                    space.content = true;
                    space.last_space = false;
                } else if let Some(alt) = node.attr("alt").filter(|s| !s.is_empty()) {
                    out.push(word::text(alt, style));
                }
                return Ok(());
            }
            "canvas" | "object" | "embed" | "iframe" | "video" | "audio" | "input" | "select"
            | "textarea" => {
                self.warnings
                    .insert(format!("unsupported visible HTML element {}", node.tag()));
            }
            _ => {}
        }
        let mut children = Vec::new();
        for child in node.children.borrow().iter() {
            let Some(computed) = self.computed(child, style)? else {
                continue;
            };
            self.inline(child, &computed, &mut children, space)?;
        }
        if node.tag() == "a"
            && let Some(href) = node.attr("href").filter(|h| !h.is_empty())
        {
            if href.starts_with('#') {
                self.warnings
                    .insert("internal HTML bookmark destinations are unsupported".into());
            } else {
                let escaped = href.replace('\\', "\\\\").replace('"', "\\\"");
                out.push(word::element(
                    "w:fldSimple",
                    &[("instr", format!("HYPERLINK \"{escaped}\""))],
                    children,
                ));
                return Ok(());
            }
        }
        out.extend(children);
        Ok(())
    }

    fn image(
        &mut self,
        node: &dom::Handle,
        style: &style::Computed,
    ) -> Result<Option<Rc<XNode>>, &'static str> {
        let Some(src) = node.attr("src") else {
            self.warnings.insert("image source is missing".into());
            return Ok(None);
        };
        let asset = if let Some(data) = src.strip_prefix("data:") {
            let Some((header, data)) = data.split_once(',') else {
                self.warnings.insert("invalid image data URI".into());
                return Ok(None);
            };
            if !header.to_ascii_lowercase().starts_with("image/") || !header.ends_with(";base64") {
                self.warnings
                    .insert("unsupported image data URI encoding".into());
                return Ok(None);
            }
            if data.len() > 12 * 1024 * 1024 {
                return Err("HTML asset input budget exceeded");
            }
            match base64::engine::general_purpose::STANDARD.decode(data) {
                Ok(bytes) => bytes,
                Err(_) => {
                    self.warnings.insert("invalid image Base64".into());
                    return Ok(None);
                }
            }
        } else {
            if src.contains(':') || src.starts_with("//") || src.contains('\\') {
                self.warnings
                    .insert("external image resource was not fetched".into());
                return Ok(None);
            }
            let directory = self.part.rsplit_once('/').map_or("", |(d, _)| d);
            let target = crate::office::pptx::package::resolve_target(directory, &src);
            let Some(size) = self.parser.pkg.part_size(&target) else {
                self.warnings
                    .insert("package image resource is missing".into());
                return Ok(None);
            };
            if size > 8 * 1024 * 1024 {
                return Err("HTML asset input budget exceeded");
            }
            match self.parser.pkg.read(&target) {
                Ok(bytes) => bytes,
                Err(_) => {
                    self.warnings
                        .insert("package image resource cannot be read".into());
                    return Ok(None);
                }
            }
        };
        let Some((px, py)) = crate::office::pptx::render::raster_dimensions(&asset) else {
            self.warnings
                .insert("unsupported or invalid raster image".into());
            return Ok(None);
        };
        if px == 0 || py == 0 || u64::from(px) * u64::from(py) > 64 * 1024 * 1024 {
            return Err("HTML raster pixel budget exceeded");
        }
        for name in ["width", "height"] {
            if style.get(name).is_some_and(|v| v.trim().ends_with('%')) {
                self.warnings
                    .insert("percentage image dimensions are unsupported".into());
            }
        }
        let dimension = |name| {
            style
                .get(name)
                .filter(|s| !s.trim().ends_with('%'))
                .and_then(|s| style::length(s, word::font_size(style)))
                .or_else(|| {
                    node.attr(name)
                        .and_then(|s| s.parse::<f64>().ok())
                        .map(|n| n * 0.75)
                })
                .filter(|n| n.is_finite() && *n > 0.0 && *n <= 100_000.0)
        };
        let (w, h) = match (dimension("width"), dimension("height")) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * f64::from(py) / f64::from(px)),
            (None, Some(h)) => (h * f64::from(px) / f64::from(py), h),
            _ => (f64::from(px) * 0.75, f64::from(py) * 0.75),
        };
        let target = self
            .parser
            .pkg
            .import_asset(&asset)
            .map_err(|_| "HTML asset storage budget exceeded")?;
        let id = self.parser.register_imported_asset(self.host, target);
        let pic = word::element(
            "pic:pic",
            &[],
            vec![word::element(
                "pic:blipFill",
                &[],
                vec![word::element("a:blip", &[("r:embed", id)], Vec::new())],
            )],
        );
        let inline = word::element(
            "wp:inline",
            &[],
            vec![
                word::element(
                    "wp:extent",
                    &[
                        ("cx", (w * EMU_PER_PT).round().to_string()),
                        ("cy", (h * EMU_PER_PT).round().to_string()),
                    ],
                    Vec::new(),
                ),
                word::element(
                    "wp:docPr",
                    &[("descr", node.attr("alt").unwrap_or_default())],
                    Vec::new(),
                ),
                word::element(
                    "a:graphic",
                    &[],
                    vec![word::element("a:graphicData", &[], vec![pic])],
                ),
            ],
        );
        Ok(Some(word::element(
            "w:r",
            &[],
            vec![word::element("w:drawing", &[], vec![inline])],
        )))
    }

    fn table(
        &mut self,
        node: &dom::Handle,
        style: &style::Computed,
    ) -> Result<Rc<XNode>, &'static str> {
        let mut source_rows = Vec::new();
        for (group, child) in node.children.borrow().iter().enumerate() {
            if child.tag() == "tr" {
                source_rows.push((child.clone(), false, 0, style.clone()));
            } else if matches!(child.tag(), "thead" | "tbody" | "tfoot") {
                let Some(group_style) = self.computed(child, style)? else {
                    continue;
                };
                for row in child.children.borrow().iter().filter(|r| r.tag() == "tr") {
                    source_rows.push((
                        row.clone(),
                        child.tag() == "thead",
                        group + 1,
                        group_style.clone(),
                    ));
                }
            } else if child.tag() == "caption" {
                self.warnings
                    .insert("table captions are unsupported".into());
            }
        }
        let mut expanded = Vec::new();
        let mut start = 0;
        while start < source_rows.len() {
            let group = source_rows[start].2;
            let end = start
                + source_rows[start..]
                    .iter()
                    .take_while(|(_, _, g, _)| *g == group)
                    .count();
            let mut height = end - start;
            for (index, (row, _, _, _)) in source_rows[start..end].iter().enumerate() {
                for cell in row
                    .children
                    .borrow()
                    .iter()
                    .filter(|n| matches!(n.tag(), "td" | "th"))
                {
                    if let Some(span) = cell.attr("rowspan") {
                        let span = span
                            .parse::<usize>()
                            .ok()
                            .filter(|n| *n <= 4096)
                            .ok_or("HTML row span exceeds budget")?;
                        height = height.max(index + span);
                    }
                }
            }
            self.charge(height.saturating_mul(32))?;
            expanded.extend(source_rows[start..end].iter().cloned());
            for _ in end - start..height {
                expanded.push((
                    dom::empty_row(),
                    source_rows[start].1,
                    group,
                    source_rows[start].3.clone(),
                ));
            }
            start = end;
        }
        let source_rows = expanded;
        let mut rows = Vec::new();
        let mut active = Vec::<(usize, usize, usize)>::new();
        let mut columns = 0;
        let mut first_widths = Vec::new();
        let mut has_percent_cells = false;
        let mut percent_cells = Vec::new();
        let row_count = source_rows.len();
        let groups: Vec<_> = source_rows.iter().map(|(_, _, group, _)| *group).collect();
        let mut previous_group = None;
        for (ri, (row, header, group, group_style)) in source_rows.into_iter().enumerate() {
            if previous_group != Some(group) {
                active.clear();
            }
            previous_group = Some(group);
            let group_end = groups[ri..]
                .iter()
                .position(|g| *g != group)
                .map_or(row_count, |n| ri + n);
            let Some(row_style) = self.computed(&row, &group_style)? else {
                continue;
            };
            let mut cells = Vec::<(usize, Rc<XNode>)>::new();
            let mut occupied = Vec::<(usize, usize)>::new();
            active.retain(|(_, _, end)| *end > ri);
            for &(start, span, _) in &active {
                self.charge(16)?;
                let pr = word::element(
                    "w:tcPr",
                    &[],
                    vec![
                        word::val("w:gridSpan", span),
                        word::val("w:vMerge", "continue"),
                    ],
                );
                cells.push((
                    start,
                    word::element(
                        "w:tc",
                        &[],
                        vec![pr, word::paragraph(&row_style, Vec::new())],
                    ),
                ));
                occupied.push((start, start + span));
            }
            let mut cursor = 0;
            for cell in row
                .children
                .borrow()
                .iter()
                .filter(|n| matches!(n.tag(), "td" | "th"))
            {
                let Some(cell_style) = self.computed(cell, &row_style)? else {
                    continue;
                };
                let count = |key: &str| -> Result<usize, &'static str> {
                    cell.attr(key)
                        .map(|v| {
                            v.parse::<usize>()
                                .ok()
                                .filter(|n| *n > 0 && *n <= 4096)
                                .ok_or("HTML table span is unsupported or exceeds budget")
                        })
                        .unwrap_or(Ok(1))
                };
                let span = count("colspan")?;
                let height = if cell.attr("rowspan").as_deref() == Some("0") {
                    group_end - ri
                } else {
                    count("rowspan")?.min(group_end - ri)
                };
                while let Some(&(_, end)) = occupied
                    .iter()
                    .find(|&&(start, end)| cursor < end && cursor + span > start)
                {
                    cursor = end;
                }
                if cursor + span > 4096 {
                    return Err("HTML table column budget exceeded");
                }
                self.charge(span.saturating_mul(16))?;
                let mut pr = vec![word::val("w:gridSpan", span)];
                pr.extend(word::cell_properties(&cell_style));
                if height > 1 {
                    pr.push(word::val("w:vMerge", "restart"));
                    active.push((cursor, span, ri + height));
                }
                if let Some(width) = cell_style
                    .get("width")
                    .and_then(|v| word::width(v, word::font_size(&cell_style)))
                {
                    if width.0 == "pct" {
                        has_percent_cells = true;
                        percent_cells.push((
                            cursor,
                            span,
                            width.1.parse::<f64>().unwrap_or(0.0) / 5000.0,
                        ));
                        if ri == 0 {
                            first_widths.push((
                                cursor,
                                span,
                                width.1.parse::<f64>().unwrap_or(0.0) / 5000.0,
                            ));
                        }
                    }
                    pr.push(word::element(
                        "w:tcW",
                        &[("type", width.0), ("w", width.1)],
                        Vec::new(),
                    ));
                }
                if let Some(color) = cell_style.get("background-color").and_then(word::color) {
                    pr.push(word::element(
                        "w:shd",
                        &[("val", "clear".into()), ("fill", color)],
                        Vec::new(),
                    ));
                }
                if let Some(align) = cell_style
                    .get("vertical-align")
                    .filter(|v| matches!(*v, "top" | "middle" | "bottom"))
                {
                    pr.push(word::val(
                        "w:vAlign",
                        if align == "middle" { "center" } else { align },
                    ));
                }
                let mut content = self.blocks(cell, &cell_style)?;
                if content.is_empty() {
                    content.push(word::paragraph(&cell_style, Vec::new()));
                }
                content.insert(0, word::element("w:tcPr", &[], pr));
                cells.push((cursor, word::element("w:tc", &[], content)));
                occupied.push((cursor, cursor + span));
                cursor += span;
                columns = columns.max(cursor);
            }
            cells.sort_by_key(|(start, _)| *start);
            let mut content = Vec::new();
            let mut row_props = Vec::new();
            if header {
                row_props.push(word::val("w:tblHeader", "1"));
            }
            if let Some(height) = row_style.get("height") {
                if height.trim().ends_with('%') {
                    self.warnings
                        .insert("percentage table row heights are unsupported".into());
                } else if let Some(points) =
                    style::length(height, word::font_size(&row_style)).filter(|v| *v > 0.0)
                {
                    row_props.push(word::element(
                        "w:trHeight",
                        &[
                            ("val", (points * 20.0).round().to_string()),
                            ("hRule", "atLeast".into()),
                        ],
                        Vec::new(),
                    ));
                }
            }
            if !row_props.is_empty() {
                content.push(word::element("w:trPr", &[], row_props));
            }
            let mut column = 0;
            for (start, cell) in cells {
                while column < start {
                    self.charge(8)?;
                    content.push(word::element(
                        "w:tc",
                        &[],
                        vec![
                            word::element("w:tcPr", &[], Vec::new()),
                            word::paragraph(&row_style, Vec::new()),
                        ],
                    ));
                    column += 1;
                }
                let span = cell
                    .path(&["w:tcPr", "w:gridSpan"])
                    .and_then(|n| n.attr("val"))
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1);
                column += span;
                content.push(cell);
            }
            columns = columns.max(column);
            rows.push(word::element("w:tr", &[], content));
        }
        let mut props = Vec::new();
        if let Some(width) = style
            .get("width")
            .and_then(|v| word::width(v, word::font_size(style)))
        {
            props.push(word::element(
                "w:tblW",
                &[("type", width.0), ("w", width.1)],
                Vec::new(),
            ));
        }
        let mut children = vec![word::element("w:tblPr", &[], props)];
        let proportional = style
            .get("width")
            .and_then(|v| word::width(v, word::font_size(style)))
            .is_some()
            && first_widths.len() == columns
            && first_widths
                .iter()
                .enumerate()
                .all(|(i, (start, span, f))| *start == i && *span == 1 && *f > 0.0)
            && (first_widths.iter().map(|(_, _, f)| f).sum::<f64>() - 1.0).abs() < 0.0001;
        if proportional
            && percent_cells.iter().any(|(start, span, fraction)| {
                first_widths
                    .get(*start..start.saturating_add(*span))
                    .is_none_or(|columns| {
                        (columns.iter().map(|(_, _, f)| f).sum::<f64>() - fraction).abs() > 0.0001
                    })
            })
        {
            self.warnings
                .insert("conflicting percentage cell widths across rows are unsupported".into());
        }
        if has_percent_cells && !proportional {
            self.warnings.insert(
                "mixed or spanning percentage cell-width negotiation is unsupported".into(),
            );
        }
        children.push(word::element(
            "w:tblGrid",
            &[],
            (0..columns)
                .map(|i| {
                    let width = if proportional {
                        1440.0 * columns as f64 * first_widths[i].2
                    } else {
                        1440.0
                    };
                    word::element("w:gridCol", &[("w", width.round().to_string())], Vec::new())
                })
                .collect(),
        ));
        children.extend(rows);
        Ok(word::element("w:tbl", &[], children))
    }
}
