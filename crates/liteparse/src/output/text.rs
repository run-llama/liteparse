use crate::types::ParsedPage;

/// Format parsed pages as plain text with page headers.
pub fn format_text(pages: &[ParsedPage]) -> String {
    pages
        .iter()
        .map(|page| format!("\n--- Page {} ---\n{}", page.page_number, page.text))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(n: usize, text: &str) -> ParsedPage {
        ParsedPage {
            page_number: n,
            page_width: 0.0,
            page_height: 0.0,
            text: text.into(),
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
        }
    }

    #[test]
    fn test_format_text_empty() {
        assert_eq!(format_text(&[]), "");
    }

    #[test]
    fn test_format_text_single_page() {
        let out = format_text(&[page(1, "hello")]);
        assert!(out.contains("--- Page 1 ---"));
        assert!(out.contains("hello"));
    }

    #[test]
    fn test_format_text_multiple_pages_joined() {
        let out = format_text(&[page(1, "a"), page(2, "b")]);
        assert!(out.contains("--- Page 1 ---"));
        assert!(out.contains("--- Page 2 ---"));
        assert!(out.find("a").unwrap() < out.find("b").unwrap());
    }
}
