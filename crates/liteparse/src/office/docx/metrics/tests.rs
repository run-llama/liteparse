use super::*;

#[test]
fn aptos_table_uses_typo_metrics() {
    let fonts = FontSystem::new(&[], false, &Default::default());
    let wf = word_face(&fonts, "Aptos", false, false).expect("Aptos table");
    assert!(wf.use_typo);
    // 1923 + 577 over 2048 units: the same single line as Calibri.
    let (a, d) = line_metrics(&fonts, "Aptos", None, 12.0);
    assert!(((a + d) - 12.0 * 2500.0 / 2048.0).abs() < 1e-6);
    assert!(wf.advance('a').is_some_and(|w| w > 0));
    assert_eq!(wf.advance('\u{4E00}'), None);
}

#[test]
fn missing_style_takes_the_closest() {
    let fonts = FontSystem::new(&[], false, &Default::default());
    let f = word_face(&fonts, "Tahoma", true, true).expect("Tahoma table");
    assert!(f.bold);
}

#[test]
fn gdi_external_leading_for_tabled_office_fonts() {
    let fonts = FontSystem::new(&[], false, &Default::default());
    // Times New Roman: (1825 + 87) / 2048 above, 443 / 2048 below.
    let (a, d) = line_metrics(&fonts, "Times New Roman", None, 2048.0);
    assert_eq!((a, d), (1912.0, 443.0));
}

#[test]
fn symbol_line_is_taller_above_the_baseline() {
    let fonts = FontSystem::new(&[], false, &Default::default());
    // Symbol's win metrics: 2059 above, 450 below (a bullet label's line).
    let (a, d) = line_metrics(&fonts, "Symbol", None, 2048.0);
    assert_eq!((a, d), (2059.0, 450.0));
}

#[test]
fn office_families_without_a_clone_are_tabled() {
    let fonts = FontSystem::new(&[], false, &Default::default());
    for family in [
        "Franklin Gothic Book",
        "Gill Sans MT",
        "Tw Cen MT",
        "Century Schoolbook",
        "Lucida Sans",
        "Rockwell",
        "Arial Rounded MT Bold",
        "Agency FB",
        "Franklin Gothic Demi",
    ] {
        let wf = word_face(&fonts, family, false, false).expect(family);
        assert!(wf.advance('e').is_some_and(|w| w > 0), "{family}");
    }
    assert!(word_face(&fonts, "Gill Sans MT", true, true).is_some_and(|f| f.bold && f.italic));
}

#[test]
fn thai_office_fonts_have_tables_with_thai_advances() {
    let fonts = FontSystem::new(&[], false, &Default::default());
    for family in ["Angsana New", "Cordia New"] {
        let wf = word_face(&fonts, family, false, false).expect("Thai table");
        assert!(wf.advance('\u{0E01}').is_some_and(|w| w > 0), "{family}");
    }
}

#[test]
fn east_asian_fonts_take_1_3_times_their_windows_line_height() {
    // Reads a Founder name the GB2312 test tables; hold the lock across the reads.
    let _guard = FONT_TABLE_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let fonts = FontSystem::new(&[], false, &Default::default());
    let total = |family: &str| {
        let (a, d) = line_metrics(&fonts, family, None, 10.0);
        a + d
    };
    // SimSun's Windows height is 1 em, Microsoft YaHei's 1.32 em.
    assert!((total("SimSun") - 13.0).abs() < 1e-6);
    assert!((total("宋体") - 13.0).abs() < 1e-6);
    assert!((total("Microsoft YaHei") - 17.16).abs() < 1e-6);
    // Founder (FZ) families, absent here and not tabled as GB2312, space like
    // Microsoft YaHei.
    assert!((total("方正小标宋简体") - 17.16).abs() < 1e-6);
    assert!((total("FZXiaoBiaoSong-B05S") - 17.16).abs() < 1e-6);
    // DFKai-SB, absent here, also spaces like Microsoft YaHei; other mono
    // families keep 1 em.
    assert!((total("標楷體") - 17.16).abs() < 1e-6);
    assert!((total("DFKai-SB") - 17.16).abs() < 1e-6);
    assert!((total("KaiTi") - 13.0).abs() < 1e-6);
    // Western fonts keep their own single line.
    assert!((total("Times New Roman") - 10.0 * 2355.0 / 2048.0).abs() < 1e-6);
}

#[test]
fn mac_palatino_and_helvetica_take_word_for_macs_line_heights() {
    // Word for Mac spaces 10 pt Palatino at 11.04 pt (1.10 em) and Helvetica at
    // 12 pt (1.20 em), whatever face draws them.
    let fonts = FontSystem::new(&[], false, &Default::default());
    for (family, line) in [("Palatino", 11.0), ("Helvetica", 12.0)] {
        let (a, d) = line_metrics(&fonts, family, None, 10.0);
        assert!((a + d - line).abs() < 0.05, "{family}: {a} + {d}");
    }
}

#[test]
fn webdings_lines_are_one_em() {
    // Webdings' Windows metrics are 1638 + 410 of 2048 units: an 11 pt line is 11 pt,
    // whatever face draws its symbols.
    let fonts = FontSystem::new(&[], false, &Default::default());
    let (a, d) = line_metrics(&fonts, "Webdings", None, 11.0);
    assert!((a + d - 11.0).abs() < 1e-6, "{a} + {d}");
}

#[test]
fn a_missing_chinese_named_font_falls_back_to_microsoft_yahei() {
    // Word for Mac draws a missing font named only in Chinese (幼圆) with Microsoft
    // YaHei, Latin included; kana or Latin letters in a name keep it out.
    let fonts = FontSystem::new(&[], false, &Default::default());
    assert_eq!(effective_family(&fonts, "幼圆"), "Microsoft YaHei");
    assert_eq!(
        effective_family(&fonts, "游ゴシック Light"),
        "游ゴシック Light"
    );
    assert_eq!(
        effective_family(&fonts, "BIZ UD明朝 Medium"),
        "BIZ UD明朝 Medium"
    );
}

#[test]
fn a_missing_myriad_pro_draws_with_segoe_ui() {
    // Word for Mac lays out a missing Myriad Pro (and its weights) with Segoe UI.
    let fonts = FontSystem::new(&[], false, &Default::default());
    assert_eq!(effective_family(&fonts, "Myriad Pro"), "Segoe UI");
    assert_eq!(effective_family(&fonts, "Myriad Pro Light"), "Segoe UI");
}

#[test]
fn cordia_new_spaces_lines_by_its_hhea_metrics() {
    // Word for Mac spaces 14 pt Cordia New lines 16 pt apart (hhea 1.147 em, where
    // the Windows metrics would give 1.386 em).
    let fonts = FontSystem::new(&[], false, &Default::default());
    let (a, d) = line_metrics(&fonts, "Cordia New", None, 14.0);
    assert!((a + d - 16.06).abs() < 0.1, "{}", a + d);
}

#[test]
fn th_sarabun_lines_take_word_s_height() {
    // Word for Mac spaces single 16 pt TH Sarabun lines (substituted by TH SarabunPSK)
    // 20.9 pt apart, taller than the scaled open substitute's own metrics.
    let fonts = FontSystem::new(&[], false, &Default::default());
    for family in ["TH SarabunPSK", "TH SarabunIT๙"] {
        let (a, d) = line_metrics(&fonts, family, None, 16.0);
        assert!((a + d - 20.9).abs() < 0.1, "{family}: {}", a + d);
    }
}

#[test]
fn a_face_with_a_shorter_hhea_box_spaces_lines_by_it() {
    // Book Antiqua: win metrics give 1.243 em, hhea 1.206 em; Word for Mac spaces a
    // 12 pt line at 14.47 pt (21.7 pt at 1.5 lines).
    let fonts = FontSystem::new(&[], false, &Default::default());
    let (a, d) = line_metrics(&fonts, "Book Antiqua", None, 12.0);
    assert!((a + d - 14.47).abs() < 0.05, "{}", a + d);
}

#[test]
fn avenir_next_lt_pro_lines_take_its_own_height() {
    // 1.213 em, where its Manrope substitute's own metrics give 1.366 em.
    let fonts = FontSystem::new(&[], false, &Default::default());
    let (a, d) = line_metrics(&fonts, "Avenir Next LT Pro", None, 10.0);
    assert!((a + d - 12.13).abs() < 0.05, "{}", a + d);
}

#[test]
fn lucida_bright_takes_word_s_own_widths_and_line_height() {
    // Bundled with Word for Mac: a 10 pt line is 11.78 pt (hhea 1980 + 432 of 2048),
    // and bold picks its Demibold face's advances.
    let fonts = FontSystem::new(&[], false, &Default::default());
    let (a, d) = line_metrics(&fonts, "Lucida Bright", None, 10.0);
    assert!((a + d - 11.78).abs() < 0.02, "{}", a + d);
    let regular = word_face(&fonts, "Lucida Bright", false, false).unwrap();
    let bold = word_face(&fonts, "Lucida Bright", true, false).unwrap();
    assert!(bold.bold && bold.advance('a') > regular.advance('a'));
}

#[test]
fn a_legacy_one_em_hhea_box_keeps_win_line_height() {
    // Baskerville Old Face: hhea 1.0 em, win 1.141 em; Word for Mac spaces a 14 pt
    // line 16 pt apart, by win.
    let fonts = FontSystem::new(&[], false, &Default::default());
    let (a, d) = line_metrics(&fonts, "Baskerville Old Face", None, 14.0);
    assert!((a + d - 15.97).abs() < 0.05, "{}", a + d);
}

#[test]
fn browallia_new_lines_take_word_s_height() {
    // Word for Mac spaces single 16 pt Browallia New lines 18 pt apart.
    let fonts = FontSystem::new(&[], false, &Default::default());
    let (a, d) = line_metrics(&fonts, "Browallia New", None, 16.0);
    assert!((a + d - 18.0).abs() < 0.1, "{}", a + d);
}

#[test]
fn apple_color_emoji_lines_take_its_hhea_height() {
    // Word for Mac sizes a 12 pt emoji at 1.0 em ascent and 0.3125 em descent.
    let fonts = FontSystem::new(&[], false, &Default::default());
    let (a, d) = line_metrics(&fonts, "Apple Color Emoji", None, 12.0);
    assert!(
        (a - 12.0).abs() < 0.05 && (d - 3.75).abs() < 0.05,
        "{a} {d}"
    );
}

#[test]
fn the_east_asian_line_extra_goes_mostly_above_the_baseline() {
    // Word for Mac probe: a 12 pt SimSun line is 15.6 pt (1.3 em) with its baseline
    // 3.2 pt above the line bottom (SimSun's own descent is 1.7 pt): 45% of the
    // 3.6 pt extra goes below the baseline.
    let fonts = FontSystem::new(&[], false, &Default::default());
    let (a, d) = line_metrics_styled(&fonts, "SimSun", None, 12.0, false, false);
    assert!((a + d - 15.6).abs() < 0.01, "{a} {d}");
    assert!((d - 3.2).abs() < 0.15, "{a} {d}");
}

#[test]
fn a_noto_font_draws_with_its_font_table_alt_name() {
    let _guard = FONT_TABLE_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // Word for Mac has no Noto fonts: Google Docs' "Noto Sans Symbols" bullets draw
    // with the font table's altName (Calibri), even where the font is available.
    let table = crate::office::pptx::xml::parse_word(&format!(
        r#"<w:fonts xmlns:w="{}"><w:font w:name="Noto Sans Symbols"><w:altName w:val="Calibri"/></w:font></w:fonts>"#,
        crate::office::docx::testutil::W
    ))
    .unwrap();
    set_font_table(Some(&table));
    // A configured substitute makes the Noto family available here.
    let subs = [("Noto Sans Symbols".to_string(), "DejaVu Sans".to_string())].into();
    let fonts = FontSystem::new(&[], false, &subs);
    let got = effective_family(&fonts, "Noto Sans Symbols");
    set_font_table(None);
    assert_eq!(got, "Calibri");
}

#[test]
fn a_postscript_font_name_draws_with_its_font_table_alt_name() {
    let _guard = FONT_TABLE_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // Word for Mac does not resolve a PostScript name such as "Arial-BoldMT" and draws
    // the altName (a footer in Malgun Gothic Semilight, Malgun Gothic's metrics), even
    // where the name itself is available.
    let table = crate::office::pptx::xml::parse_word(&format!(
        r#"<w:fonts xmlns:w="{}"><w:font w:name="Arial-BoldMT"><w:altName w:val="Malgun Gothic Semilight"/></w:font><w:font w:name="Palatino-Roman"><w:altName w:val="Times New Roman"/></w:font></w:fonts>"#,
        crate::office::docx::testutil::W
    ))
    .unwrap();
    set_font_table(Some(&table));
    let subs = [
        ("Arial-BoldMT".to_string(), "DejaVu Sans".to_string()),
        ("Palatino-Roman".to_string(), "DejaVu Serif".to_string()),
    ]
    .into();
    let fonts = FontSystem::new(&[], false, &subs);
    let got = [
        effective_family(&fonts, "Arial-BoldMT"),
        effective_family(&fonts, "Palatino-Roman"),
    ];
    set_font_table(None);
    assert_eq!(got, ["Malgun Gothic", "Times New Roman"]);
}

#[test]
fn a_noto_font_without_an_alt_name_draws_with_arial() {
    let _guard = FONT_TABLE_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // Google Docs embeds "Noto Sans Symbols" for list bullets with no altName: Word for
    // Mac draws them with Arial at the body's line height.
    let table = crate::office::pptx::xml::parse_word(&format!(
        r#"<w:fonts xmlns:w="{}"><w:font w:name="Noto Sans Symbols"/></w:fonts>"#,
        crate::office::docx::testutil::W
    ))
    .unwrap();
    set_font_table(Some(&table));
    let subs = [("Noto Sans Symbols".to_string(), "DejaVu Sans".to_string())].into();
    let fonts = FontSystem::new(&[], false, &subs);
    let got = effective_family(&fonts, "Noto Sans Symbols");
    set_font_table(None);
    assert_eq!(got, "Arial");
}

#[test]
fn engravers_mt_lines_follow_word_s_own_font() {
    // Engravers MT ships with Word for Mac (Windows metrics 1920 + 465 of 2048): a
    // 22 pt line is 25.6 pt even without the font, where a fallback would be taller.
    let fonts = FontSystem::new(&[], false, &Default::default());
    let (a, d) = line_metrics(&fonts, "Engravers MT", None, 22.0);
    assert!((a + d - 22.0 * 2385.0 / 2048.0).abs() < 0.01, "{a} + {d}");
}

#[test]
fn a_missing_founder_gbk_font_draws_with_microsoft_yahei_under_a_gb2312_charset() {
    let _guard = FONT_TABLE_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // Word for Mac draws a missing 方正…简体 tabled as GB2312 with SimHei (1 em) but a
    // missing 方正…_GBK with the same charset with Microsoft YaHei (1.32 em).
    let table = crate::office::pptx::xml::parse_word(&format!(
        r#"<w:fonts xmlns:w="{}"><w:font w:name="方正仿宋_GBK"><w:charset w:val="86"/></w:font><w:font w:name="方正小标宋简体"><w:charset w:val="86"/></w:font></w:fonts>"#,
        crate::office::docx::testutil::W
    ))
    .unwrap();
    set_font_table(Some(&table));
    let fonts = FontSystem::new(&[], false, &Default::default());
    let total = |family: &str| {
        let (a, d) = line_metrics(&fonts, family, None, 10.0);
        a + d
    };
    let (gbk, simplified) = (total("方正仿宋_GBK"), total("方正小标宋简体"));
    let face = effective_family(&fonts, "方正仿宋_GBK");
    set_font_table(None);
    assert!((gbk - 17.16).abs() < 1e-6, "{gbk}");
    assert!((simplified - 13.0).abs() < 1e-6, "{simplified}");
    assert_eq!(face, "Microsoft YaHei");
}

/// A font with the given hhea (ascender, descender) and OS/2 win metrics, 1000 upm.
fn metrics_font(family: &str, hhea: (i16, i16), win: (u16, u16)) -> Vec<u8> {
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut hh = vec![0u8; 36];
    hh[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hh[4..6].copy_from_slice(&hhea.0.to_be_bytes());
    hh[6..8].copy_from_slice(&hhea.1.to_be_bytes());
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&1u16.to_be_bytes());
    let mut os2 = vec![0u8; 78];
    os2[4..6].copy_from_slice(&400u16.to_be_bytes());
    os2[6..8].copy_from_slice(&5u16.to_be_bytes());
    os2[62..64].copy_from_slice(&0x40u16.to_be_bytes());
    os2[74..76].copy_from_slice(&win.0.to_be_bytes());
    os2[76..78].copy_from_slice(&win.1.to_be_bytes());
    let utf16: Vec<u8> = family
        .encode_utf16()
        .flat_map(|u| u.to_be_bytes())
        .collect();
    let mut name = Vec::new();
    for v in [0u16, 2, 6 + 24] {
        name.extend_from_slice(&v.to_be_bytes());
    }
    for id in [1u16, 6] {
        for v in [3u16, 1, 0x409, id, utf16.len() as u16, 0] {
            name.extend_from_slice(&v.to_be_bytes());
        }
    }
    name.extend_from_slice(&utf16);
    let tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"OS/2", os2),
        (b"head", head),
        (b"hhea", hh),
        (b"maxp", maxp),
        (b"name", name),
    ];
    let mut out = Vec::new();
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&[0, 64, 0, 2, 0, 16]);
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

#[test]
fn an_installed_face_with_a_shorter_hhea_box_spaces_lines_by_it() {
    // hhea 1.2 em, win 1.3 em (as Roboto: 1.172 and 1.2): Word for Mac spaces a
    // 10 pt line at 12 pt; a legacy 1 em hhea box keeps the win height.
    let dir = std::env::temp_dir().join(format!("docx-read-hhea-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("a.ttf"),
        metrics_font("Hhea Probe", (950, -250), (1000, 300)),
    )
    .unwrap();
    std::fs::write(
        dir.join("b.ttf"),
        metrics_font("Legacy Probe", (800, -200), (1000, 300)),
    )
    .unwrap();
    let fonts = FontSystem::new(
        &[dir.to_string_lossy().into_owned()],
        false,
        &Default::default(),
    );
    let height = |family: &str| {
        let id = fonts.resolve(family, false, false).unwrap();
        let (a, d) = source_line_metrics(&fonts, id, 10.0);
        a + d
    };
    let (probe, legacy) = (height("Hhea Probe"), height("Legacy Probe"));
    drop(std::fs::remove_dir_all(&dir));
    assert!((probe - 12.0).abs() < 0.01, "{probe}");
    assert!((legacy - 13.0).abs() < 0.01, "{legacy}");
}
