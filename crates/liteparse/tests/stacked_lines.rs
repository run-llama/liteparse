//! A short run followed by two lines stacked just above and below its baseline
//! (#575): every string must survive. The upper line used to join the short run
//! as one two-line-tall item, which dedup then dropped as overpaint of the lower.
//! Also #486: one glyph with huge font metrics must not stretch its line over
//! the next and get it deduped away.
use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig};

/// One row of 10pt Helvetica at y=600, `QUEBECROME` and `KIT`, then `LIMA NO.
/// MIKE` `up` points above the row and `OSCAR PAPA:` `down` points below it.
fn stacked(up: f32, down: f32, lower_first: bool) -> Vec<u8> {
    let row = 600.0;
    let upper = (140.0, row + up, "LIMA NO. MIKE");
    let lower = (140.0, row - down, "OSCAR PAPA:");
    let mut runs = vec![(20.0, row, "QUEBECROME"), (123.0, row, "KIT")];
    if lower_first {
        runs.extend([lower, upper]);
    } else {
        runs.extend([upper, lower]);
    }
    let content = runs
        .iter()
        .map(|(x, y, t)| format!("BT /F1 10 Tf {x} {y:.1} Td ({t}) Tj ET"))
        .collect::<Vec<_>>()
        .join("\n");
    pdf(&content, &[])
}

/// A one-page PDF with Helvetica as `/F1`. `extra` objects are numbered from 6;
/// the first, when present, is the font `/F2`.
fn pdf(content: &str, extra: &[&str]) -> Vec<u8> {
    let fonts = if extra.is_empty() {
        "/F1 4 0 R"
    } else {
        "/F1 4 0 R /F2 6 0 R"
    };
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << {fonts} >> >> /Contents 5 0 R >>"
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ];
    objects.extend(extra.iter().map(|s| s.to_string()));
    let mut data = b"%PDF-1.4\n".to_vec();
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

async fn items(bytes: Vec<u8>) -> Vec<String> {
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        ..LiteParseConfig::default()
    });
    let result = parser.parse_input(PdfInput::Bytes(bytes)).await.unwrap();
    result.pages[0]
        .text_items
        .iter()
        .map(|item| item.text.clone())
        .collect()
}

#[tokio::test]
async fn stacked_lines_after_short_run_keep_every_string() {
    let got = items(stacked(6.0, 4.0, false)).await;
    for want in ["QUEBECROME", "KIT", "LIMA NO. MIKE", "OSCAR PAPA:"] {
        assert!(got.iter().any(|t| t == want), "missing {want:?} in {got:?}");
    }
}

#[tokio::test]
async fn stacked_lines_clear_of_the_row_keep_every_string() {
    let got = items(stacked(12.0, 12.0, false)).await;
    for want in ["QUEBECROME", "KIT", "LIMA NO. MIKE", "OSCAR PAPA:"] {
        assert!(got.iter().any(|t| t == want), "missing {want:?} in {got:?}");
    }
}

/// Drawn lower line first, the 4pt step down is within superscript range and
/// still joins `KIT`; no text may be lost either way.
#[tokio::test]
async fn stacked_lines_lower_first_lose_no_text() {
    let got = items(stacked(6.0, 4.0, true)).await.join(" ");
    for want in ["QUEBECROME", "KIT", "LIMA NO. MIKE", "OSCAR PAPA:"] {
        assert!(got.contains(want), "missing {want:?} in {got:?}");
    }
}

/// #486: a hyphen in a font with Cambria Math's `/Ascent 3116 /Descent -2463`
/// ends the middle of three 9pt lines; the line above must survive.
#[tokio::test]
async fn tall_glyph_does_not_drop_adjacent_line() {
    let lines = [
        "Based upon payment history, the otherwise applicable premium shall be modified.",
        "if there are no late fees in the previous 24 months. History is re",
        "evaluated at each renewal.",
    ];
    let content = lines
        .iter()
        .enumerate()
        .map(|(k, text)| {
            let y = 700.0 - 10.4 * k as f32;
            let hyphen = if k == 1 { "/F2 9 Tf (-) Tj " } else { "" };
            format!("BT /F1 9 Tf 72 {y:.1} Td ({text}) Tj {hyphen}ET")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let tall = [
        "<< /Type /Font /Subtype /TrueType /BaseFont /TallMath /Encoding /WinAnsiEncoding /FirstChar 45 /LastChar 45 /Widths [333] /FontDescriptor 7 0 R >>",
        "<< /Type /FontDescriptor /FontName /TallMath /Flags 32 /FontBBox [-1474 -2463 2867 3116] /ItalicAngle 0 /Ascent 3116 /Descent -2463 /CapHeight 500 /StemV 88 >>",
    ];
    let got = items(pdf(&content, &tall)).await.join(" ");
    for want in [
        "Based upon payment history",
        "no late fees",
        "evaluated at each renewal.",
    ] {
        assert!(got.contains(want), "missing {want:?} in {got:?}");
    }
}
