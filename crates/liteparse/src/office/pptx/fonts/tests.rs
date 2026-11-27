use super::*;

#[test]
fn substitution_table_keys_are_lowercase() {
    for (k, _) in SUBSTITUTIONS {
        assert_eq!(*k, k.to_lowercase());
    }
}

#[test]
fn reads_panose_weight_and_serif_style() {
    // Aharoni: Latin text, sans (serif style 1 = any), weight 8 (bold).
    assert_eq!(panose_hints("02010803020104030203"), (true, false));
    // Times New Roman: serif style 2 (cove), weight 6.
    assert_eq!(panose_hints("02020603050405020304"), (false, true));
    assert_eq!(panose_hints("xx"), (false, false));
}

#[test]
fn normalizes_family_names() {
    assert_eq!(
        normalize_family("Arial,Sans-Serif"),
        ("Arial".to_string(), None)
    );
    assert_eq!(
        normalize_family("Segoe UI Semibold"),
        ("Segoe UI".to_string(), Some(600))
    );
    assert_eq!(
        normalize_family("Open Sans Light"),
        ("Open Sans".to_string(), Some(300))
    );
    assert_eq!(
        normalize_family("Arial Black"),
        ("Arial".to_string(), Some(900))
    );
    assert_eq!(
        normalize_family("SVYILS+ArialMT"),
        ("Arial".to_string(), None)
    );
    assert_eq!(
        normalize_family("Arial-BoldMT"),
        ("Arial".to_string(), Some(700))
    );
    let n = |s: &str| normalize_family(s);
    let some = |s: &str, w: u16| (s.to_string(), Some(w));
    assert_eq!(
        n("TimesNewRomanPSMT"),
        ("Times New Roman".to_string(), None)
    );
    assert_eq!(n("TimesNewRomanPS-BoldMT"), some("Times New Roman", 700));
    assert_eq!(n("OpenSans-SemiBold"), some("Open Sans", 600));
    assert_eq!(n("Calibri (Body)"), ("Calibri".to_string(), None));
    assert_eq!(n("Montserrat Semi Bold"), some("Montserrat", 600));
    assert_eq!(n("Lato Regular"), some("Lato", 400));
    assert_eq!(n("Montserrat Bold Italics"), some("Montserrat", 700));
    assert_eq!(n("Times New Roman"), ("Times New Roman".to_string(), None));
    assert_eq!(
        n("Bahnschrift SemiBold Condensed"),
        some("Bahnschrift Condensed", 600)
    );
    assert_eq!(n("Roboto Condensed Light"), some("Roboto Condensed", 300));
    assert_eq!(n("Segoe UI Semibold"), some("Segoe UI", 600));
    assert_eq!(n("Tw Cen MT"), ("Tw Cen".to_string(), None));
}

#[test]
fn names_variable_instances_by_weight() {
    assert_eq!(weight_style_name(700.0, false), "Bold");
    assert_eq!(weight_style_name(600.0, true), "SemiBoldItalic");
    assert_eq!(weight_style_name(400.0, true), "Italic");
    assert_eq!(weight_style_name(100.0, false), "Thin");
}

#[test]
fn substitute_scales_parse_and_shrink() {
    for (_, subs) in SUBSTITUTIONS {
        for s in subs.iter() {
            if let Some((name, scale)) = s.rsplit_once('*') {
                let v: f64 = scale.parse().unwrap();
                assert!(!name.is_empty() && v > 0.0 && v <= 1.0, "{s}");
            }
        }
    }
}

/// A minimal sfnt (head, hhea, maxp, and a name table with family and
/// PostScript names when `family` is given).
fn tiny_font(family: Option<&str>) -> Vec<u8> {
    tiny_font_weighted(family, None)
}

/// [`tiny_font`] with an OS/2 table declaring `weight` (700 and up marked bold).
fn tiny_font_weighted(family: Option<&str>, weight: Option<u16>) -> Vec<u8> {
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&1u16.to_be_bytes());
    let mut tables: Vec<(&[u8; 4], Vec<u8>)> =
        vec![(b"head", head), (b"hhea", hhea), (b"maxp", maxp)];
    if let Some(family) = family {
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
        tables.push((b"name", name));
    }
    if let Some(weight) = weight {
        let mut os2 = vec![0u8; 78];
        os2[4..6].copy_from_slice(&weight.to_be_bytes());
        os2[6..8].copy_from_slice(&5u16.to_be_bytes());
        let selection: u16 = if weight >= 700 { 0x20 } else { 0x40 };
        os2[62..64].copy_from_slice(&selection.to_be_bytes());
        tables.push((b"OS/2", os2));
    }
    tables.sort_by_key(|t| *t.0);
    let mut out = Vec::new();
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&[0, 32, 0, 1, 0, 0]);
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
fn embedded_fonts_fill_missing_families_by_slot() {
    let fonts = FontSystem::new(&[], false, &HashMap::new());
    assert_eq!(fonts.resolve("Sniglet", false, false), None);
    // Unnamed subset: registered under the deck's typeface anyway.
    assert!(fonts.add_embedded("Sniglet", false, false, tiny_font(None)));
    assert!(fonts.add_embedded("Sniglet", true, false, tiny_font(Some("Odd Subset"))));
    assert!(!fonts.add_embedded("Sniglet", false, true, b"not a font".to_vec()));
    let regular = fonts.resolve("Sniglet", false, false).unwrap();
    let bold = fonts.resolve("sniglet", true, false).unwrap();
    assert_ne!(regular, bold);
    // No italic slots: italic falls back to the same weight.
    assert_eq!(fonts.resolve("Sniglet", false, true), Some(regular));
    assert_eq!(fonts.resolve("Sniglet", true, true), Some(bold));
    assert_eq!(fonts.face(bold).unwrap().family, "Sniglet");
    assert!(fonts.has_family("Sniglet"));
    // The face's own name never answers lookups for an installed family.
    assert!(!fonts.has_family("Odd Subset"));
    assert!(fonts.substituted.borrow().is_empty());
}

#[test]
fn installed_family_beats_embedded_copy() {
    let dir = std::env::temp_dir().join(format!("pptx-read-fonts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sniglet.ttf"), tiny_font(Some("Sniglet"))).unwrap();
    let fonts = FontSystem::new(
        &[dir.to_string_lossy().into_owned()],
        false,
        &HashMap::new(),
    );
    let installed = fonts.resolve("Sniglet", false, false).unwrap();
    let fonts2 = FontSystem::new(
        &[dir.to_string_lossy().into_owned()],
        false,
        &HashMap::new(),
    );
    assert!(fonts2.add_embedded("Sniglet", false, false, tiny_font(Some("Sniglet"))));
    let chosen = fonts2.resolve("Sniglet", false, false).unwrap();
    assert!(!fonts2.embedded_names.borrow().contains_key(&chosen));
    assert_eq!(
        fonts2.postscript_name(chosen),
        fonts.postscript_name(installed)
    );
    drop(std::fs::remove_dir_all(&dir));
}

#[test]
fn scripts_prefer_their_own_fallback_faces() {
    assert_eq!(script_fallbacks('ب')[0], "Noto Naskh Arabic");
    assert_eq!(script_fallbacks('ש'), &["Noto Sans Hebrew"]);
    assert!(script_fallbacks('a').is_empty());
}

#[test]
fn requested_weight_reads_name_and_bold_flag() {
    // Per-character fallbacks are chosen at this weight, so "Arial Black" text its
    // substitute cannot draw stays black rather than regular.
    assert_eq!(requested_weight("Arial Black", false), 900);
    assert_eq!(requested_weight("Arial Black", true), 900);
    assert_eq!(requested_weight("Segoe UI Semibold", false), 600);
    assert_eq!(requested_weight("Segoe UI Semibold", true), 800);
    assert_eq!(requested_weight("Calibri Light", true), 700);
    assert_eq!(requested_weight("Arial", false), 400);
    assert_eq!(requested_weight("Arial", true), 700);
}

#[test]
fn weight_named_embedded_copy_beats_installed_base_family() {
    let dir = std::env::temp_dir().join(format!("pptx-read-fonts-w-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sniglet.ttf"), tiny_font(Some("Sniglet"))).unwrap();
    let fonts = FontSystem::new(
        &[dir.to_string_lossy().into_owned()],
        false,
        &HashMap::new(),
    );
    // Family names match regardless of case.
    let installed = fonts.resolve("sniglet", false, false).unwrap();
    assert!(!fonts.embedded_names.borrow().contains_key(&installed));
    assert!(fonts.substituted.borrow().is_empty());
    // Without an embedded copy, the weight-named family is the installed base.
    assert!(fonts.resolve("Sniglet Extra Bold", false, false).is_some());
    let fonts2 = FontSystem::new(
        &[dir.to_string_lossy().into_owned()],
        false,
        &HashMap::new(),
    );
    assert!(fonts2.add_embedded("Sniglet Extra Bold", false, false, tiny_font(None)));
    let chosen = fonts2.resolve("Sniglet Extra Bold", false, false).unwrap();
    assert!(fonts2.embedded_names.borrow().contains_key(&chosen));
    drop(std::fs::remove_dir_all(&dir));
}

#[test]
fn every_spelling_of_a_substituted_family_is_reported() {
    let dir = std::env::temp_dir().join(format!("pptx-read-subs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sniglet.ttf"), tiny_font(Some("Sniglet"))).unwrap();
    let subs: HashMap<String, String> = [("Arial".to_string(), "Sniglet".to_string())].into();
    let fonts = FontSystem::new(&[dir.to_string_lossy().into_owned()], false, &subs);
    // The second spelling is a cache hit; it is reported all the same, so the
    // warnings do not depend on which page asked first.
    let id = fonts.resolve("Arial", false, false);
    assert!(id.is_some());
    assert_eq!(fonts.resolve("arial", false, false), id);
    let got: Vec<(String, String)> = fonts.substituted.borrow().iter().cloned().collect();
    let pair = |a: &str| (a.to_string(), "Sniglet".to_string());
    assert_eq!(got, vec![pair("Arial"), pair("arial")]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn source_face_requires_exact_family_and_preserves_style_and_precedence() {
    let dir = std::env::temp_dir().join(format!("source-symbol-fonts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("arial.ttf"), tiny_font(Some("Arial"))).unwrap();
    let fonts = FontSystem::new(
        &[dir.to_string_lossy().into_owned()],
        false,
        &HashMap::new(),
    );
    assert!(fonts.resolve("Missing Source", false, false).is_some());
    assert!(fonts.source_face("Missing Source", false, false).is_none());
    let installed = fonts.source_face("Arial", false, false).unwrap();
    assert!(fonts.add_embedded("Arial", false, false, tiny_font(Some("Other Name"))));
    assert_eq!(fonts.source_face("Arial", false, false), Some(installed));
    assert!(fonts.add_embedded("Private Alias", false, false, tiny_font(None)));
    assert!(fonts.add_embedded("Private Alias", true, false, tiny_font(None)));
    let regular = fonts.source_face("Private Alias", false, false).unwrap();
    let bold = fonts.source_face("private alias", true, false).unwrap();
    assert_ne!(regular, bold);
    assert_eq!(
        fonts.source_face("Private Alias", false, true),
        Some(regular)
    );
    let subs = HashMap::from([("private alias".into(), "Arial".into())]);
    let substituted = FontSystem::new(&[dir.to_string_lossy().into_owned()], false, &subs);
    assert!(substituted.add_embedded("Private Alias", false, false, tiny_font(None)));
    assert!(substituted.resolve("Private Alias", false, false).is_some());
    assert!(
        substituted
            .source_face("Private Alias", false, false)
            .is_none()
    );
    drop(std::fs::remove_dir_all(dir));
}

#[test]
fn arial_narrow_keeps_a_narrow_face_without_liberation_sans_narrow() {
    // Debian's Liberation 2 has no Narrow cut; the generic sans is ~22% too wide.
    let dir = std::env::temp_dir().join(format!("pptx-read-narrow-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["Liberation Sans", "Barlow Semi Condensed"] {
        std::fs::write(dir.join(format!("{name}.ttf")), tiny_font(Some(name))).unwrap();
    }
    let fonts = FontSystem::new(
        &[dir.to_string_lossy().into_owned()],
        false,
        &HashMap::new(),
    );
    let id = fonts.resolve("Arial Narrow", false, false).unwrap();
    assert_eq!(fonts.face(id).unwrap().family, "Barlow Semi Condensed");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_light_request_takes_the_light_face_over_a_semilight_one() {
    // Selawik's Light and Semilight both declare weight 300.
    let mut db = fontdb::Database::new();
    for ps in ["Test-Semilight", "Test-Light"] {
        db.push_face_info(fontdb::FaceInfo {
            id: fontdb::ID::dummy(),
            source: fontdb::Source::Binary(std::sync::Arc::new(Vec::<u8>::new())),
            index: 0,
            families: vec![("Test".into(), fontdb::Language::English_UnitedStates)],
            post_script_name: ps.into(),
            style: fontdb::Style::Normal,
            weight: fontdb::Weight(300),
            stretch: fontdb::Stretch::Normal,
            monospaced: false,
        });
    }
    super::semilight_weights(&mut db);
    let id = db
        .query(&fontdb::Query {
            families: &[fontdb::Family::Name("Test")],
            weight: fontdb::Weight(300),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(db.face(id).unwrap().post_script_name, "Test-Light");
}

#[test]
fn a_missing_lucida_sans_takes_the_closer_width_dejavu_sans() {
    // Lucida Sans Unicode averages 0.53 em a character, DejaVu Sans 0.55 and IBM Plex
    // Sans 0.49: with both installed, DejaVu Sans keeps PowerPoint's line breaks.
    let dir = std::env::temp_dir().join(format!("pptx-read-lucida-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plex.ttf"), tiny_font(Some("IBM Plex Sans"))).unwrap();
    std::fs::write(dir.join("dejavu.ttf"), tiny_font(Some("DejaVu Sans"))).unwrap();
    let fonts = FontSystem::new(
        &[dir.to_string_lossy().into_owned()],
        false,
        &HashMap::new(),
    );
    let families: Vec<String> = ["Lucida Sans Unicode", "Lucida Sans", "Lucida Grande"]
        .iter()
        .map(|f| {
            let id = fonts.resolve(f, false, false).unwrap();
            fonts.face(id).unwrap().family.clone()
        })
        .collect();
    drop(std::fs::remove_dir_all(&dir));
    assert_eq!(families, ["DejaVu Sans"; 3]);
}

#[test]
fn bold_comic_sans_keeps_the_regular_face() {
    // PowerPoint for Mac draws bold Comic Sans MS with its regular face (probe: the
    // PDF holds ComicSansMS only), at regular widths.
    let dir = std::env::temp_dir().join(format!("pptx-read-comic-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("r.ttf"),
        tiny_font_weighted(Some("Comic Relief"), Some(400)),
    )
    .unwrap();
    std::fs::write(
        dir.join("b.ttf"),
        tiny_font_weighted(Some("Comic Relief"), Some(700)),
    )
    .unwrap();
    let fonts = FontSystem::new(
        &[dir.to_string_lossy().into_owned()],
        false,
        &HashMap::new(),
    );
    let regular = fonts.resolve("Comic Sans MS", false, false);
    let bold = fonts.resolve("Comic Sans MS", true, false);
    let other = fonts.resolve("Comic Relief", true, false);
    drop(std::fs::remove_dir_all(&dir));
    assert_ne!(other, regular);
    assert_eq!(bold, regular);
}

#[test]
fn a_no_bold_light_family_keeps_its_light_weight() {
    // Segoe UI Light has no bold: bold and regular runs both take its 300 weight
    // (PowerPoint probe), never the substitute's 400.
    let dir = std::env::temp_dir().join(format!("pptx-read-light-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("l.ttf"),
        tiny_font_weighted(Some("Selawik"), Some(300)),
    )
    .unwrap();
    std::fs::write(
        dir.join("r.ttf"),
        tiny_font_weighted(Some("Selawik"), Some(400)),
    )
    .unwrap();
    let fonts = FontSystem::new(
        &[dir.to_string_lossy().into_owned()],
        false,
        &HashMap::new(),
    );
    let light = fonts.resolve("Selawik Light", false, false);
    let regular = fonts.resolve("Selawik", false, false);
    let plain = fonts.resolve("Segoe UI Light", false, false);
    let bold = fonts.resolve("Segoe UI Light", true, false);
    drop(std::fs::remove_dir_all(&dir));
    assert_ne!(light, regular);
    assert_eq!(plain, light);
    assert_eq!(bold, light);
}
