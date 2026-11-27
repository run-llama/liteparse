use super::*;
use crate::office::pptx::config::Limits;
use std::{collections::HashMap, io::Write};

fn tiny_font(family: Option<&str>) -> Vec<u8> {
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

fn package(table: &str, rels: &str, data: &[u8]) -> Package {
    package_with(
        table,
        rels,
        data,
        false,
        "application/vnd.openxmlformats-officedocument.obfuscatedFont",
    )
}
fn package_with(table: &str, rels: &str, data: &[u8], strict: bool, kind: &str) -> Package {
    let r = if strict {
        "http://purl.oclc.org/ooxml/officeDocument/relationships"
    } else {
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    };
    let w = if strict {
        "http://purl.oclc.org/ooxml/wordprocessingml/main"
    } else {
        crate::office::docx::testutil::W
    };
    let xml = |body: &str| {
        format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
        )
    };
    let entries = [
        ("[Content_Types].xml", format!(r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="odttf" ContentType="{kind}"/></Types>"#).into_bytes()),
        ("custom/_rels/main.xml.rels", xml(&format!(r#"<Relationship Id="ft" Type="{r}/fontTable" Target="meta/fonts.xml"/>"#)).into_bytes()),
        ("custom/meta/fonts.xml", format!(r#"<w:fonts xmlns:w="{w}" xmlns:r="{r}">{table}</w:fonts>"#).into_bytes()),
        ("custom/meta/_rels/fonts.xml.rels", xml(rels).into_bytes()),
        ("custom/resources/font.odttf", data.to_vec()),
    ];
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        for (name, bytes) in entries {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    Package::open(buf.into_inner(), &Limits::default()).unwrap()
}
const ZERO: &str = "{00000000-0000-0000-0000-000000000000}";
const REL: &str = r#"<Relationship Id="f" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/font" Target="../resources/font.odttf"/>"#;
fn fonts() -> FontSystem {
    FontSystem::new(&[], false, &HashMap::new())
}

#[test]
fn guid_octets_reverse_and_repeat_over_exactly_first_32_bytes() {
    // Ascending source bytes XOR reversed GUID octets, specified independently.
    let encrypted = [vec![15; 16], vec![31; 16], vec![32, 33]].concat();
    assert_eq!(
        decode(encrypted, "{00010203-0405-0607-0809-0A0B0C0D0E0F}").unwrap(),
        (0..34).collect::<Vec<u8>>()
    );
    assert_eq!(decode(vec![42; 40], ZERO).unwrap(), vec![42; 40]);
}

#[test]
fn malformed_keys_and_short_parts_are_rejected() {
    for key in [
        "",
        "00000000000000000000000000000000",
        "00000000-0000-0000-0000-00000000000G",
        "{00000000-0000-0000-0000-000000000000",
        "0000000é-0000-0000-0000-00000000000",
    ] {
        assert!(decode(vec![0; 40], key).is_err(), "{key}");
    }
    assert!(decode(vec![0; 31], ZERO).is_err());
}

#[test]
fn related_font_table_registers_all_styles_in_document_local_system() {
    let table = format!(
        r#"<w:font w:name="Package Face"><w:embedRegular r:id="f" w:fontKey="{ZERO}"/><w:embedBold r:id="f" w:fontKey="{ZERO}"/><w:embedItalic r:id="f" w:fontKey="{ZERO}"/><w:embedBoldItalic r:id="f" w:fontKey="{ZERO}"/></w:font>"#
    );
    let pkg = package(&table, REL, &tiny_font(None));
    let system = fonts();
    assert!(!system.has_family("Package Face"));
    let warnings = register(&pkg, "custom/main.xml", &system);
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut ids = Vec::new();
    for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
        let id = system.resolve("Package Face", bold, italic).unwrap();
        assert_eq!(system.embedded_style(id), Some((bold, italic)));
        ids.push(id);
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 4);
    assert!(!fonts().has_family("Package Face"));
}

#[test]
fn invalid_relationships_and_payloads_are_nonfatal_and_contextual() {
    let table = format!(
        r#"<w:font w:name="Package Face"><w:embedRegular r:id="f" w:fontKey="{ZERO}"/></w:font>"#
    );
    for rels in [
        REL.replace("/font\"", "/image\""),
        REL.replace("Target=", "TargetMode=\"External\" Target="),
        String::new(),
        REL.replace("font.odttf", "missing.odttf"),
    ] {
        let warnings = register(
            &package(&table, &rels, &tiny_font(None)),
            "custom/main.xml",
            &fonts(),
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].1.contains("custom/meta/fonts.xml"));
    }
    assert_eq!(
        register(
            &package(&table, REL, b"not a font"),
            "custom/main.xml",
            &fonts()
        )
        .len(),
        1
    );
}

#[test]
fn official_ecma_376_part1_17_8_1_vector() {
    let bytes = |s: &str| {
        s.as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect::<Vec<_>>()
    };
    let plain = bytes("00010000001201000004002044534947A30EBFF30001368C000014DC4C545348");
    let obfuscated = bytes("AE1F8E94A00AED90D54E608A982352470D103167A019DA1CD54A747690244848");
    assert_eq!(
        decode(obfuscated, "001B70DC-AA60-4AD5-90EC-18A0948E1EAE").unwrap(),
        plain
    );
}

#[test]
fn strict_namespaces_raw_fonts_and_obfuscated_fonts_resolve() {
    let key = "001B70DC-AA60-4AD5-90EC-18A0948E1EAE";
    let raw = tiny_font(None);
    for (strict, key, kind, data) in [
        (true, ZERO, "application/x-font-ttf", raw.clone()),
        (
            false,
            key,
            "application/vnd.openxmlformats-officedocument.obfuscatedFont",
            decode(raw.clone(), key).unwrap(),
        ),
    ] {
        let table = format!(
            r#"<w:font w:name="Package Face"><w:embedRegular r:id="f" w:fontKey="{key}"/></w:font>"#
        );
        let rel = if strict {
            REL.replace(
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
                "http://purl.oclc.org/ooxml/officeDocument/relationships",
            )
        } else {
            REL.into()
        };
        let pkg = package_with(&table, &rel, &data, strict, kind);
        let system = fonts();
        assert!(register(&pkg, "custom/main.xml", &system).is_empty());
        assert!(system.has_family("Package Face"));
    }
    let table = format!(
        r#"<w:font w:name="Package Face"><w:embedRegular r:id="f" w:fontKey="{ZERO}"/></w:font>"#
    );
    let pkg = package_with(&table, REL, &raw, false, "application/x-fontdata");
    assert_eq!(register(&pkg, "custom/main.xml", &fonts()).len(), 1);
}

#[test]
fn malformed_font_warning_reaches_extract_index_without_aborting_document() {
    let table = format!(
        r#"<w:font w:name="Package Face"><w:embedRegular r:id="f" w:fontKey="{ZERO}"/></w:font>"#
    );
    let pkg = package(&table, REL, b"broken");
    let dir = std::env::temp_dir().join(format!("docx-embedded-index-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("input.docx");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&input).unwrap());
    for name in pkg.names() {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&pkg.read(name).unwrap()).unwrap();
    }
    for (name, data) in [
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="d" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="custom/main.xml"/></Relationships>"#,
        ),
        (
            "custom/main.xml",
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p/><w:sectPr/></w:body></w:document>"#,
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let index = dir.join("index.json");
    let pages = dir.join("pages.jsonl");
    crate::office::docx::extract::run_read(
        r#"{"render":{"noSystemFonts":true},"text":{"enabled":true}}"#,
        input.to_str().unwrap(),
        index.to_str().unwrap(),
        pages.to_str().unwrap(),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(index).unwrap()).unwrap();
    assert!(
        value["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "EMBEDDED_FONT_INVALID")
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn installed_family_keeps_precedence_over_package_font() {
    let dir = std::env::temp_dir().join(format!("docx-embedded-precedence-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("installed.ttf"), tiny_font(Some("Package Face"))).unwrap();
    let system = FontSystem::new(&[dir.to_str().unwrap().to_owned()], false, &HashMap::new());
    let installed = system.resolve("Package Face", false, false).unwrap();
    let table = format!(
        r#"<w:font w:name="Package Face"><w:embedRegular r:id="f" w:fontKey="{ZERO}"/></w:font>"#
    );
    assert!(
        register(
            &package(&table, REL, &tiny_font(None)),
            "custom/main.xml",
            &system
        )
        .is_empty()
    );
    assert_eq!(
        system.resolve("Package Face", false, false),
        Some(installed)
    );
    assert_eq!(system.embedded_style(installed), None);
    std::fs::remove_dir_all(dir).unwrap();
}
