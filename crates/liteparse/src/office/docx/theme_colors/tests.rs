use super::*;
use crate::office::docx::{
    props::RPr,
    styles::{StyleSheet, ThemeFonts},
};

fn colors(mapping: &str) -> ThemeColors {
    let theme = crate::office::pptx::xml::parse_word(r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="test"><a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="4F81BD"/></a:accent2><a:dk1><a:sysClr val="windowText" lastClr="FFFFFF"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="202020"/></a:lt1></a:clrScheme></a:themeElements></a:theme>"#).unwrap();
    let settings = crate::office::pptx::xml::parse_word(&format!(
        r#"<w:settings xmlns:w="{}"><w:clrSchemeMapping {mapping}/></w:settings>"#,
        crate::office::docx::testutil::W
    ))
    .unwrap();
    ThemeColors::parse(Some(&theme), Some(&settings))
}

fn rpr(color: &str) -> RPr {
    let node = crate::office::pptx::xml::parse_word(&format!(
        r#"<w:rPr xmlns:w="{}"><w:color {color}/></w:rPr>"#,
        crate::office::docx::testutil::W
    ))
    .unwrap();
    RPr::parse(&node)
}

#[test]
fn theme_run_color_overrides_cache_and_survives_cascade() {
    let mut sheet = StyleSheet::parse(None, ThemeFonts::default());
    sheet.theme_colors = colors("");
    let mut inherited = rpr(r#"w:val="FF0000" w:themeColor="accent1" w:themeShade="BF""#);
    assert_eq!(
        sheet.run_style(&inherited).color.unwrap().to_argb_hex(),
        "FF2F5496"
    );
    inherited.apply(&rpr(
        r#"w:themeColor="accent2" w:themeTint="99" w:themeShade="00""#,
    ));
    assert_eq!(
        sheet.run_style(&inherited).color.unwrap().to_argb_hex(),
        "FF95B3D7"
    );
    inherited.apply(&rpr(r#"w:val="auto""#));
    assert_eq!(sheet.run_style(&inherited).color, None);
    inherited.apply(&rpr(r#"w:val="123456""#));
    assert_eq!(
        sheet.run_style(&inherited).color.unwrap().to_argb_hex(),
        "FF123456"
    );
}

#[test]
fn theme_mapping_system_policy_and_invalid_sources() {
    let colors = colors(r#"w:accent1="accent2" w:t2="accent1""#);
    let resolve = |source: &str| {
        colors
            .resolve(rpr(source).color.as_ref().unwrap())
            .map(|c| c.to_argb_hex())
    };
    assert_eq!(
        resolve(r#"w:themeColor="accent1""#).as_deref(),
        Some("FF4F81BD")
    );
    assert_eq!(
        resolve(r#"w:themeColor="text2""#).as_deref(),
        Some("FF4472C4")
    );
    assert_eq!(
        resolve(r#"w:themeColor="text1""#).as_deref(),
        Some("FF000000")
    );
    assert_eq!(
        resolve(r#"w:themeColor="background1""#).as_deref(),
        Some("FFFFFFFF")
    );
    assert_eq!(
        resolve(r#"w:val="123456" w:themeColor="text1" w:themeTint="00""#).as_deref(),
        Some("FF123456")
    );
    assert_eq!(
        resolve(r#"w:val="123456" w:themeColor="invalid""#).as_deref(),
        Some("FF123456")
    );
    assert_eq!(resolve(r#"w:val="invalid" w:themeColor="invalid""#), None);
    assert_eq!(resolve(r#"w:themeColor="accent1" w:themeTint="GG""#), None);
    assert_eq!(
        resolve(r#"w:themeColor="accent1" w:themeTint="00" w:themeShade="GG""#).as_deref(),
        Some("FFFFFFFF")
    );
    assert_eq!(
        resolve(r#"w:val=" #123456 " w:themeColor="invalid""#).as_deref(),
        Some("FF123456")
    );
    assert_eq!(
        resolve(r#"w:val="123456" w:themeColor="accent1" w:themeShade="0""#).as_deref(),
        Some("FF123456")
    );
    assert_eq!(
        ThemeColors::default()
            .resolve(
                rpr(r#"w:val="123456" w:themeColor="accent1" w:themeTint="00""#)
                    .color
                    .as_ref()
                    .unwrap()
            )
            .unwrap()
            .to_argb_hex(),
        "FF123456"
    );
}

#[test]
fn theme_color_inherits_through_default_table_character_and_extra_levels() {
    let xml = format!(
        r#"<w:styles xmlns:w="{}">
        <w:docDefaults><w:rPrDefault><w:rPr><w:color w:themeColor="accent1"/></w:rPr></w:rPrDefault></w:docDefaults>
        <w:style w:type="paragraph" w:styleId="Base"><w:rPr><w:color w:themeColor="accent2" w:themeTint="99"/></w:rPr></w:style>
        <w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Base"/></w:style>
        <w:style w:type="character" w:styleId="Char"><w:rPr><w:color w:val="123456"/></w:rPr></w:style>
        </w:styles>"#,
        crate::office::docx::testutil::W
    );
    let node = crate::office::pptx::xml::parse_word(&xml).unwrap();
    let mut sheet = StyleSheet::parse(Some(&node), ThemeFonts::default());
    sheet.theme_colors = colors("");
    let empty = RPr::default();
    let table = rpr(r#"w:themeColor="accent1" w:themeShade="BF""#);
    let extra = rpr(r#"w:themeColor="accent2""#);
    let get = |p, r, t, e| {
        sheet
            .run_style(&sheet.resolve_rpr(p, r, t, e))
            .color
            .unwrap()
            .to_argb_hex()
    };
    assert_eq!(get(None, &empty, None, None), "FF4472C4");
    assert_eq!(get(None, &empty, Some(&table), None), "FF2F5496");
    assert_eq!(get(Some("Child"), &empty, Some(&table), None), "FF95B3D7");
    assert_eq!(get(Some("Child"), &empty, None, Some(&extra)), "FF4F81BD");
    let character = RPr {
        r_style: Some("Char".into()),
        ..Default::default()
    };
    assert_eq!(get(Some("Child"), &character, None, None), "FF123456");
}

#[test]
fn theme_run_color_uses_related_parts_and_canonical_strict_attributes() {
    let dir = std::env::temp_dir().join(format!("docx-theme-color-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let body = r#"<w:p><w:r><w:rPr><w:color w:val="FF0000" w:themeColor="accent1"/></w:rPr><w:t>THEME</w:t></w:r></w:p>"#;
    let theme = r#"<a:theme xmlns:a="http://purl.oclc.org/ooxml/drawingml/main"><a:themeElements><a:clrScheme name="custom"><a:accent1><a:srgbClr val="FF0000"/></a:accent1><a:accent2><a:srgbClr val="0000FF"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#;
    let settings = r#"<w:settings xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main"><w:clrSchemeMapping w:accent1="accent2"/></w:settings>"#;
    let data = crate::office::docx::testutil::with_related_part(
        crate::office::docx::testutil::docx(body, None, None),
        "theme",
        "theme/custom.xml",
        theme,
        false,
    );
    let data = crate::office::docx::testutil::with_related_part(
        data,
        "settings",
        "config/custom.xml",
        settings,
        false,
    );
    let input = dir.join("source.docx");
    std::fs::write(&input, data).unwrap();
    let pages = dir.join("pages.jsonl");
    crate::office::docx::extract::run_read(
        r#"{"text":{"enabled":true}}"#,
        input.to_str().unwrap(),
        dir.join("index.json").to_str().unwrap(),
        pages.to_str().unwrap(),
    )
    .unwrap();
    let page: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&pages).unwrap().trim()).unwrap();
    let item = page["text"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["unicode"] == "THEME")
        .unwrap();
    assert_eq!(
        page["text"]["colors"][item["c"].as_u64().unwrap() as usize],
        "ff0000ff"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transformed_channels_floor_without_intermediate_hsl_quantization() {
    for (source, shade, expected) in [
        ("5B9BD5", 0x80, "FF1F4E79"),
        ("3CB6CE", 0x80, "FF1A5D6A"),
        ("156082", 0xBF, "FF0F4761"),
        ("FFFFFF", 0x80, "FF808080"),
    ] {
        let color = Rgba::from_hex(source).unwrap();
        assert_eq!(luminance(color, None, Some(shade)).to_argb_hex(), expected);
        assert_eq!(luminance(color, None, None), color);
        assert_eq!(luminance(color, None, Some(255)), color);
        assert_eq!(luminance(color, Some(0), Some(0)), Rgba::WHITE);
        assert_eq!(luminance(color, None, Some(0)), Rgba::BLACK);
    }
}

fn underline(attrs: &str) -> RPr {
    let node = crate::office::pptx::xml::parse_word(&format!(
        r#"<w:rPr xmlns:w="{}"><w:u {attrs}/></w:rPr>"#,
        crate::office::docx::testutil::W
    ))
    .unwrap();
    RPr::parse(&node)
}

#[test]
fn underline_theme_color_and_pattern_inherit_independently() {
    let mut sheet = StyleSheet::parse(None, ThemeFonts::default());
    sheet.theme_colors = colors("");
    let mut inherited =
        underline(r#"w:val="double" w:color="FF0000" w:themeColor="accent1" w:themeShade="BF""#);
    let resolved = sheet.run_style(&inherited);
    assert_eq!(resolved.underline.as_deref(), Some("double"));
    assert_eq!(resolved.underline_color.unwrap().to_argb_hex(), "FF2F5496");
    inherited.apply(&underline(r#"w:val="single""#));
    assert_eq!(
        sheet.run_style(&inherited).underline_color,
        resolved.underline_color
    );
    inherited.apply(&underline(
        r#"w:themeColor="accent2" w:themeTint="99" w:themeShade="00""#,
    ));
    let resolved = sheet.run_style(&inherited);
    assert_eq!(resolved.underline.as_deref(), Some("single"));
    assert_eq!(resolved.underline_color.unwrap().to_argb_hex(), "FF95B3D7");
    inherited.apply(&underline(""));
    assert_eq!(
        sheet.run_style(&inherited).underline_color,
        resolved.underline_color
    );
    // A transform without a theme reference does not supply a new color.
    inherited.apply(&underline(r#"w:themeTint="00" w:themeShade="00""#));
    assert_eq!(
        sheet.run_style(&inherited).underline_color,
        resolved.underline_color
    );
    inherited.apply(&underline(r#"w:color="auto""#));
    assert_eq!(sheet.run_style(&inherited).underline_color, None);
    assert_eq!(
        sheet.run_style(&inherited).underline.as_deref(),
        Some("single")
    );
    inherited.apply(&underline(r#"w:color="123456""#));
    assert_eq!(
        sheet
            .run_style(&inherited)
            .underline_color
            .unwrap()
            .to_argb_hex(),
        "FF123456"
    );
    inherited.apply(&underline(r#"w:val="none""#));
    assert_eq!(sheet.run_style(&inherited).underline, None);
    assert_eq!(
        sheet
            .run_style(&underline(r#"w:themeColor="accent1""#))
            .underline,
        None
    );
}

#[test]
fn underline_theme_color_shares_mapping_and_cached_fallback() {
    let mut sheet = StyleSheet::parse(None, ThemeFonts::default());
    sheet.theme_colors = colors(r#"w:accent1="accent2""#);
    for (attrs, expected) in [
        (r#"w:themeColor="accent1""#, "FF4F81BD"),
        (
            r#"w:color="345678" w:themeColor="text1" w:themeTint="00""#,
            "FF345678",
        ),
        (r#"w:color="123456" w:themeColor="invalid""#, "FF123456"),
        (
            r#"w:color="123456" w:themeColor="accent1" w:themeShade="GG""#,
            "FF123456",
        ),
    ] {
        assert_eq!(
            sheet
                .run_style(&underline(attrs))
                .underline_color
                .unwrap()
                .to_argb_hex(),
            expected
        );
    }
}

fn shaded(attrs: &str) -> RPr {
    RPr::parse(
        &crate::office::pptx::xml::parse_word(&format!(
            r#"<w:rPr xmlns:w="{}"><w:shd {attrs}/></w:rPr>"#,
            crate::office::docx::testutil::W
        ))
        .unwrap(),
    )
}

#[test]
fn shading_theme_colors_resolve_before_pattern_mixing_and_cascade() {
    let mut sheet = StyleSheet::parse(None, ThemeFonts::default());
    sheet.theme_colors = colors("");
    let mut source =
        shaded(r#"w:val="clear" w:fill="FF0000" w:themeFill="accent1" w:themeFillShade="BF""#);
    assert_eq!(
        sheet.run_style(&source).shading.unwrap().to_argb_hex(),
        "FF2F5496"
    );
    source.apply(&RPr::default());
    assert_eq!(
        sheet.run_style(&source).shading.unwrap().to_argb_hex(),
        "FF2F5496"
    );
    source.apply(&shaded(r#"w:val="solid" w:themeColor="accent2" w:themeTint="99" w:themeShade="00" w:themeFill="accent1""#));
    assert_eq!(
        sheet.run_style(&source).shading.unwrap().to_argb_hex(),
        "FF95B3D7"
    );
    source.apply(&shaded(r#"w:val="clear" w:fill="123456""#));
    assert_eq!(
        sheet.run_style(&source).shading.unwrap().to_argb_hex(),
        "FF123456"
    );
}

#[test]
fn shading_preserves_nil_compatibility_and_explicit_clear_reset() {
    let mut sheet = StyleSheet::parse(None, ThemeFonts::default());
    sheet.theme_colors = colors("");
    let mut inherited = shaded(r#"w:val="clear" w:themeFill="accent1""#);
    inherited.apply(&shaded(r#"w:val="nil""#));
    assert_eq!(
        sheet.run_style(&inherited).shading.unwrap().to_argb_hex(),
        "FF4472C4"
    );
    inherited.apply(&shaded(r#"w:val="clear" w:fill="auto""#));
    assert_eq!(sheet.run_style(&inherited).shading, None);
    assert_eq!(
        sheet
            .run_style(&shaded(r##"w:val="clear" w:fill="#123456""##))
            .shading,
        None
    );
    assert_eq!(
        sheet
            .run_style(&shaded(
                r#"w:val="nil" w:fill="123456" w:themeFill="accent1""#
            ))
            .shading
            .unwrap()
            .to_argb_hex(),
        "FF123456"
    );
}

#[test]
fn shading_resolves_both_channels_and_paragraph_style_background() {
    let mut sheet = StyleSheet::parse(None, ThemeFonts::default());
    sheet.theme_colors = colors("");
    let patterned = shaded(
        r#"w:val="pct50" w:themeFill="accent1" w:themeFillShade="BF" w:themeColor="accent2" w:themeTint="99""#,
    );
    assert_eq!(
        sheet.run_style(&patterned).shading.unwrap().to_argb_hex(),
        "FF6284B7"
    );
    let p = crate::office::pptx::xml::parse_word(&format!(r#"<w:pPr xmlns:w="{}"><w:shd w:val="clear" w:themeFill="accent1" w:themeFillShade="BF"/></w:pPr>"#, crate::office::docx::testutil::W)).unwrap();
    let mut inherited = crate::office::docx::props::PPr::parse(&p);
    inherited.apply(&crate::office::docx::props::PPr::default());
    assert_eq!(
        sheet
            .para_style(&inherited, 12.0)
            .shading
            .unwrap()
            .to_argb_hex(),
        "FF2F5496"
    );
}

#[test]
fn shading_theme_reaches_conditional_cells_and_cell_paragraphs() {
    let styles = r#"<w:style w:type="table" w:styleId="T"><w:tcPr><w:shd w:val="clear" w:themeFill="accent1"/></w:tcPr><w:pPr><w:shd w:val="clear" w:themeFill="accent2"/></w:pPr><w:rPr><w:shd w:val="clear" w:themeFill="accent1"/></w:rPr><w:tblStylePr w:type="firstRow"><w:tcPr><w:shd w:val="clear" w:themeFill="accent2"/></w:tcPr></w:tblStylePr></w:style>"#;
    let body = r#"<w:tbl><w:tblPr><w:tblStyle w:val="T"/><w:tblLook w:firstRow="1"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>FIRST</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:shd w:val="clear" w:fill="auto"/></w:tcPr><w:p><w:r><w:t>SECOND</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let theme = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="test"><a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="4F81BD"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#;
    let package = crate::office::docx::testutil::with_related_part(
        crate::office::docx::testutil::docx(body, Some(styles), None),
        "theme",
        "theme/custom.xml",
        theme,
        false,
    );
    let opened = crate::office::docx::extract::open(
        package,
        &crate::office::pptx::config::Config::default(),
    )
    .unwrap();
    let crate::office::docx::model::Block::Table(table) = &opened.sections[0].blocks[0] else {
        panic!("table")
    };
    assert_eq!(
        table.rows[0].cells[0].shading.unwrap().to_argb_hex(),
        "FF4F81BD"
    );
    assert_eq!(table.rows[1].cells[0].shading, None);
    let crate::office::docx::model::Block::Para(p) = &table.rows[0].cells[0].blocks[0] else {
        panic!("paragraph")
    };
    assert_eq!(p.style.shading.unwrap().to_argb_hex(), "FF4F81BD");
    assert_eq!(p.mark.shading.unwrap().to_argb_hex(), "FF4472C4");
}

#[test]
fn border_theme_color_resolves_after_side_and_run_cascade() {
    let mut sheet = StyleSheet::parse(None, ThemeFonts::default());
    sheet.theme_colors = colors("");
    let node = crate::office::pptx::xml::parse_word(&format!(r#"<w:rPr xmlns:w="{}"><w:bdr w:val="double" w:sz="16" w:space="3" w:color="FF0000" w:themeColor="accent1" w:themeShade="BF"/></w:rPr>"#, crate::office::docx::testutil::W)).unwrap();
    let run = sheet.run_style(&RPr::parse(&node));
    let border = run.bdr.unwrap();
    assert_eq!(
        crate::office::docx::layout::border_color(&border).to_argb_hex(),
        "FF2F5496"
    );
    assert_eq!(
        (border.style.as_str(), border.width, border.space),
        ("double", 2.0, 3.0)
    );
    let p = crate::office::pptx::xml::parse_word(&format!(r#"<w:pPr xmlns:w="{}"><w:pBdr><w:top w:val="single" w:themeColor="accent2" w:themeTint="99" w:themeShade="00"/><w:bottom w:val="nil" w:themeColor="accent1"/></w:pBdr></w:pPr>"#, crate::office::docx::testutil::W)).unwrap();
    let style = sheet.para_style(&crate::office::docx::props::PPr::parse(&p), 12.0);
    assert_eq!(
        crate::office::docx::layout::border_color(style.borders.top.as_ref().unwrap())
            .to_argb_hex(),
        "FF95B3D7"
    );
    assert!(!style.borders.bottom.as_ref().unwrap().visible());
}

#[test]
fn border_theme_paint_preserves_source_grouping_identity_and_fallback() {
    let palette = colors("");
    let border = |attrs: &str| {
        let node = crate::office::pptx::xml::parse_word(&format!(
            r#"<w:top xmlns:w="{}" w:val="single" {attrs}/>"#,
            crate::office::docx::testutil::W
        ))
        .unwrap();
        palette.border(&crate::office::docx::props::Border::parse(&node).unwrap())
    };
    let themed = border(r#"w:themeColor="accent1""#);
    let direct = border(r#"w:color="4472C4""#);
    assert_eq!(
        crate::office::docx::layout::border_color(&themed),
        crate::office::docx::layout::border_color(&direct)
    );
    assert_ne!(themed, direct);
    assert_eq!(themed, border(r#"w:themeColor="accent1""#));
    assert_eq!(
        border(r#"w:color="123456" w:themeColor="text1" w:themeTint="00""#).color,
        "123456"
    );
    assert_eq!(
        border(r#"w:color="123456" w:themeColor="invalid""#).color,
        "123456"
    );
    assert_eq!(
        border(r#"w:color="123456" w:themeColor="accent1" w:themeShade="GG""#).color,
        "123456"
    );
    assert_eq!(
        crate::office::docx::layout::border_color(&border("")),
        crate::office::pptx::color::Rgba::BLACK
    );
}

#[test]
fn border_theme_reaches_section_table_row_cell_and_diagonal_paths() {
    let styles = r#"<w:style w:type="table" w:styleId="T"><w:tblPr><w:tblBorders><w:insideH w:val="single" w:themeColor="accent1"/></w:tblBorders></w:tblPr><w:tblStylePr w:type="firstRow"><w:tcPr><w:tcBorders><w:top w:val="single" w:themeColor="accent2"/></w:tcBorders></w:tcPr></w:tblStylePr></w:style>"#;
    let body = r#"<w:tbl><w:tblPr><w:tblStyle w:val="T"/><w:tblLook w:firstRow="1"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tblPrEx><w:tblBorders><w:left w:val="single" w:themeColor="accent2"/></w:tblBorders></w:tblPrEx><w:tc><w:tcPr><w:tcBorders><w:tl2br w:val="single" w:themeColor="accent1"/><w:bottom w:val="nil" w:themeColor="accent1"/></w:tcBorders></w:tcPr><w:p><w:r><w:t>BORDER</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:sectPr><w:pgBorders><w:top w:val="single" w:themeColor="accent1"/></w:pgBorders></w:sectPr>"#;
    let theme = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="test"><a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="4F81BD"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#;
    let data = crate::office::docx::testutil::with_related_part(
        crate::office::docx::testutil::docx(body, Some(styles), None),
        "theme",
        "theme/custom.xml",
        theme,
        false,
    );
    let opened =
        crate::office::docx::extract::open(data, &crate::office::pptx::config::Config::default())
            .unwrap();
    let section = &opened.sections[0];
    let crate::office::docx::model::Block::Table(table) = &section.blocks[0] else {
        panic!("table")
    };
    assert_eq!(table.borders.inside_h.as_ref().unwrap().color, "4472C4");
    assert_eq!(
        table.rows[0]
            .borders
            .as_ref()
            .unwrap()
            .left
            .as_ref()
            .unwrap()
            .color,
        "4F81BD"
    );
    let cell = &table.rows[0].cells[0];
    assert_eq!(cell.borders.top.as_ref().unwrap().color, "4F81BD");
    assert_eq!(cell.diagonals[0].as_ref().unwrap().color, "4472C4");
    assert!(!cell.borders.bottom.as_ref().unwrap().visible());
    assert_eq!(
        section
            .props
            .pg_borders
            .as_ref()
            .unwrap()
            .borders
            .top
            .as_ref()
            .unwrap()
            .color,
        "4472C4"
    );
}
