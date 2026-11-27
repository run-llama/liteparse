//! Characters outside the Basic Multilingual Plane (emoji, CJK Extension B, math
//! alphanumerics) reach the text output whole. pdfium reports each one as two UTF-16
//! halves; the text path used to drop both halves, so a status column of 🟢/🔴 or a name
//! like 𠮷野家 lost the character. Office documents converted through LibreOffice
//! map their glyphs this way.
use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig};

/// F1 is Helvetica for ASCII. F2 is a Type3 font whose glyphs 1-3 are plain boxes,
/// with a ToUnicode map that points them at characters above U+FFFF, written as
/// UTF-16BE surrogate pairs as PDF producers do.
fn pdf(content: &str) -> Vec<u8> {
    let glyph = "600 0 50 0 550 600 d1\n50 0 500 600 re f";
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def \
/CMapName /Adobe-Identity-UCS def /CMapType 2 def \
1 begincodespacerange <00> <FF> endcodespacerange \
3 beginbfchar \
<01> <D83DDFE2> \
<02> <D83DDD34> \
<03> <D842DFB7> \
endbfchar endcmap CMapName currentdict /CMap defineresource pop end end";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 600 700] /FontMatrix [0.001 0 0 0.001 0 0] \
/CharProcs << /g1 7 0 R /g2 7 0 R /g3 7 0 R >> /Encoding << /Type /Encoding /Differences [1 /g1 /g2 /g3] >> \
/FirstChar 1 /LastChar 3 /Widths [600 600 600] /ToUnicode 8 0 R /Resources << >> >>"
            .to_string(),
        stream(content),
        stream(glyph),
        stream(cmap),
    ];
    let mut data = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(data.len());
        data.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = data.len();
    data.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        data.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    data.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    data
}

fn stream(content: &str) -> String {
    format!(
        "<< /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    )
}

async fn page_text(content: &str) -> String {
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        ..LiteParseConfig::default()
    });
    let result = parser
        .parse_input(PdfInput::Bytes(pdf(content)))
        .await
        .unwrap();
    result.pages[0].text.clone()
}

#[tokio::test]
async fn emoji_status_markers_survive() {
    // A status table: the colour is the only thing that says on track or blocked.
    let text = page_text(
        "BT /F1 12 Tf 50 700 Td (Payment API) Tj ET BT /F2 12 Tf 250 700 Td <01> Tj ET \
         BT /F1 12 Tf 50 680 Td (Billing reports) Tj ET BT /F2 12 Tf 250 680 Td <02> Tj ET",
    )
    .await;
    assert!(text.contains('\u{1F7E2}'), "green circle lost: {text:?}");
    assert!(text.contains('\u{1F534}'), "red circle lost: {text:?}");
    assert!(!text.contains('\u{FFFD}'), "replacement char: {text:?}");
}

#[tokio::test]
async fn cjk_extension_b_name_survives() {
    // 𠮷 (U+20BB7) is common in Japanese names; dropping it yields a different name.
    let text =
        page_text("BT /F2 12 Tf 50 700 Td <03> Tj ET BT /F1 12 Tf 57.2 700 Td (Yoshinoya) Tj ET")
            .await;
    assert!(text.contains('\u{20BB7}'), "CJK Ext B char lost: {text:?}");
}
