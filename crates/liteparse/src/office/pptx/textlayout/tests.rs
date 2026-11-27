use super::*;

#[test]
fn formats_autonumbers() {
    assert_eq!(format_autonum("arabicPeriod", 3), "3.");
    assert_eq!(format_autonum("arabicParenR", 2), "2)");
    assert_eq!(format_autonum("romanUcPeriod", 4), "IV.");
    assert_eq!(format_autonum("alphaLcParenBoth", 28), "(bb)");
    assert_eq!(format_autonum("arabicPlain", 7), "7");
}

#[test]
fn classifies_scripts() {
    assert!(script_of('漢') == Script::EastAsian);
    assert!(script_of('ア') == Script::EastAsian);
    assert!(script_of('ب') == Script::Complex);
    assert!(script_of('a') == Script::Latin);
}

#[test]
fn single_line_height_is_1_2_times_the_size() {
    // Arial 24pt: metrics 1.117 em, PowerPoint pitch 28.8pt.
    assert!((natural_height(24.0, 21.72, 5.09, false) - 28.8).abs() < 1e-9);
    // Latin text in a font with oversized metrics still gets 1.2 em.
    assert!((natural_height(20.0, 21.0, 6.0, false) - 24.0).abs() < 1e-9);
    // A tall CJK face (1.5 em) keeps its own metrics.
    assert!((natural_height(20.0, 22.0, 8.0, true) - 30.0).abs() < 1e-9);
}

#[test]
fn visual_order_reverses_rtl_runs_inside_ltr() {
    // "abc ARABIC def": the RTL atoms (level 1) flip, the rest stays.
    assert_eq!(visual_order(&[0, 0, 1, 1, 1, 0]), vec![0, 1, 4, 3, 2, 5]);
    // RTL paragraph with an embedded LTR number (level 2).
    assert_eq!(visual_order(&[1, 1, 2, 2, 1]), vec![4, 2, 3, 1, 0]);
    assert_eq!(visual_order(&[0, 0]), vec![0, 1]);
}

#[test]
fn recognizes_proportional_japanese_fonts() {
    assert!(is_proportional_cjk("ＭＳ Ｐゴシック"));
    assert!(is_proportional_cjk("MS PGothic"));
    assert!(is_proportional_cjk("Meiryo UI"));
    assert!(is_proportional_cjk("BIZ UDPゴシック"));
    assert!(!is_proportional_cjk("ＭＳ ゴシック"));
    assert!(!is_proportional_cjk("Calibri"));
}

#[test]
fn autofit_scale_rounds_to_whole_points() {
    assert_eq!(scaled_size(Some(18.0), 0.925), 17.0);
    assert_eq!(scaled_size(Some(27.0), 0.9), 24.0);
    assert_eq!(scaled_size(Some(44.0), 0.9), 40.0);
    assert_eq!(scaled_size(Some(10.5), 1.0), 10.5);
}

/// Lays out one text box holding a paragraph of `runs` (`a:r` XML) with the
/// machine's fonts. `None` when no font is installed (nothing to measure).
fn text_scene(
    runs: &str,
) -> Option<(
    crate::office::pptx::scene::Scene,
    crate::office::pptx::fonts::FontSystem,
)> {
    text_scene_paras(&format!("<a:p>{runs}</a:p>"))
}

/// Like `text_scene`, for a body of whole `a:p` paragraphs.
fn text_scene_paras(
    paras: &str,
) -> Option<(
    crate::office::pptx::scene::Scene,
    crate::office::pptx::fonts::FontSystem,
)> {
    text_scene_with(paras, &Default::default())
}

/// Like `text_scene_paras`, with user font substitutions.
fn text_scene_with(
    paras: &str,
    subs: &std::collections::HashMap<String, String>,
) -> Option<(
    crate::office::pptx::scene::Scene,
    crate::office::pptx::fonts::FontSystem,
)> {
    use std::io::Write;
    // Paragraphs may open with their own `a:bodyPr`.
    let (body, paras) = match paras.strip_prefix("<a:bodyPr") {
        Some(rest) => {
            let end = rest.find("/>")? + 2;
            (format!("<a:bodyPr{}", &rest[..end]), &rest[end..])
        }
        None => (r#"<a:bodyPr wrap="none"/>"#.to_string(), paras),
    };
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, subs);
    fonts.resolve("Liberation Serif", false, false)?;
    fonts.resolve("Liberation Sans", false, false)?;
    let ns_r = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let rels = |kind: &str, target: &str| {
        format!(
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{ns_r}/{kind}" Target="{target}"/></Relationships>"#
        )
    };
    let sp = format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="6000000" cy="1000000"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr><p:txBody>{body}{paras}</p:txBody></p:sp>"#
    );
    let parts = [
        (
            "_rels/.rels".to_string(),
            rels("officeDocument", "ppt/presentation.xml"),
        ),
        (
            "ppt/presentation.xml".into(),
            format!(
                r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="{ns_r}"><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
            ),
        ),
        (
            "ppt/_rels/presentation.xml.rels".into(),
            rels("slide", "slides/slide1.xml"),
        ),
        (
            "ppt/slides/slide1.xml".into(),
            format!(
                r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="{ns_r}"><p:cSld><p:spTree>{sp}</p:spTree></p:cSld></p:sld>"#
            ),
        ),
    ];
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, body) in &parts {
            z.start_file(name.as_str(), zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }
    let cfg = crate::office::pptx::config::Config::default();
    let doc = crate::office::pptx::document::Document::open(buf.into_inner(), &cfg.limits).unwrap();
    let chain = doc.chain(&doc.slides[0].part).unwrap();
    let images = crate::office::pptx::render::ImageCache::new(cfg.limits.max_image_pixels, &fonts);
    let scene =
        crate::office::pptx::slide::SlideBuilder::new(&doc, &fonts, &images, &cfg, &chain, 1)
            .build();
    Some((scene, fonts))
}

fn glyph_runs(scene: &crate::office::pptx::scene::Scene) -> Vec<&GlyphRun> {
    scene
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Text(r) => Some(r),
            _ => None,
        })
        .collect()
}

#[test]
fn small_caps_flags_mark_lowercase_letters() {
    assert_eq!(
        small_caps_flags("aB1ß", Some("small")),
        vec![true, false, false, true, true]
    );
    assert!(small_caps_flags("ab", Some("all")).is_empty());
    assert_eq!(apply_caps("aB1ß", Some("small")).chars().count(), 5);
}

#[test]
fn small_caps_draw_lowercase_as_smaller_capitals() {
    let r = r#"<a:r><a:rPr lang="en-US" sz="2000" cap="small"><a:latin typeface="Liberation Sans"/></a:rPr><a:t>Small</a:t></a:r>"#;
    let Some((scene, _)) = text_scene(r) else {
        return;
    };
    let runs = glyph_runs(&scene);
    let texts: Vec<(&str, f64)> = runs.iter().map(|r| (r.text.as_str(), r.size)).collect();
    assert_eq!(texts, vec![("S", 20.0), ("MALL", 20.0 * SMALL_CAPS_SCALE)]);
    assert!(!runs[1].space_before);
}

#[test]
fn underline_follows_text_outline_and_u_fill() {
    let rpr = |extra: &str| {
        format!(
            r#"<a:r><a:rPr lang="en-US" sz="2000" u="sng"><a:ln w="12700"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:ln><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill>{extra}<a:latin typeface="Liberation Sans"/></a:rPr><a:t>Under</a:t></a:r>"#
        )
    };
    let underline = |scene: &crate::office::pptx::scene::Scene| {
        scene
            .items
            .iter()
            .find_map(|i| match i {
                Item::Path(p) if p.from_text => Some(p.clone()),
                _ => None,
            })
            .unwrap()
    };
    let Some((scene, _)) = text_scene(&rpr("")) else {
        return;
    };
    let p = underline(&scene);
    let Some(Paint::Solid(fill)) = p.fill else {
        panic!("{:?}", p.fill)
    };
    assert!(fill.g > 0.9 && fill.r < 0.1);
    let stroke = p.stroke.expect("outlined run strokes its underline");
    let Paint::Solid(line) = stroke.paint else {
        panic!()
    };
    assert!(line.r > 0.9 && line.g < 0.1);
    assert!((stroke.width - 1.0).abs() < 1e-6);

    let u_fill = r#"<a:uFill><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill></a:uFill>"#;
    let (scene, _) = text_scene(&rpr(u_fill)).unwrap();
    let Some(Paint::Solid(fill)) = underline(&scene).fill else {
        panic!()
    };
    assert!(fill.b > 0.9 && fill.g < 0.1);
}

#[test]
fn italic_is_synthesized_only_for_upright_faces() {
    let r = r#"<a:r><a:rPr lang="en-US" sz="2000" i="1"><a:latin typeface="Liberation Sans"/></a:rPr><a:t>Slant</a:t></a:r>"#;
    let Some((scene, fonts)) = text_scene(r) else {
        return;
    };
    let run = glyph_runs(&scene)[0];
    assert_eq!(run.fake_italic, !fonts.is_italic(run.face));
    let upright = r#"<a:r><a:rPr lang="en-US" sz="2000"><a:latin typeface="Liberation Sans"/></a:rPr><a:t>Up</a:t></a:r>"#;
    let (scene, _) = text_scene(upright).unwrap();
    assert!(!glyph_runs(&scene)[0].fake_italic);
}

#[test]
fn a_missing_thai_charset_font_takes_powerpoints_thai_fallback() {
    // Browallia New stands in for Liberation Serif so the fallback shows on Latin text.
    let subs = [("browallia new".to_string(), "Liberation Serif".to_string())].into();
    let run = |charset: &str| {
        format!(
            r#"<a:p><a:r><a:rPr lang="th-TH" sz="3000"><a:latin typeface="Jasmine News" {charset}/></a:rPr><a:t>Plan</a:t></a:r></a:p>"#
        )
    };
    let Some((thai, _)) = text_scene_with(&run(r#"charset="-34""#), &subs) else {
        return;
    };
    let (plain, _) = text_scene_with(&run(""), &subs).unwrap();
    assert_eq!(glyph_runs(&thai)[0].font_name, "Liberation Serif");
    assert_ne!(glyph_runs(&plain)[0].font_name, "Liberation Serif");
}

/// Baseline (slide y, points) of each drawn glyph run, in order.
fn baselines(paras: &str) -> Option<Vec<f64>> {
    let (scene, _) = text_scene_paras(paras)?;
    Some(
        glyph_runs(&scene)
            .iter()
            .map(|r| r.transform.ty as f64)
            .collect(),
    )
}

#[test]
fn spc_first_last_para_spaces_the_first_paragraph_too() {
    let para = r#"<a:p><a:pPr><a:spcBef><a:spcPts val="1000"/></a:spcBef></a:pPr><a:r><a:rPr lang="en-US" sz="1300"/><a:t>First</a:t></a:r></a:p>"#;
    let Some(plain) = baselines(para) else {
        return;
    };
    let spaced = baselines(&format!(
        r#"<a:bodyPr wrap="none" spcFirstLastPara="1"/>{para}"#
    ))
    .unwrap();
    assert!(
        (spaced[0] - plain[0] - 10.0).abs() < 0.01,
        "{plain:?} {spaced:?}"
    );
}

#[test]
fn spacing_above_100_percent_scales_only_the_first_lines_ascent() {
    let para = |pct: u32| {
        format!(
            r#"<a:p><a:pPr><a:lnSpc><a:spcPct val="{pct}"/></a:lnSpc></a:pPr><a:r><a:rPr lang="en-US" sz="2200"><a:latin typeface="Liberation Sans"/></a:rPr><a:t>One</a:t></a:r><a:br><a:rPr lang="en-US" sz="2200"/></a:br><a:r><a:rPr lang="en-US" sz="2200"><a:latin typeface="Liberation Sans"/></a:rPr><a:t>Two</a:t></a:r></a:p>"#
        )
    };
    let Some(single) = baselines(&para(100000)) else {
        return;
    };
    let spaced = baselines(&para(160000)).unwrap();
    // The first line drops by 60 % of its ascent, less than 60 % of its full
    // pitch; the second line keeps the full 160 % pitch.
    let pitch = |b: &[f64]| b[1] - b[0];
    let drop = spaced[0] - single[0];
    assert!(
        drop > 0.0 && drop < 0.6 * pitch(&single) - 2.0,
        "{single:?} {spaced:?}"
    );
    assert!(
        (pitch(&spaced) - 1.6 * pitch(&single)).abs() < 0.2,
        "{single:?} {spaced:?}"
    );
}

#[test]
fn a_long_path_after_a_space_starts_its_own_line() {
    let path = "/aaaaaaaa/bbbbbbbb/cccccccc/dddddddd/eeeeeeee/ffffffff/gggggggg/hhhhhhhh";
    let para = format!(
        r#"<a:bodyPr wrap="square"/><a:p><a:r><a:rPr lang="en-US" sz="2000"><a:latin typeface="Liberation Sans"/></a:rPr><a:t>GET {path}</a:t></a:r></a:p>"#
    );
    let Some((scene, _)) = text_scene_paras(&para) else {
        return;
    };
    let runs = glyph_runs(&scene);
    // The path wraps at the space, then inside itself only where the line is full.
    assert_eq!(
        runs[0].text.trim_end(),
        "GET",
        "{:?}",
        runs.iter().map(|r| &r.text).collect::<Vec<_>>()
    );
    assert!(
        runs[1].text.starts_with("/aaaaaaaa/bbbbbbbb/"),
        "{}",
        runs[1].text
    );
}

#[test]
fn raised_runs_do_not_grow_the_line() {
    // Superscript at 30 %: drawn at 2/3 of 36 pt, so each line is 1.2 × 24 pt and the
    // percentage spacing before is a fraction of that line too.
    let sup = |t: &str| {
        format!(
            r#"<a:p><a:pPr><a:spcBef><a:spcPct val="20000"/></a:spcBef></a:pPr><a:r><a:rPr lang="en-US" sz="3600" baseline="30000"><a:latin typeface="Liberation Sans"/></a:rPr><a:t>{t}</a:t></a:r></a:p>"#
        )
    };
    let Some(ys) = baselines(&format!("{}{}", sup("One"), sup("Two"))) else {
        return;
    };
    assert!((ys[1] - ys[0] - (28.8 + 0.2 * 28.8)).abs() < 1e-3, "{ys:?}");
    // An empty raised paragraph is as tall as a raised line.
    let empty = r#"<a:p><a:pPr><a:spcBef><a:spcPct val="20000"/></a:spcBef></a:pPr><a:endParaRPr lang="en-US" sz="3600" baseline="30000"/></a:p>"#;
    let ys = baselines(&format!("{}{empty}{}", sup("One"), sup("Two"))).unwrap();
    assert!((ys[1] - ys[0] - 2.0 * 1.2 * 28.8).abs() < 1e-3, "{ys:?}");
}

#[test]
fn blank_line_takes_the_size_of_its_break() {
    // The blank line between the two breaks is as tall as its 40 pt a:br, even with
    // an explicit, smaller endParaRPr (which sizes only the paragraph's last line).
    let run = |t: &str| {
        format!(
            r#"<a:r><a:rPr lang="en-US" sz="2000"><a:latin typeface="Liberation Sans"/></a:rPr><a:t>{t}</a:t></a:r>"#
        )
    };
    let br = r#"<a:br><a:rPr lang="en-US" sz="4000"/></a:br>"#;
    let body = format!(
        r#"<a:p>{}{br}{br}{}<a:endParaRPr lang="en-US" sz="1000"/></a:p>"#,
        run("A"),
        run("B")
    );
    let Some(ys) = baselines(&body) else {
        return;
    };
    assert!((ys[1] - ys[0] - (48.0 + 24.0)).abs() < 1e-3, "{ys:?}");
}

#[test]
fn negative_first_line_start_without_bullet_clamps_to_the_inset() {
    let first_x = |ppr: &str| {
        let (scene, _) = text_scene_paras(&format!(
            r#"<a:p><a:pPr {ppr}><a:buNone/></a:pPr><a:r><a:t>Hello</a:t></a:r></a:p>"#
        ))?;
        Some(glyph_runs(&scene)[0].transform.tx)
    };
    let Some(plain) = first_x(r#"marL="0" indent="0""#) else {
        return;
    };
    // marL 0 with an 18 pt outdent: PowerPoint starts the first line at the inset.
    let outdented = first_x(r#"marL="0" indent="-228600""#).unwrap();
    assert!((outdented - plain).abs() < 0.01, "{outdented} vs {plain}");
    // marL 36 pt, indent -18 pt: the first line still starts 18 pt in.
    let hanging = first_x(r#"marL="457200" indent="-228600""#).unwrap();
    assert!(hanging - plain > 1.0, "{hanging} vs {plain}");
}

#[test]
fn empty_runs_do_not_size_a_blank_line() {
    // Measured in PowerPoint: the spacer line takes its end mark's size (10 pt, so
    // 12 pt tall) whatever the empty run's size, at a paragraph end or between breaks.
    let a = r#"<a:r><a:rPr lang="en-US" sz="2000"><a:latin typeface="Liberation Sans"/></a:rPr><a:t>A</a:t></a:r>"#;
    let empty = r#"<a:r><a:rPr lang="en-US" sz="4000"/><a:t></a:t></a:r>"#;
    let end = r#"<a:endParaRPr lang="en-US" sz="1000"/>"#;
    let br = r#"<a:br><a:rPr lang="en-US" sz="1000"/></a:br>"#;
    let spacer_para = format!("<a:p>{a}</a:p><a:p>{empty}{end}</a:p><a:p>{a}</a:p>");
    let Some(ys) = baselines(&spacer_para) else {
        return;
    };
    assert!((ys[1] - ys[0] - (24.0 + 12.0)).abs() < 1e-3, "{ys:?}");
    let between_breaks = format!("<a:p>{a}{br}{empty}{br}{a}{end}</a:p>");
    let ys = baselines(&between_breaks).unwrap();
    assert!((ys[1] - ys[0] - (24.0 + 12.0)).abs() < 1e-3, "{ys:?}");
}

#[test]
fn a_bullet_font_replaces_every_script_slot_and_drops_the_texts_font_hints() {
    // The first run is Times New Roman (serif PANOSE, Thai complex script); the
    // bullet is Wingdings. None of the run's font choices may reach the bullet.
    let mut props = RunProps {
        latin: Some("Times New Roman".into()),
        latin_panose: Some("02020603050405020304".into()),
        cs: Some("Times New Roman".into()),
        cs_thai: Some(true),
        ..Default::default()
    };
    set_bullet_font(&mut props, "Wingdings");
    for slot in [&props.latin, &props.ea, &props.cs, &props.sym] {
        assert_eq!(slot.as_deref(), Some("Wingdings"));
    }
    assert_eq!(props.latin_panose, None);
    assert_eq!(props.cs_thai, None);
}

#[test]
fn a_tab_in_an_outdented_first_line_stops_at_the_hanging_indent() {
    // marL 40.5 pt, indent -40.5 pt: "\t1. text" puts "1." at 40.5 pt, in line with
    // the wrapped lines, before the 72 pt default stop.
    let hanging = ParaProps {
        mar_l: Some(40.5),
        indent: Some(-40.5),
        ..Default::default()
    };
    assert_eq!(next_tab(0.0, &hanging, 72.0), 40.5);
    // Past the margin, the default stops apply again.
    assert_eq!(next_tab(45.0, &hanging, 72.0), 72.0);
    // Without a hanging indent the margin is no stop.
    let flush = ParaProps {
        mar_l: Some(40.5),
        ..Default::default()
    };
    assert_eq!(next_tab(0.0, &flush, 72.0), 72.0);
}
