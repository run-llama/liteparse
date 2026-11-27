use super::*;
use crate::office::docx::testutil::{docx, pages, with_related_part};
use crate::office::pptx::scene::Item;

fn chart_drawing() -> &'static str {
    r#"<w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><wp:extent cx="2540000" cy="1524000"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rchart"/></a:graphicData></a:graphic></wp:inline></w:drawing>"#
}

fn package(direct: bool, table: bool) -> Vec<u8> {
    let drawing = chart_drawing();
    let mut inner = format!("<w:p><w:r>{drawing}</w:r></w:p>");
    if table {
        inner = format!(
            "<w:tbl><w:tblPr><w:tblW w:w=\"4000\" w:type=\"dxa\"/></w:tblPr><w:tblGrid><w:gridCol w:w=\"4000\"/></w:tblGrid><w:tr><w:tc>{inner}</w:tc></w:tr></w:tbl>"
        );
    }
    let outer = format!(
        r#"<w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wp:extent cx="3810000" cy="2540000"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:prstGeom prst="rect"/><a:noFill/></wps:spPr><wps:txbx><w:txbxContent>{inner}</w:txbxContent></wps:txbx><wps:bodyPr lIns="0" tIns="0" rIns="0" bIns="0"/></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
    );
    let direct = if direct {
        format!("<w:p><w:r>{drawing}</w:r></w:p>")
    } else {
        String::new()
    };
    let chart = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:ser><c:idx val="0"/><c:order val="0"/><c:cat><c:strRef><c:f>Sheet1!A1:A2</c:f><c:strCache><c:ptCount val="2"/><c:pt idx="0"><c:v>NESTED_A</c:v></c:pt><c:pt idx="1"><c:v>NESTED_B</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Sheet1!B1:B2</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="2"/><c:pt idx="0"><c:v>2</c:v></c:pt><c:pt idx="1"><c:v>5</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="b"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="l"/><c:crossAx val="1"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#;
    with_related_part(
        docx(&format!("{direct}{outer}"), None, None),
        "chart",
        "charts/chart1.xml",
        chart,
        false,
    )
}

#[test]
fn nested_only_chart_triggers_graphics_pass_and_emits_content() {
    for table in [false, true] {
        let bytes = package(false, table);
        let opened =
            crate::office::docx::extract::open(bytes.clone(), &Default::default()).unwrap();
        assert!(any(&opened.sections[0].blocks));
        let output = pages(bytes);
        let labels = output
            .iter()
            .flat_map(|page| &page.items)
            .filter(
                |item| matches!(item, crate::office::pptx::scene::Item::Text(run) if run.text == "NESTED_A"),
            )
            .count();
        assert_eq!(labels, usize::from(true));
        {
            assert!(
                output
                    .iter()
                    .flat_map(|page| &page.items)
                    .filter(|item| { matches!(item, crate::office::pptx::scene::Item::Path(_)) })
                    .count()
                    > 2,
                "chart axes and bars must be emitted"
            );
        }
    }
}

#[test]
fn direct_chart_and_nested_chart_each_render_once() {
    let output = pages(package(true, false));
    let labels = output
        .iter()
        .flat_map(|page| &page.items)
        .filter(|item| matches!(item, crate::office::pptx::scene::Item::Text(run) if run.text == "NESTED_A"))
        .count();
    assert_eq!(labels, 1 + usize::from(true));
}

fn grouped_package(children: &str) -> Vec<u8> {
    let source =
        crate::office::pptx::package::Package::open(package(false, false), &Default::default())
            .unwrap();
    let chart = source.read_string("word/charts/chart1.xml").unwrap();
    let body = format!(
        r#"<w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><wp:extent cx="2540000" cy="1524000"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"><wpg:wgp><wpg:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="1524000"/><a:chOff x="0" y="0"/><a:chExt cx="2540000" cy="1524000"/></a:xfrm></wpg:grpSpPr>{children}</wpg:wgp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
    );
    let bytes = with_related_part(
        docx(&body, None, None),
        "chart",
        "charts/chart1.xml",
        &chart,
        false,
    );
    with_related_part(
        bytes,
        "chart2",
        "charts/chart2.xml",
        &chart.replace("NESTED_A", "SECOND_A"),
        false,
    )
}

fn group_frame(rid: &str) -> String {
    format!(
        r#"<wpg:graphicFrame><wpg:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="1524000"/></wpg:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="{rid}"/></a:graphicData></a:graphic></wpg:graphicFrame>"#
    )
}

#[test]
fn pending_only_group_survives_parse_and_emits_chart_once() {
    let bytes = grouped_package(&group_frame("rchart"));
    let opened = crate::office::docx::extract::open(bytes.clone(), &Default::default()).unwrap();
    assert!(any(&opened.sections[0].blocks));
    let output = pages(bytes);
    assert_eq!(
        output
            .iter()
            .flat_map(|p| &p.items)
            .filter(|i| matches!(i,Item::Text(t) if t.text=="NESTED_A"))
            .count(),
        usize::from(true)
    );
}

#[test]
fn grouped_charts_keep_source_paint_order_and_adjacent_insertion_order() {
    let rect = |color: &str| {
        format!(
            r#"<wps:wsp><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="1524000"/></a:xfrm><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="{color}"/></a:solidFill><a:ln><a:noFill/></a:ln></wps:spPr></wps:wsp>"#
        )
    };
    let output = pages(grouped_package(&format!(
        "{}{}{}{}",
        rect("FF0000"),
        group_frame("rchart"),
        group_frame("rchart2"),
        rect("00FF00")
    )));
    let items: Vec<_> = output.iter().flat_map(|p| &p.items).collect();
    let labels: Vec<_> = items
        .iter()
        .enumerate()
        .filter_map(|(n, i)| {
            matches!(i,Item::Text(t) if t.text=="NESTED_A" || t.text=="SECOND_A").then_some(n)
        })
        .collect();
    assert_eq!(labels.len(), 2 * usize::from(true));
    if !labels.is_empty() {
        assert!(
            matches!(items[0], Item::Path(p) if matches!(&p.fill, Some(crate::office::pptx::scene::Paint::Solid(c)) if c.r == 1.0 && c.g == 0.0))
        );
        assert!(matches!(items[labels[0]],Item::Text(t) if t.text=="NESTED_A"));
        assert!(matches!(items[labels[1]],Item::Text(t) if t.text=="SECOND_A"));
        assert!(labels[0] > 0 && labels[1] > labels[0]);
        assert!(
            matches!(items.last().unwrap(), Item::Path(p) if matches!(&p.fill, Some(crate::office::pptx::scene::Paint::Solid(c)) if c.r == 0.0 && c.g == 1.0))
        );
        assert!(labels[1] < items.len() - 1);
    }
}

#[test]
fn missing_grouped_chart_keeps_relationship_diagnostic() {
    let opened = crate::office::docx::extract::open(
        grouped_package(&group_frame("rMissingGrouped")),
        &Default::default(),
    )
    .unwrap();
    let fonts = FontSystem::new(&[], true, &Default::default());
    let laid = crate::office::docx::extract::layout(&opened, &fonts);
    assert_eq!(laid.warnings.len(), usize::from(true));
    if let Some((code, detail)) = laid.warnings.first() {
        assert_eq!(code, "CHART_MISSING");
        assert!(detail.contains("word/document.xml [r:id=rMissingGrouped]"));
    }
}

#[test]
fn grouped_graphics_materialize_once_and_clear_detection() {
    let mut opened = crate::office::docx::extract::open(
        grouped_package(&group_frame("rchart")),
        &Default::default(),
    )
    .unwrap();
    let fonts = FontSystem::new(&[], true, &Default::default());
    let graphics = Graphics::new(opened.pkg.clone(), &opened.main, &fonts);
    graphics.sections(&mut opened.sections);
    assert!(!any(&opened.sections[0].blocks));
    let first = crate::office::docx::extract::layout(&opened, &fonts);
    graphics.sections(&mut opened.sections);
    let second = crate::office::docx::extract::layout(&opened, &fonts);
    let count = |laid: &crate::office::docx::extract::Laid| {
        laid.pages
            .iter()
            .flat_map(|p| &p.items)
            .filter(|i| matches!(i, Item::Text(t) if t.text == "NESTED_A"))
            .count()
    };
    assert_eq!(count(&first), usize::from(true));
    assert_eq!(count(&first), count(&second));
}

#[test]
fn direct_chart_provenance_excludes_grouped_and_unresolved_graphics() {
    let fonts = FontSystem::new(&[], true, &Default::default());
    for (bytes, direct) in [
        (package(true, false), true),
        (grouped_package(&group_frame("rchart")), false),
    ] {
        let opened = crate::office::docx::extract::open(bytes, &Default::default()).unwrap();
        let graphics = Graphics::new(opened.pkg.clone(), &opened.main, &fonts);
        let mut sections = opened.sections.clone();
        graphics.sections(&mut sections);
        let Block::Para(p) = &sections[0].blocks[0] else {
            panic!("paragraph")
        };
        let Inline::Picture(pic, _) = &p.inlines[0] else {
            panic!("picture")
        };
        assert_eq!(pic.shape.as_ref().unwrap().direct_chart, direct);
        assert!(pic.graphic.is_none());
        if direct {
            let Block::Para(original) = &opened.sections[0].blocks[0] else {
                panic!("paragraph")
            };
            let Inline::Picture(original, _) = &original.inlines[0] else {
                panic!("picture")
            };
            let (data, part) = original.graphic.as_ref().unwrap();
            assert!(
                !graphics
                    .render(data, part, original.w, original.h)
                    .unwrap()
                    .direct_chart
            );
            let mut missing = original.clone();
            missing.graphic = Some((data.clone(), "word/missing.xml".into()));
            graphics.picture(&mut missing);
            assert!(missing.shape.is_none());
        }
    }
}
