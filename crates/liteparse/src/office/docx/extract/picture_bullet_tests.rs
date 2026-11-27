use std::io::{Read, Write};

use crate::office::pptx::scene::Item;

fn package(text: &str, picture_ref: bool, valid_asset: bool) -> Vec<u8> {
    let picture = if picture_ref {
        r#"<w:lvlPicBulletId w:val="3"/>"#
    } else {
        ""
    };
    let numbering = format!(
        r#"
      <w:numPicBullet w:numPicBulletId="3"><w:pict xmlns:v="urn:schemas-microsoft-com:vml" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><v:shape style="width:12pt;height:10pt"><v:imagedata r:id="image"/></v:shape></w:pict></w:numPicBullet>
      <w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="{text}"/>{picture}<w:suff w:val="space"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>
      <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#
    );
    let body = r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>BODY</w:t></w:r></w:p>"#;
    let bytes = crate::office::docx::testutil::docx(body, None, Some(&numbering));
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut output = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut output);
        for i in 0..input.len() {
            let mut file = input.by_index(i).unwrap();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            let original = file.name().unwrap().into_owned();
            let name = match original.as_str() {
                "word/numbering.xml" => "word/lists/numbering.xml",
                name => name,
            };
            if name == "word/_rels/document.xml.rels" {
                bytes = String::from_utf8(bytes)
                    .unwrap()
                    .replace("Target=\"numbering.xml\"", "Target=\"lists/numbering.xml\"")
                    .into_bytes();
            }
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.start_file(
            "word/lists/_rels/numbering.xml.rels",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="image" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/bullet.png"/></Relationships>"#).unwrap();
        zip.start_file(
            "word/media/bullet.png",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        let mut pixmap = tiny_skia::Pixmap::new(32, 32).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(0, 80, 255, 255));
        let asset = if valid_asset {
            pixmap.encode_png().unwrap()
        } else {
            b"invalid image".to_vec()
        };
        zip.write_all(&asset).unwrap();
        zip.start_file(
            "[Content_Types].xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/lists/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/></Types>"#).unwrap();
        zip.finish().unwrap();
    }
    output.into_inner()
}

fn patch(bytes: Vec<u8>, part: &str, edit: impl FnOnce(String) -> String) -> Vec<u8> {
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut output = std::io::Cursor::new(Vec::new());
    let mut original = String::new();
    {
        let mut zip = zip::ZipWriter::new(&mut output);
        for i in 0..input.len() {
            let mut file = input.by_index(i).unwrap();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            if file.name().unwrap() == part {
                original = String::from_utf8(bytes).unwrap();
                continue;
            }
            zip.start_file(
                file.name().unwrap(),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.start_file(part, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(edit(original).as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    output.into_inner()
}

fn image_count(pages: &[crate::office::docx::paginate::OutPage]) -> usize {
    pages
        .iter()
        .flat_map(|p| &p.items)
        .filter(|item| matches!(item, Item::Image(_)))
        .count()
}

#[test]
fn picture_bullet_approximate_size_follows_resolved_label_and_aspect() {
    for (mark, level, expected) in [(10, None, 8.0), (24, None, 19.2), (16, Some(24), 19.2)] {
        for dimensions in ["width:12pt;height:10pt", "width:1200pt;height:1000pt"] {
            let bytes = patch(package("A", true, true), "word/document.xml", |s| {
                s.replace(
                    "</w:pPr>",
                    &format!("<w:rPr><w:sz w:val=\"{}\"/></w:rPr></w:pPr>", mark * 2),
                )
            });
            let bytes = patch(bytes, "word/lists/numbering.xml", |s| {
                let s = s.replace("width:12pt;height:10pt", dimensions);
                match level {
                    Some(size) => s.replace(
                        "</w:lvl>",
                        &format!("<w:rPr><w:sz w:val=\"{}\"/></w:rPr></w:lvl>", size * 2),
                    ),
                    None => s,
                }
            });
            let pages = crate::office::docx::testutil::pages(bytes);
            assert_eq!(pages.len(), 1);
            let images: Vec<_> = pages[0]
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Image(image) => Some(image),
                    _ => None,
                })
                .collect();
            assert_eq!(images.len(), 1);
            assert!((images[0].transform.sy as f64 - expected).abs() < 0.001);
            assert!((images[0].transform.sx as f64 - expected * 1.2).abs() < 0.001);
        }
    }
}

#[test]
fn picture_bullet_fallbacks_keep_original_label_and_paint() {
    let baseline = crate::office::docx::testutil::pages(package("AA", false, true));
    let scene = |pages: &[crate::office::docx::paginate::OutPage]| {
        format!("{:?}", pages.iter().map(|p| &p.items).collect::<Vec<_>>())
    };
    let bad_relation = patch(
        package("AA", true, true),
        "word/lists/_rels/numbering.xml.rels",
        |s| s.replace("../media/bullet.png", "../media/absent.png"),
    );
    let no_size = patch(package("AA", true, true), "word/lists/numbering.xml", |s| {
        s.replace("width:12pt;height:10pt", "")
    });
    for bytes in [package("AA", true, false), bad_relation, no_size] {
        let pages = crate::office::docx::testutil::pages(bytes);
        assert_eq!(image_count(&pages), 0);
        assert_eq!(scene(&pages), scene(&baseline));
    }
    assert_eq!(
        image_count(&crate::office::docx::testutil::pages(package(
            "", true, true
        ))),
        0
    );
}

#[test]
fn picture_bullet_override_and_relationship_part_are_respected() {
    let bytes = patch(
        package("AA", true, true),
        "word/_rels/document.xml.rels",
        |s| {
            s.replace("</Relationships>", r#"<Relationship Id="image" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/missing.png"/></Relationships>"#)
        },
    );
    assert_eq!(image_count(&crate::office::docx::testutil::pages(bytes)), 2);
    let bytes = patch(package("AA", true, true), "word/lists/numbering.xml", |s| {
        s.replace("</w:num>", r#"<w:lvlOverride w:ilvl="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="B"/></w:lvl></w:lvlOverride></w:num>"#)
    });
    let pages = crate::office::docx::testutil::pages(bytes);
    assert_eq!(image_count(&pages), 0);
    let raw: String = pages
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|item| match item {
            Item::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    assert!(raw.starts_with('B'), "{raw}");
}

#[test]
fn picture_bullet_table_eligibility_depends_on_decoded_image() {
    for valid in [false, true] {
        let bytes = patch(package("AA", true, valid), "word/document.xml", |s| {
            s.replace(
                "<w:body>",
                "<w:body><w:tbl><w:tblGrid><w:gridCol w:w=\"6000\"/></w:tblGrid><w:tr><w:tc>",
            )
            .replace("</w:body>", "</w:tc></w:tr></w:tbl></w:body>")
        });
        let pages = crate::office::docx::testutil::pages(bytes);
        assert_eq!(image_count(&pages), if valid { 2 } else { 0 });
        let sources: Vec<_> = pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|item| match item {
                Item::Text(t) if t.text.contains("BODY") => t.source.as_ref(),
                _ => None,
            })
            .collect();
        assert!(!sources.is_empty());
        assert!(
            sources
                .iter()
                .all(|source| source.cells.iter().all(|cell| cell.text_only != valid))
        );
    }
}

#[test]
fn picture_bullet_replaces_each_literal_label_character() {
    let pages = crate::office::docx::testutil::pages(package("AA", true, true));
    let images: Vec<_> = pages
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|item| match item {
            Item::Image(i) => Some(i),
            _ => None,
        })
        .collect();
    assert_eq!(
        images.len(),
        2,
        "each literal label character owns an image"
    );
    assert!(
        images
            .iter()
            .all(|image| image.image.part == "word/media/bullet.png")
    );
    let raw: String = pages
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|item| match item {
            Item::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(raw.trim(), "BODY");
}

#[test]
fn picture_bullet_counts_unicode_scalars_and_respects_rtl() {
    let bytes = patch(package("A😀", true, true), "word/document.xml", |s| {
        s.replace("<w:pPr>", "<w:pPr><w:bidi/>")
    });
    let pages = crate::office::docx::testutil::pages(bytes);
    assert_eq!(image_count(&pages), 2);
    let body = pages
        .iter()
        .flat_map(|p| &p.items)
        .find_map(|item| match item {
            Item::Text(t) if t.text == "BODY" => Some(t),
            _ => None,
        })
        .unwrap();
    assert!(
        pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|item| match item {
                Item::Image(i) => Some(i),
                _ => None,
            })
            .all(|image| image.transform.tx as f64 > body.transform.tx as f64)
    );
}

#[test]
fn picture_bullet_in_footnote_uses_numbering_relationships() {
    let bytes = patch(package("AA", true, true), "word/document.xml", |s| {
        s.replace(
            "<w:t>BODY</w:t>",
            "<w:t>BODY</w:t><w:footnoteReference w:id=\"2\"/>",
        )
    });
    let bytes = patch(bytes, "word/_rels/document.xml.rels", |s| {
        s.replace("</Relationships>", r#"<Relationship Id="notes" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/></Relationships>"#)
    });
    let bytes = patch(bytes, "word/footnotes.xml", |_| {
        r#"<w:footnotes xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:footnote w:id="2"><w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>NOTE_BODY</w:t></w:r></w:p></w:footnote></w:footnotes>"#.into()
    });
    let pages = crate::office::docx::testutil::pages(bytes);
    assert_eq!(image_count(&pages), 4);
    assert!(
        pages
            .iter()
            .flat_map(|p| &p.items)
            .any(|item| matches!(item, Item::Text(t) if t.text.contains("NOTE_BODY")))
    );
}

#[test]
fn picture_bullet_caller_image_limit_preserves_glyph_and_table() {
    let bytes = patch(package("A", true, true), "word/document.xml", |s| {
        s.replace(
            "<w:body>",
            "<w:body><w:tbl><w:tblGrid><w:gridCol w:w=\"6000\"/></w:tblGrid><w:tr><w:tc>",
        )
        .replace("</w:body>", "</w:tc></w:tr></w:tbl></w:body>")
    });
    let dir = std::env::temp_dir().join(format!("docx-picture-limit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("source.docx");
    std::fs::write(&input, bytes).unwrap();
    let output = dir.join("pages.jsonl");
    super::run_read(r#"{"limits":{"maxImagePixels":100},"text":{"enabled":true},"structtree":{"enabled":true}}"#, input.to_str().unwrap(), dir.join("index.json").to_str().unwrap(), output.to_str().unwrap()).unwrap();
    let page: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&output).unwrap().trim()).unwrap();
    let raw: String = page["text"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["unicode"].as_str())
        .collect();
    assert!(raw.starts_with("A"), "{raw:?}");
    assert!(raw.contains("BODY"));
    assert_eq!(page["structtree"]["children"][0]["sourceTextOnly"], true);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn picture_bullet_continuation_fragment_is_not_text_only() {
    let paragraph = |text: &str, numbered: bool| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="exact"/>{}</w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#,
            if numbered {
                r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>"#
            } else {
                ""
            }
        )
    };
    let left: String = (0..36)
        .map(|i| paragraph(&format!("MERGE{i:02}"), i == 0))
        .collect();
    let right: String = (0..36)
        .map(|i| paragraph(&format!("RIGHT{i:02}"), false))
        .collect();
    let body = format!(
        r#"{}<w:tbl><w:tblPr><w:tblW w:type="dxa" w:w="4800"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr>{left}</w:tc><w:tc>{}</w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc>{right}</w:tc></w:tr></w:tbl><w:sectPr><w:pgSz w:w="6000" w:h="6000"/><w:pgMar w:top="600" w:bottom="600" w:left="600" w:right="600"/></w:sectPr>"#,
        paragraph("START", false),
        paragraph("TOP", false)
    );
    let bytes = patch(package("A", true, true), "word/document.xml", |s| {
        let start = s.find("<w:body>").unwrap() + "<w:body>".len();
        let end = s.find("</w:body>").unwrap();
        format!("{}{}{}", &s[..start], body, &s[end..])
    });
    let dir = std::env::temp_dir().join(format!("docx-picture-merge-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("source.docx");
    std::fs::write(&input, bytes).unwrap();
    let output = dir.join("pages.jsonl");
    super::run_read(
        r#"{"text":{"enabled":true},"structtree":{"enabled":true}}"#,
        input.to_str().unwrap(),
        dir.join("index.json").to_str().unwrap(),
        output.to_str().unwrap(),
    )
    .unwrap();
    let pages: Vec<serde_json::Value> = std::fs::read_to_string(&output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(pages.len(), 2);
    let table = pages[1]["structtree"]["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["type"] == "Table")
        .unwrap();
    assert_eq!(table["sourceTextOnly"], false);
    assert_eq!(table["children"].as_array().unwrap().len(), 1);
    assert_eq!(table["children"][0]["sourceRowIndex"], 1);
    assert_eq!(
        table["children"][0]["children"][0]["sourceOwnerRowIndex"],
        0
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn picture_bullet_outline_scales_with_the_raster() {
    let drawing = r#"<w:drawing xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><wp:inline><wp:extent cx="15240000" cy="12700000"/><wp:docPr id="1" name="bullet"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="bullet"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="image"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="15240000" cy="12700000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:ln w="127000"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:ln></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing>"#;
    let bytes = patch(package("A", true, true), "word/lists/numbering.xml", |s| {
        let start = s.find("<w:pict").unwrap();
        let end = s.find("</w:pict>").unwrap() + "</w:pict>".len();
        format!("{}{}{}", &s[..start], drawing, &s[end..])
    });
    let pages = crate::office::docx::testutil::pages(bytes);
    assert_eq!(pages.len(), 1);
    let image = pages[0]
        .items
        .iter()
        .find_map(|i| {
            if let Item::Image(i) = i {
                Some(i)
            } else {
                None
            }
        })
        .unwrap();
    let outline = pages[0]
        .items
        .iter()
        .find_map(|i| {
            if let Item::Path(p) = i {
                p.stroke.as_ref().map(|s| (p, s))
            } else {
                None
            }
        })
        .unwrap();
    let bounds = outline.0.path.bounds();
    assert!((bounds.width() - image.transform.sx).abs() < 0.001);
    assert!((bounds.height() - image.transform.sy).abs() < 0.001);
    assert!((outline.1.width - 10.0 * image.transform.sy as f64 / 1000.0).abs() < 0.001);
}
