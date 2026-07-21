use crate::config::ImageMode;
use crate::markdown_layout::{
    build_heading_map, classify_page_with_filters, compute_body_size, compute_header_footer_set,
    detect_single_page_chrome, render_blocks,
};
use crate::types::{OutlineTarget, ParsedPage};

/// Format parsed pages as markdown.
///
/// Whole-document signals (body font size, heading-level map, repeating
/// header/footer set) are computed once up front, then each page is classified
/// into blocks ([`classify_page_with_filters`]) and rendered. Block classes
/// cover headings, paragraphs (with de-hyphenation and inline emphasis), lists,
/// code blocks, ruled and borderless tables, horizontal rules, and figures.
///
/// Pages are emitted in order, separated by `\n\n-----\n\n`.
/// Pages that contain no projected lines (e.g. blank
/// or fully-OCR pages without font-size info) fall back to the projected text
/// wrapped in a fenced block so we never silently drop content.
pub fn format_markdown(
    pages: &[ParsedPage],
    outline: &[OutlineTarget],
    image_mode: ImageMode,
) -> String {
    format_markdown_pages(pages, outline, image_mode).join("\n\n-----\n\n")
}

/// Render each page to its own markdown string, returning one entry per input
/// page (in order). [`format_markdown`] joins these with page separators to
/// build the document output; callers wanting per-page markdown (e.g. to
/// populate `ParsedPage.markdown`) use this directly. Doc-level context
/// (body size, heading map, header/footer set) is still computed across all
/// pages so a single page renders identically whether requested alone or as
/// part of the document.
pub fn format_markdown_pages(
    pages: &[ParsedPage],
    outline: &[OutlineTarget],
    image_mode: ImageMode,
) -> Vec<String> {
    if pages.is_empty() {
        return Vec::new();
    }

    let body_size = compute_body_size(pages);
    let heading_map = build_heading_map(pages, body_size);
    let header_footer = compute_header_footer_set(pages);

    pages
        .iter()
        .map(|page| {
            if page.projected_lines.is_empty() {
                // No structural metadata for this page — fall back to the
                // projection text inside a fence so nothing is dropped.
                let mut out = String::from("```text\n");
                out.push_str(&page.text);
                if !page.text.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("```");
                return out;
            }

            // Filter outline entries to this page so the classifier's y/title
            // match is a O(entries_on_page) scan per line, not O(whole doc).
            let target_index = (page.page_number as i32).saturating_sub(1);
            let page_outline: Vec<OutlineTarget> = outline
                .iter()
                .filter(|e| e.page_index == target_index)
                .cloned()
                .collect();
            let chrome_indices = detect_single_page_chrome(page, body_size);
            let mut blocks = classify_page_with_filters(
                page,
                &heading_map,
                &header_footer,
                &page_outline,
                image_mode,
                &chrome_indices,
            );
            // A page whose only surviving blocks are horizontal rules (all its
            // text was stripped as chrome) should render empty, not as a stack
            // of bare `---` separators.
            let has_content = blocks.iter().any(|b| {
                !matches!(
                    b,
                    crate::markdown_layout::Block::HorizontalRule
                        | crate::markdown_layout::Block::Figure { .. }
                )
            });
            if !has_content {
                blocks.retain(|b| !matches!(b, crate::markdown_layout::Block::HorizontalRule));
            }
            dedupe_rules(&mut blocks);
            render_blocks(&blocks)
        })
        .collect()
}

/// Collapse cosmetic horizontal-rule noise on a single page's block stream:
/// drop leading/trailing rules (which would otherwise abut the `-----` page
/// separator) and collapse runs of consecutive rules to one. Rules come from
/// two sources — vector-graphics detection and decorative divider text — and
/// doubling up reads as sloppy output to a human, while carrying no extra
/// structure for an LLM.
fn dedupe_rules(blocks: &mut Vec<crate::markdown_layout::Block>) {
    use crate::markdown_layout::Block::HorizontalRule;
    while matches!(blocks.first(), Some(HorizontalRule)) {
        blocks.remove(0);
    }
    while matches!(blocks.last(), Some(HorizontalRule)) {
        blocks.pop();
    }
    blocks.dedup_by(|a, b| matches!((a, b), (HorizontalRule, HorizontalRule)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Anchor, ProjectedLine, Rect, TextItem};

    fn line(text: &str, x: f32, y: f32, h: f32, size: f32) -> ProjectedLine {
        ProjectedLine {
            text: text.into(),
            bbox: Rect {
                x,
                y,
                width: text.chars().count() as f32 * (size * 0.5),
                height: h,
            },
            anchor: Anchor::Left,
            indent_x: x,
            dominant_font_size: size,
            font_size_is_estimated: false,
            heading_font_size: None,
            dominant_font_name: Some("Arial".into()),
            all_bold: false,
            all_italic: false,
            all_mono: false,
            all_strike: false,
            spans: vec![TextItem::default()],
            region_path: Vec::new(),
            mcid: None,
            in_figure: false,
        }
    }

    fn page_with(n: usize, lines: Vec<ProjectedLine>) -> ParsedPage {
        ParsedPage {
            page_number: n,
            page_width: 612.0,
            page_height: 792.0,
            text: "fallback".into(),
            markdown: String::new(),
            text_items: vec![],
            projected_lines: lines,
            regions: crate::types::Region::default(),
            graphics: vec![],
            vector_graphics: None,
            figures: vec![],
            struct_nodes: vec![],
            image_refs: vec![],
            complexity: None,
        }
    }

    #[test]
    fn test_empty() {
        assert_eq!(format_markdown(&[], &[], ImageMode::Placeholder), "");
    }

    #[test]
    fn dedupe_rules_drops_edges_and_collapses_runs() {
        use crate::markdown_layout::Block::{self, HorizontalRule, Paragraph};
        let p = |t: &str| Paragraph {
            text: t.into(),
            bold: false,
            italic: false,
        };
        let mut blocks = vec![
            HorizontalRule,
            p("a"),
            HorizontalRule,
            HorizontalRule,
            p("b"),
            HorizontalRule,
        ];
        dedupe_rules(&mut blocks);
        let kinds: Vec<bool> = blocks
            .iter()
            .map(|b| matches!(b, Block::HorizontalRule))
            .collect();
        // Leading + trailing rules gone; the doubled interior run collapsed to one.
        assert_eq!(kinds, vec![false, true, false]);
    }

    #[test]
    fn test_fallback_when_no_projected_lines() {
        let p = ParsedPage {
            page_number: 1,
            page_width: 0.0,
            page_height: 0.0,
            text: "hello".into(),
            markdown: String::new(),
            text_items: vec![],
            projected_lines: vec![],
            regions: crate::types::Region::default(),
            graphics: vec![],
            vector_graphics: None,
            figures: vec![],
            struct_nodes: vec![],
            image_refs: vec![],
            complexity: None,
        };
        let out = format_markdown(&[p], &[], ImageMode::Placeholder);
        assert!(out.contains("```text"));
        assert!(out.contains("hello"));
    }

    #[test]
    fn test_heading_and_paragraph() {
        let p = page_with(
            1,
            vec![
                line("My Title For This Test Document", 50.0, 50.0, 18.0, 18.0),
                // Enough body text to dominate the char-weighted body-size
                // mode so the title at 18pt registers as larger-than-body.
                line("First sentence of body prose here.", 50.0, 80.0, 10.0, 10.0),
                line(
                    "Second sentence of body prose here.",
                    50.0,
                    92.0,
                    10.0,
                    10.0,
                ),
                line(
                    "Third sentence of body prose here.",
                    50.0,
                    104.0,
                    10.0,
                    10.0,
                ),
            ],
        );
        let out = format_markdown(&[p], &[], ImageMode::Placeholder);
        assert!(out.contains("# My Title For This Test Document"));
        assert!(out.contains("First sentence of body prose here."));
    }

    #[test]
    fn test_multi_page_separator() {
        let a = page_with(1, vec![line("A page.", 50.0, 80.0, 10.0, 10.0)]);
        let b = page_with(2, vec![line("B page.", 50.0, 80.0, 10.0, 10.0)]);
        let out = format_markdown(&[a, b], &[], ImageMode::Placeholder);
        assert!(out.contains("-----"));
        assert!(out.find("A page.").unwrap() < out.find("B page.").unwrap());
    }

    #[test]
    fn test_per_page_matches_joined_document() {
        let a = page_with(1, vec![line("A page.", 50.0, 80.0, 10.0, 10.0)]);
        let b = page_with(2, vec![line("B page.", 50.0, 80.0, 10.0, 10.0)]);
        let pages = [a, b];
        let per_page = format_markdown_pages(&pages, &[], ImageMode::Placeholder);
        assert_eq!(per_page.len(), 2);
        assert!(per_page[0].contains("A page."));
        assert!(per_page[1].contains("B page."));
        // The per-page strings carry no separator on their own; the document
        // form is exactly the join.
        assert!(!per_page[0].contains("-----"));
        assert_eq!(
            format_markdown(&pages, &[], ImageMode::Placeholder),
            per_page.join("\n\n-----\n\n")
        );
    }
}
