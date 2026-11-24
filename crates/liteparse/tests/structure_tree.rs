//! Tagged-PDF regressions for page-scoped content IDs.
use std::io::Write;

use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig};

/// The first paragraph uses a direct MCID on page 1 and an MCR on page 2.
/// Both MCID numbers also belong to unrelated elements on the opposite page.
fn paragraph_across_pages() -> Vec<u8> {
    let stream = |text: &str| format!("<< /Length {} >>\nstream\n{text}\nendstream", text.len());
    let first_page = stream(concat!(
        "/H1 << /MCID 0 >> BDC BT /F1 16 Tf 50 745 Td (Introduction) Tj ET EMC\n",
        "/P << /MCID 1 >> BDC BT /F1 12 Tf 50 713 Td (A paragraph begins here) Tj ET EMC",
    ));
    let second_page = stream(concat!(
        "/P << /MCID 0 >> BDC BT /F1 12 Tf 50 745 Td (and finishes here.) Tj ET EMC\n",
        "/P << /MCID 1 >> BDC BT /F1 12 Tf 50 713 Td (A different paragraph.) Tj ET EMC",
    ));
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 8 0 R /MarkInfo << /Marked true >> >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R /StructParents 0 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 7 0 R /StructParents 1 >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        &first_page,
        &second_page,
        "<< /Type /StructTreeRoot /K 9 0 R /ParentTree 13 0 R /ParentTreeNextKey 2 >>",
        "<< /Type /StructElem /S /Document /P 8 0 R /K [10 0 R 11 0 R 12 0 R] >>",
        "<< /Type /StructElem /S /H1 /P 9 0 R /Pg 3 0 R /K 0 >>",
        "<< /Type /StructElem /S /P /P 9 0 R /Pg 3 0 R /K [1 << /Type /MCR /Pg 4 0 R /MCID 0 >>] >>",
        "<< /Type /StructElem /S /P /P 9 0 R /Pg 4 0 R /K 1 >>",
        "<< /Nums [0 [10 0 R 11 0 R] 1 [11 0 R 12 0 R]] >>",
    ];
    let mut data = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(data.len());
        writeln!(data, "{} 0 obj\n{object}\nendobj", index + 1).unwrap();
    }
    let xref = data.len();
    let size = offsets.len() + 1;
    writeln!(data, "xref\n0 {size}\n0000000000 65535 f ").unwrap();
    for offset in offsets {
        writeln!(data, "{offset:010} 00000 n ").unwrap();
    }
    writeln!(
        data,
        "trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
    )
    .unwrap();
    data
}

#[tokio::test]
async fn structure_tree_uses_only_the_current_pages_marked_content_ids() {
    let expected = [
        (1, vec![("H1", vec![0]), ("P", vec![1])]),
        (2, vec![("P", vec![0]), ("P", vec![1])]),
    ];
    let pdf = paragraph_across_pages();
    for (target_pages, expected_pages) in [
        (None, &expected[..]),
        (Some("1"), &expected[..1]),
        (Some("2"), &expected[1..]),
    ] {
        let parser = LiteParse::new(LiteParseConfig {
            target_pages: target_pages.map(str::to_owned),
            extract_structure_tree: true,
            ocr_enabled: false,
            quiet: true,
            ..LiteParseConfig::default()
        });
        let parsed = parser
            .parse_input(PdfInput::Bytes(pdf.clone()))
            .await
            .unwrap();
        let actual: Vec<_> = parsed
            .pages
            .iter()
            .map(|page| {
                let root = &page.structure_tree.as_ref().unwrap().roots[0];
                let children = root
                    .children
                    .iter()
                    .map(|element| {
                        (
                            element.element_type.as_str(),
                            element.marked_content_ids.clone(),
                        )
                    })
                    .collect::<Vec<_>>();
                (page.page_number, children)
            })
            .collect();
        assert_eq!(actual, expected_pages, "target_pages={target_pages:?}");
    }
}
