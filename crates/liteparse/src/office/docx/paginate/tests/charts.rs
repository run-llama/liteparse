use super::*;
use std::io::{Read, Write};

fn chart_package(bytes: Vec<u8>) -> Vec<u8> {
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for index in 0..input.len() {
        let mut file = input.by_index(index).unwrap();
        let name = file.name().unwrap().to_string();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        if name == "word/document.xml" {
            let mut xml = String::from_utf8(bytes).unwrap();
            let mut cursor = 0;
            while let Some(start) = xml[cursor..].find("<a:graphic>") {
                let start = cursor + start;
                let end = start + xml[start..].find("</a:graphic>").unwrap() + "</a:graphic>".len();
                let graphic = r#"<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rChart"/></a:graphicData></a:graphic>"#;
                xml.replace_range(start..end, graphic);
                cursor = start + graphic.len();
            }
            bytes = xml.into_bytes();
        } else if name == "word/_rels/document.xml.rels" {
            bytes = String::from_utf8(bytes).unwrap().replace("</Relationships>", r#"<Relationship Id="rChart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="charts/chart1.xml"/></Relationships>"#).into_bytes();
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    output
        .start_file(
            "word/charts/chart1.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    output.write_all(br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:order val="0"/><c:cat><c:strLit><c:ptCount val="1"/><c:pt idx="0"><c:v>CHART_LABEL</c:v></c:pt></c:strLit></c:cat><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>5</c:v></c:pt></c:numLit></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/><c:axPos val="b"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:axPos val="l"/><c:crossAx val="1"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#).unwrap();
    output.finish().unwrap().into_inner()
}

fn body_positions(
    pages: &[crate::office::docx::paginate::OutPage],
) -> Vec<(usize, String, f32, f32)> {
    pages
        .iter()
        .enumerate()
        .flat_map(|(page, p)| {
            p.items.iter().filter_map(move |item| {
                if let crate::office::pptx::scene::Item::Text(run) = item
                    && (run.text.contains("ANCHOR")
                        || run.text.contains("LINE")
                        || run.text.contains("NOTE")
                        || run.text.contains("MERGE")
                        || run.text.contains("LATER"))
                {
                    return Some((page, run.text.clone(), run.transform.tx, run.transform.ty));
                }
                None
            })
        })
        .collect()
}

#[test]
fn direct_charts_share_picture_note_and_column_retry_ownership() {
    for kind in [
        "existing",
        "pending",
        "later",
        "same-paragraph",
        "next-column",
        "table-column",
        "fresh-keep",
        "column-local",
        "column-pair",
        "column-impossible",
        "column-tall",
        "fresh-offset",
    ] {
        let bytes = picture_with_note(kind);
        let raster = pages(bytes.clone());
        let chart = pages(chart_package(bytes));
        assert_eq!(chart.len(), raster.len(), "{kind}");
        assert_eq!(body_positions(&chart), body_positions(&raster), "{kind}");
        let expected = raster
            .iter()
            .flat_map(|p| &p.items)
            .filter(|i| matches!(i, crate::office::pptx::scene::Item::Image(_)))
            .count();
        let labels = chart
            .iter()
            .map(page_texts)
            .collect::<String>()
            .matches("CHART_LABEL")
            .count();
        assert_eq!(
            labels, expected,
            "{kind}: chart paint once per source anchor"
        );
    }
}

#[test]
fn direct_chart_fit_preserves_front_behind_and_fixed_source_policies() {
    for behind in [false, true] {
        for (rel, offset, height, wrap) in [
            ("paragraph", 381000, 1905000, "Square"),
            ("paragraph", 9000000, 1905000, "Square"),
            ("paragraph", -381000, 1905000, "Square"),
            ("page", 3378200, 1905000, "Square"),
            ("margin", 2921000, 1905000, "Square"),
            ("paragraph", 381000, 4445000, "Square"),
            ("paragraph", 381000, 1905000, "None"),
        ] {
            let bytes = wrapped_picture_package(20, rel, offset, height, wrap, behind, false);
            let raster = pages(bytes.clone());
            let chart = pages(chart_package(bytes));
            assert_eq!(chart.len(), raster.len(), "{rel} {offset} {behind}");
            assert_eq!(
                body_positions(&chart),
                body_positions(&raster),
                "{rel} {offset} {behind}"
            );
        }
    }
}

#[test]
fn direct_chart_carry_and_retry_restore_note_and_exclusion_state() {
    let bytes = chart_package(picture_with_note("carry"));
    let opened = crate::office::docx::extract::open(bytes.clone(), &Default::default()).unwrap();
    with_column_pager(bytes, |pager, sections| {
        let mut sections = sections.to_vec();
        crate::office::docx::graphics::Graphics::new(
            opened.pkg.clone(),
            &opened.main,
            pager.env.fonts,
        )
        .sections(&mut sections);
        pager.pages[0].note_carry = Some((1, 0.0));
        let props = &sections[0].props;
        let units = crate::office::docx::layout::Flow::new(pager.env, 15).units(
            &sections[0].blocks,
            props.columns()[0].1,
            false,
        );
        pager.place_units(&units, 0, props);
        assert_eq!(pager.pages.len(), 2);
        let anchors: Vec<_> = pager
            .placed
            .iter()
            .filter_map(|placed| {
                if let super::super::Placed::Line {
                    unit,
                    line,
                    page,
                    alt,
                    ..
                } = placed
                    && let crate::office::docx::layout::Unit::Para(original) = &units[*unit]
                {
                    let p = alt.as_deref().unwrap_or(original);
                    if !p.lines[*line].anchors.is_empty() {
                        return Some(*page);
                    }
                }
                None
            })
            .collect();
        assert_eq!(anchors, vec![1]);
        let before = pager.snapshot();
        let notes = pager.pages.last().unwrap().notes.clone();
        let rects = pager.st.picture_rects.clone();
        assert!(!rects.is_empty());
        pager.st.picture_rects.clear();
        pager.st.picture_bottom += 100.0;
        pager.restore(&before);
        assert_eq!(pager.st.picture_rects, rects);
        assert_eq!(pager.st.picture_bottom, before.picture_bottom);
        assert_eq!(pager.pages.last().unwrap().notes, notes);
        pager.new_page(0, props, &sections);
        assert!(pager.st.picture_rects.is_empty());
        assert_eq!(pager.st.picture_bottom, 0.0);
    });
}

fn merged_anchor_package(bytes: Vec<u8>) -> Vec<u8> {
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for index in 0..input.len() {
        let mut file = input.by_index(index).unwrap();
        let name = file.name().unwrap().to_string();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        if name == "word/document.xml" {
            let mut xml = String::from_utf8(bytes).unwrap();
            let start = xml.find(r#"<w:p><w:pPr><w:spacing w:line="240""#).unwrap();
            let end = start + xml[start..].find("</w:p>").unwrap() + "</w:p>".len();
            let table = format!(
                r#"<w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr>{}</w:tc><w:tc><w:p><w:r><w:t>MERGE_START</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc><w:p><w:r><w:t>MERGE_END</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
                &xml[start..end]
            );
            xml.replace_range(start..end, &table);
            bytes = xml.into_bytes();
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    output.finish().unwrap().into_inner()
}

#[test]
fn direct_chart_in_merged_row_keeps_picture_trial_and_note_ownership() {
    for kind in ["existing", "pending"] {
        let bytes = merged_anchor_package(picture_with_note(kind));
        let raster = pages(bytes.clone());
        let chart = pages(chart_package(bytes));
        assert_eq!(chart.len(), raster.len(), "{kind}");
        assert_eq!(body_positions(&chart), body_positions(&raster), "{kind}");
        let text = chart.iter().map(page_texts).collect::<String>();
        for token in ["ANCHOR", "MERGE_START", "MERGE_END", "NOTE", "CHART_LABEL"] {
            assert_eq!(text.matches(token).count(), 1, "{kind} {token}");
        }
    }
}
