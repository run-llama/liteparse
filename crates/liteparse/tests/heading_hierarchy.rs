//! A heading found only by its bold style must nest one level below the
//! section it sits under (#657). In a tagged PDF whose headings share the body
//! font, these labels came out as `#` under `###` sections; under font-size
//! headings they could jump straight to `######`.
use liteparse::config::OcrBackend;
use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig, OutputFormat};

/// A section heading at a level, or a body-size bold label. Each is followed
/// by one body line.
#[derive(Clone, Copy)]
enum Entry {
    H(u8, &'static str),
    Label(&'static str),
}
use Entry::{H, Label};

/// What gives the section headings their level.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    /// Bold Helvetica at 20 / 16 / 13pt over a 10pt body.
    Size,
    /// Bold at body size, tagged `H1`..`H6` in the structure tree.
    Tag,
    /// Bold at body size, with a bookmark at the heading's depth.
    Outline,
}

/// PDF of `pages`, Helvetica body text and Helvetica-Bold headings and labels.
fn pdf(source: Source, pages: &[&[Entry]]) -> Vec<u8> {
    let n = pages.len();
    // 1 catalog, 2 page tree, 3-4 fonts, then a page and its content per page
    let mut objects = vec![String::new(); 4 + 2 * n];
    let page_obj = |i: usize| 5 + 2 * i;
    let mut tags: Vec<(usize, usize, String)> = Vec::new(); // (page, mcid, role)
    let mut marks: Vec<(u8, &str, usize, f32)> = Vec::new(); // (level, title, page, y)
    for (i, entries) in pages.iter().enumerate() {
        let mut ops = Vec::new();
        let mut y = 750.0;
        let mut mcid = 0;
        let mut tag = |role: &str, op: String, ops: &mut Vec<String>| {
            if source == Source::Tag {
                ops.push(format!("/{role} <</MCID {mcid}>> BDC {op} EMC"));
                tags.push((i, mcid, role.to_string()));
                mcid += 1;
            } else {
                ops.push(op);
            }
        };
        for entry in *entries {
            let (role, size, text) = match *entry {
                H(level, text) if source == Source::Size => (
                    "P".to_string(),
                    [20.0, 16.0, 13.0][level as usize - 1],
                    text,
                ),
                H(level, text) => (format!("H{level}"), 10.0, text),
                Label(text) => ("P".to_string(), 10.0, text),
            };
            y -= size * 2.0;
            tag(
                &role,
                format!("BT /F2 {size} Tf 72 {y} Td ({text}) Tj ET"),
                &mut ops,
            );
            if let (H(level, _), Source::Outline) = (entry, source) {
                marks.push((*level, text, i, y + size));
            }
            y -= 14.0;
            // naming the entry keeps every body line unique, so none reads as
            // a running header or footer
            let body =
                format!("Lorem ipsum dolor sit amet on {text}, consectetur adipiscing elit sed.");
            tag(
                "P",
                format!("BT /F1 10 Tf 72 {y} Td ({body}) Tj ET"),
                &mut ops,
            );
        }
        let content = ops.join("\n");
        let struct_parents = if source == Source::Tag {
            format!(" /StructParents {i}")
        } else {
            String::new()
        };
        objects[page_obj(i) - 1] = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R{struct_parents} >>",
            page_obj(i) + 1
        );
        objects[page_obj(i)] = format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        );
    }
    objects[2] =
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into();
    objects[3] =
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
            .into();

    let mut catalog = String::from("<< /Type /Catalog /Pages 2 0 R");
    if source == Source::Tag {
        let root = objects.len() + 1;
        let document = root + 1;
        let first = root + 2;
        let refs = |page: usize| {
            tags.iter()
                .enumerate()
                .filter(|(_, t)| t.0 == page)
                .map(|(k, _)| format!("{} 0 R", first + k))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let nums = (0..n)
            .map(|i| format!("{i} [{}]", refs(i)))
            .collect::<Vec<_>>()
            .join(" ");
        let all = (0..tags.len())
            .map(|k| format!("{} 0 R", first + k))
            .collect::<Vec<_>>()
            .join(" ");
        objects.push(format!(
            "<< /Type /StructTreeRoot /K {document} 0 R /ParentTree << /Nums [{nums}] >> /ParentTreeNextKey {n} >>"
        ));
        objects.push(format!(
            "<< /Type /StructElem /S /Document /P {root} 0 R /K [{all}] >>"
        ));
        for (page, mcid, role) in &tags {
            objects.push(format!(
                "<< /Type /StructElem /S /{role} /P {document} 0 R /Pg {} 0 R /K {mcid} >>",
                page_obj(*page)
            ));
        }
        catalog += &format!(" /MarkInfo << /Marked true >> /StructTreeRoot {root} 0 R");
    }
    if !marks.is_empty() {
        // a bookmark's parent is the nearest earlier one a level up
        let root = objects.len() + 1;
        let num = |k: usize| root + 1 + k;
        let parent: Vec<Option<usize>> = (0..marks.len())
            .map(|k| (0..k).rev().find(|&j| marks[j].0 < marks[k].0))
            .collect();
        let children = |p: Option<usize>| -> Vec<usize> {
            (0..marks.len()).filter(|&k| parent[k] == p).collect()
        };
        let top = children(None);
        objects.push(format!(
            "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>",
            num(top[0]),
            num(*top.last().unwrap()),
            marks.len()
        ));
        for (k, (_, title, page, y)) in marks.iter().enumerate() {
            let siblings = children(parent[k]);
            let at = siblings.iter().position(|&s| s == k).unwrap();
            let mut item = format!(
                "<< /Title ({title}) /Parent {} 0 R /Dest [{} 0 R /XYZ 0 {y} 0]",
                parent[k].map_or(root, num),
                page_obj(*page)
            );
            if at > 0 {
                item += &format!(" /Prev {} 0 R", num(siblings[at - 1]));
            }
            if let Some(&next) = siblings.get(at + 1) {
                item += &format!(" /Next {} 0 R", num(next));
            }
            let kids = children(Some(k));
            if let (Some(&first), Some(&last)) = (kids.first(), kids.last()) {
                item += &format!(
                    " /First {} 0 R /Last {} 0 R /Count {}",
                    num(first),
                    num(last),
                    kids.len()
                );
            }
            objects.push(item + " >>");
        }
        catalog += &format!(" /Outlines {root} 0 R");
    }
    objects[0] = catalog + " >>";
    let kids = (0..n)
        .map(|i| format!("{} 0 R", page_obj(i)))
        .collect::<Vec<_>>()
        .join(" ");
    objects[1] = format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>");

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

/// Markdown ATX headings, as `"### Text"`, in reading order.
async fn headings(bytes: Vec<u8>) -> Vec<String> {
    let parser = LiteParse::new(LiteParseConfig {
        // the heuristic block path: an installed ocria-nano package would run LayoutNet instead
        ocr_backend: Some(OcrBackend::Tesseract),
        ocr_enabled: false,
        output_format: OutputFormat::Markdown,
        quiet: true,
        ..LiteParseConfig::default()
    });
    let result = parser.parse_input(PdfInput::Bytes(bytes)).await.unwrap();
    result
        .pages
        .iter()
        .flat_map(|p| p.markdown.lines())
        .filter(|l| l.starts_with('#'))
        .map(String::from)
        .collect()
}

/// Sections climbing `# ## ### ## #`, each followed by a label.
const CLIMB: &[Entry] = &[
    H(1, "Alpha Section"),
    Label("Alpha Label:"),
    H(2, "Beta Section"),
    Label("1. Beta Label"),
    H(3, "Gamma Section"),
    Label("Gamma Label:"),
    H(2, "Delta Section"),
    Label("2. Delta Label"),
    H(1, "Epsilon Section"),
    Label("Epsilon Label:"),
];

const CLIMB_EXPECTED: &[&str] = &[
    "# Alpha Section",
    "## Alpha Label:",
    "## Beta Section",
    "### 1. Beta Label",
    "### Gamma Section",
    "#### Gamma Label:",
    "## Delta Section",
    "### 2. Delta Label",
    "# Epsilon Section",
    "## Epsilon Label:",
];

#[tokio::test]
async fn labels_nest_under_tagged_body_size_sections() {
    // the #657 case: a Word-style export whose headings share the body font
    assert_eq!(headings(pdf(Source::Tag, &[CLIMB])).await, CLIMB_EXPECTED);
}

#[tokio::test]
async fn labels_nest_under_font_size_sections() {
    assert_eq!(headings(pdf(Source::Size, &[CLIMB])).await, CLIMB_EXPECTED);
}

#[tokio::test]
async fn labels_nest_under_bookmarked_sections() {
    assert_eq!(
        headings(pdf(Source::Outline, &[CLIMB])).await,
        CLIMB_EXPECTED
    );
}

#[tokio::test]
async fn label_opening_a_page_ranks_below_every_section_level() {
    // its section began on the previous page, which the page cannot see
    let pages: &[&[Entry]] = &[
        &[
            H(1, "Alpha Section"),
            H(2, "Beta Section"),
            H(3, "Gamma Section"),
        ],
        &[Label("Continued Label:"), H(1, "Delta Section")],
    ];
    assert_eq!(
        headings(pdf(Source::Tag, pages)).await,
        [
            "# Alpha Section",
            "## Beta Section",
            "### Gamma Section",
            "#### Continued Label:",
            "# Delta Section",
        ]
    );
}

#[tokio::test]
async fn labels_under_h6_stay_at_h6() {
    let entries: &[Entry] = &[
        H(1, "Level One Section"),
        H(2, "Level Two Section"),
        H(3, "Level Three Section"),
        H(4, "Level Four Section"),
        H(5, "Level Five Section"),
        H(6, "Level Six Section"),
        Label("Deepest Label:"),
    ];
    let got = headings(pdf(Source::Tag, &[entries])).await;
    assert_eq!(
        got.last().map(String::as_str),
        Some("###### Deepest Label:")
    );
}

#[tokio::test]
async fn labels_without_sections_stay_top_level() {
    let entries: &[Entry] = &[Label("Overview Label:"), Label("1. Numbered Label")];
    assert_eq!(
        headings(pdf(Source::Size, &[entries])).await,
        ["# Overview Label:", "# 1. Numbered Label"]
    );
}
