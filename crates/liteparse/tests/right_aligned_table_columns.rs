//! Borderless tables whose money columns are right-aligned (#449).
//!
//! The fixture is the fictional grouped-row invoice from the issue: a sparse
//! group row, an indented note, then three detail rows whose amounts are
//! right-aligned and tightly set, so the start x of each amount depends on
//! its width and PDFium joins some neighbouring amounts into one run.
use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig, OutputFormat};

/// One-page 612x792 PDF; `items` are `(x, y, text)` in PDF user space, set in
/// 7pt Helvetica.
fn pdf_with_text(items: &[(f32, f32, &str)]) -> Vec<u8> {
    let mut content = String::new();
    for (x, y, text) in items {
        content += &format!("BT /F1 7 Tf {x} {y} Td ({text}) Tj ET\n");
    }
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
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

/// `(y, description, [(x, amount); 4])` for one detail row.
type DetailRow = (f32, &'static str, [(f32, &'static str); 4]);

/// The #449 reproduction. Amount x positions are the right edges 464 / 514 /
/// 550 / 590 minus each string's Helvetica width, as ReportLab's
/// `drawRightString` places them.
fn grouped_invoice_pdf() -> Vec<u8> {
    let mut items: Vec<(f32, f32, &str)> = vec![
        (108.0, 702.0, "Account"),
        (520.0, 702.0, "Adjust"),
        (48.0, 692.0, "Order Date"),
        (108.0, 692.0, "Suffix"),
        (150.0, 692.0, "Line Section"),
        (215.0, 692.0, "Item Code"),
        (300.0, 692.0, "Description"),
        (362.0, 692.0, "Status"),
        (424.0, 692.0, "Unit Cost"),
        (470.0, 692.0, "Freight Charge"),
        (520.0, 692.0, "Tax"),
        (556.0, 692.0, "Total Billed"),
        (48.0, 676.0, "03/14/2024"),
        (215.0, 676.0, "IC-1048"),
        (362.0, 676.0, "Shipped"),
        (
            300.0,
            664.0,
            "Shipment was delayed in transit and re-routed through the regional distribution hub.",
        ),
    ];
    let rows: [DetailRow; 3] = [
        (
            652.0,
            "WIDGET ASSEMBLY",
            [
                (425.08, "$482,110.40"),
                (496.49, "$0.00"),
                (518.86, "$9,215.75"),
                (551.08, "$491,326.15"),
            ],
        ),
        (
            640.0,
            "BRACKET SET",
            [
                (446.49, "$0.00"),
                (496.49, "$0.00"),
                (532.49, "$0.00"),
                (572.49, "$0.00"),
            ],
        ),
        (
            628.0,
            "CONTROL MODULE",
            [
                (428.97, "$31,905.22"),
                (482.86, "$4,100.00"),
                (532.49, "$0.00"),
                (554.97, "$36,005.22"),
            ],
        ),
    ];
    for (y, description, amounts) in rows {
        items.push((215.0, y, "IC-1048"));
        items.push((300.0, y, description));
        items.push((362.0, y, "Shipped"));
        for (x, amount) in amounts {
            items.push((x, y, amount));
        }
    }
    pdf_with_text(&items)
}

async fn markdown(pdf: Vec<u8>) -> String {
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        quiet: true,
        output_format: OutputFormat::Markdown,
        ..LiteParseConfig::default()
    });
    let result = parser.parse_input(PdfInput::Bytes(pdf)).await.unwrap();
    result.pages[0].markdown.clone()
}

fn cells(row: &str) -> Vec<String> {
    let inner = row.trim().trim_start_matches('|').trim_end_matches('|');
    inner.split('|').map(|c| c.trim().to_string()).collect()
}

#[tokio::test]
async fn grouped_sparse_rows_stay_in_one_table_with_stable_money_columns() {
    let md = markdown(grouped_invoice_pdf()).await;
    let table_rows: Vec<&str> = md.lines().filter(|l| l.starts_with('|')).collect();
    let separators = table_rows.iter().filter(|l| l.starts_with("|---")).count();
    assert_eq!(separators, 1, "expected one table:\n{md}");

    let header = table_rows
        .iter()
        .map(|r| cells(r))
        .find(|c| c.first().map(String::as_str) == Some("Order Date"))
        .unwrap_or_else(|| panic!("no column header row:\n{md}"));
    assert_eq!(
        header,
        [
            "Order Date",
            "Suffix",
            "Line Section",
            "Item Code",
            "Description",
            "Status",
            "Unit Cost",
            "Freight Charge",
            "Tax",
            "Total Billed"
        ],
        "{md}"
    );
    let col = |name: &str| header.iter().position(|h| h == name).unwrap();

    let row_with = |needle: &str| -> Vec<String> {
        table_rows
            .iter()
            .map(|r| cells(r))
            .find(|c| c.iter().any(|cell| cell.contains(needle)))
            .unwrap_or_else(|| panic!("no row containing {needle:?}:\n{md}"))
    };

    // The note stays in the column it is printed under, not in a money column.
    let note = row_with("Shipment was delayed");
    assert!(
        note[col("Description")].starts_with("Shipment was delayed"),
        "{md}"
    );
    assert!(note[col("Unit Cost")].is_empty(), "{md}");

    let expect_amounts = |needle: &str, amounts: [&str; 4]| {
        let row = row_with(needle);
        for (name, amount) in ["Unit Cost", "Freight Charge", "Tax", "Total Billed"]
            .iter()
            .zip(amounts)
        {
            assert_eq!(row[col(name)], amount, "{name} of {needle:?}:\n{md}");
        }
        assert_eq!(row[col("Item Code")], "IC-1048", "{md}");
    };
    expect_amounts(
        "WIDGET",
        ["$482,110.40", "$0.00", "$9,215.75", "$491,326.15"],
    );
    expect_amounts("BRACKET SET", ["$0.00", "$0.00", "$0.00", "$0.00"]);
    expect_amounts(
        "CONTROL",
        ["$31,905.22", "$4,100.00", "$0.00", "$36,005.22"],
    );

    // Every printed amount appears exactly once.
    for amount in [
        "$482,110.40",
        "$9,215.75",
        "$491,326.15",
        "$31,905.22",
        "$4,100.00",
        "$36,005.22",
    ] {
        assert_eq!(md.matches(amount).count(), 1, "{amount}:\n{md}");
    }
}
