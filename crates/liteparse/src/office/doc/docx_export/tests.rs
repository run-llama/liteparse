use super::convert;
use crate::office::doc::tests::{fixture, put16, put32};
use std::io::Read;

fn part(docx: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
    let mut out = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    out
}

/// The base fixture plus a "Data" stream (sectors 18..25) and its directory entry.
fn fixture_with_data_stream() -> Vec<u8> {
    let mut b = fixture();
    b.resize(27 * 512, 0);
    for id in 18..26 {
        put32(
            &mut b,
            512 + id * 4,
            if id == 25 { 0xffff_fffe } else { id as u32 + 1 },
        );
    }
    put32(&mut b, 1024 + 2 * 128 + 72, 3);
    let d = &mut b[1024 + 3 * 128..1024 + 4 * 128];
    for (j, u) in "Data".encode_utf16().enumerate() {
        put16(d, j * 2, u);
    }
    put16(d, 64, 10);
    d[66] = 2;
    d[67] = 1;
    for off in [68, 72, 76] {
        put32(d, off, 0xffff_ffff);
    }
    put32(d, 116, 18);
    put32(d, 120, 4096);
    b
}

#[test]
fn document_without_formatting_tables_still_converts() {
    let converted = convert(&fixture()).unwrap();
    let document = part(&converted.docx, "word/document.xml");
    assert!(document.contains(">Body</w:t>"));
    assert!(document.contains("<w:sectPr>"));
}

/// Main story `text` (at most 10 bytes, the fixture's piece; compressed at WordDocument 2048) with one CHPX FKP (page 7) whose
/// runs end at the given text offsets and carry the given modifiers.
fn story_with_runs(b: &mut [u8], text: &[u8], runs: &[(usize, &[u8])]) {
    let (w0, t0) = (1536, 5632);
    b[w0 + 2048..w0 + 2048 + text.len()].copy_from_slice(text);
    put32(b, w0 + 76, text.len() as u32);
    put32(b, w0 + 80, 0);
    // PlcBteChpx (pair 12) at table offset 128.
    put32(b, w0 + 154 + 12 * 8, 128);
    put32(b, w0 + 158 + 12 * 8, 12);
    for (k, v) in [2048u32, 2048 + text.len() as u32, 7]
        .into_iter()
        .enumerate()
    {
        put32(b, t0 + 128 + k * 4, v);
    }
    let fkp = w0 + 3584;
    put32(b, fkp, 2048);
    let mut at = 400;
    for (k, &(end, grpprl)) in runs.iter().enumerate() {
        put32(b, fkp + 4 + k * 4, 2048 + end as u32);
        b[fkp + 4 * (runs.len() + 1) + k] = (at / 2) as u8;
        b[fkp + at] = grpprl.len() as u8;
        b[fkp + at + 1..fkp + at + 1 + grpprl.len()].copy_from_slice(grpprl);
        at += 2 + grpprl.len() + grpprl.len() % 2;
    }
    b[fkp + 511] = runs.len() as u8;
}

/// Font table (pair 15) at table offset 256: (name, charset) per index.
fn font_table(b: &mut [u8], fonts: &[(&str, u8)]) {
    let (w0, t0) = (1536, 5632);
    let mut sttb = vec![fonts.len() as u8, 0, 0, 0];
    for &(name, charset) in fonts {
        let mut ffn = vec![0u8; 39];
        ffn[1] = 144; // weight 400
        ffn[2] = 1;
        ffn[3] = charset;
        ffn.extend(name.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        sttb.push(ffn.len() as u8);
        sttb.extend(ffn);
    }
    b[t0 + 256..t0 + 256 + sttb.len()].copy_from_slice(&sttb);
    put32(b, w0 + 154 + 15 * 8, 256);
    put32(b, w0 + 158 + 15 * 8, sttb.len() as u32);
}

#[test]
fn symbol_font_is_dropped_from_words_but_kept_on_single_letters() {
    let mut b = fixture();
    font_table(&mut b, &[("Times New Roman", 0), ("Symbol", 2)]);
    let symbol: &[u8] = &[0x4f, 0x4a, 1, 0];
    story_with_runs(&mut b, b"Para a\r", &[(5, symbol), (6, symbol), (7, &[])]);

    let document = part(&convert(&b).unwrap().docx, "word/document.xml");
    let run = |text: &str| {
        let end = document.find(&format!(">{text}</w:t>")).unwrap();
        document[document[..end].rfind("<w:r>").unwrap()..end].to_string()
    };
    assert!(!run("Para ").contains("Symbol"));
    assert!(run("a").contains("w:ascii=\"Symbol\""));
}

#[test]
fn pictures_in_toc_results_are_dropped_and_others_kept() {
    let mut b = fixture();
    // TOC field whose result holds a picture marker, then a picture outside any field.
    let text = b"\x13TOC\x14\x01\x15\x01\r";
    story_with_runs(&mut b, text, &[(text.len(), &[0x55, 0x08, 1])]);

    let diagnostics = convert(&b).unwrap().diagnostics;
    assert!(diagnostics.contains(&"toc_picture_dropped: 1".to_string()));
    assert!(diagnostics.contains(&"inline_picture_unresolved: 1".to_string()));
}

/// Two-cell row "A|B|" then "Tail", with the row's TDefTable stored in the Data stream
/// through sprmPHugePapx. Returns the file and the TC80 array offset.
fn huge_papx_table() -> (Vec<u8>, usize) {
    let mut b = fixture_with_data_stream();
    let (w0, t0, data0) = (1536, 5632, 512 + 18 * 512);
    // Main story "A|B||Tail" with cell marks: two cells, a row end, then a paragraph.
    b[w0 + 2048..w0 + 2058].copy_from_slice(b"A\x07B\x07\x07Tail\r");
    put32(&mut b, w0 + 76, 10);
    put32(&mut b, w0 + 80, 0);
    // PlcBtePapx (pair 13) at table offset 64: one FKP at page 6 of WordDocument.
    put32(&mut b, w0 + 154 + 13 * 8, 64);
    put32(&mut b, w0 + 158 + 13 * 8, 12);
    for (k, v) in [2048u32, 2058, 6].into_iter().enumerate() {
        put32(&mut b, t0 + 64 + k * 4, v);
    }
    let fkp = w0 + 3072;
    for (k, fc) in [2048u32, 2050, 2052, 2053, 2058].into_iter().enumerate() {
        put32(&mut b, fkp + k * 4, fc);
    }
    b[fkp + 511] = 4;
    for (k, word_offset) in [100u8, 100, 115, 0].into_iter().enumerate() {
        b[fkp + 20 + k * 13] = word_offset;
    }
    // Cell paragraphs: fInTable. Row end: fInTable, fTtp, sprmPHugePapx -> Data offset 0.
    b[fkp + 200..fkp + 206].copy_from_slice(&[3, 0, 0, 0x16, 0x24, 1]);
    b[fkp + 230..fkp + 246].copy_from_slice(&[
        0, 7, 0, 0, 0x16, 0x24, 1, 0x17, 0x24, 1, 0x46, 0x66, 0, 0, 0, 0,
    ]);
    // PrcData: sprmTDefTable with two cells whose edges are 0, 2000 and 5000 twips.
    put16(&mut b, data0, 51);
    b[data0 + 2..data0 + 7].copy_from_slice(&[0x08, 0xd6, 48, 0, 2]);
    for (k, x) in [0u16, 2000, 5000].into_iter().enumerate() {
        put16(&mut b, data0 + 7 + k * 2, x);
    }
    (b, data0 + 13)
}

#[test]
fn huge_papx_row_properties_rebuild_table_cells_and_grid() {
    let (b, _) = huge_papx_table();
    let document = part(&convert(&b).unwrap().docx, "word/document.xml");
    let table = &document[document.find("<w:tbl>").unwrap()..document.find("</w:tbl>").unwrap()];
    assert!(table.contains("<w:gridCol w:w=\"2000\"/><w:gridCol w:w=\"3000\"/>"));
    let cells: Vec<&str> = table.split("<w:tc>").skip(1).collect();
    assert_eq!(cells.len(), 2);
    assert!(cells[0].contains(">A</w:t>") && cells[1].contains(">B</w:t>"));
    assert!(document[document.find("</w:tbl>").unwrap()..].contains(">Tail</w:t>"));
}

#[test]
fn vertical_cell_flags_become_text_direction() {
    let (mut b, tc80) = huge_papx_table();
    // TC80 rgf: fVertical | fBackward on the first cell, fVertical alone on the second.
    put16(&mut b, tc80, 0x0c);
    put16(&mut b, tc80 + 20, 0x04);

    let document = part(&convert(&b).unwrap().docx, "word/document.xml");
    let cells: Vec<&str> = document.split("<w:tc>").skip(1).collect();
    assert!(cells[0].contains("<w:textDirection w:val=\"btLr\"/>"));
    assert!(cells[1].contains("<w:textDirection w:val=\"tbRl\"/>"));
}

#[test]
fn a_nil_cell_border_removes_the_table_border_on_that_side() {
    let (mut b, tc80) = huge_papx_table();
    // TC80 brcBottom of the first cell is brcNil (all bits set); the second cell's
    // borders stay zero.
    b[tc80 + 12..tc80 + 16].fill(0xff);

    let document = part(&convert(&b).unwrap().docx, "word/document.xml");
    let cells: Vec<&str> = document.split("<w:tc>").skip(1).collect();
    assert!(cells[0].contains("<w:tcBorders><w:bottom w:val=\"nil\"/></w:tcBorders>"));
    assert!(!cells[1].contains("<w:tcBorders>"));
}

/// One OfficeArt record: header (ver/instance, type, length) then body.
fn record(ver: u16, instance: u16, kind: u16, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(((instance << 4) | ver).to_le_bytes());
    out.extend(kind.to_le_bytes());
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

/// SpContainer with an FSP (shape type, id) and an OPT holding (property id, value) pairs.
fn shape_container(kind: u16, id: u32, opt: &[(u16, u32)]) -> Vec<u8> {
    let mut fsp = id.to_le_bytes().to_vec();
    fsp.extend(0u32.to_le_bytes());
    let mut props = Vec::new();
    for &(op, value) in opt {
        props.extend(op.to_le_bytes());
        props.extend(value.to_le_bytes());
    }
    let mut body = record(2, kind, 0xf00a, &fsp);
    body.extend(record(3, opt.len() as u16, 0xf00b, &props));
    record(0xf, 0, 0xf004, &body)
}

#[test]
fn shape_paint_honours_explicit_off_switches_and_drawing_order() {
    let shapes = [
        // Rectangle with fFilled off and a red (0x00BBGGRR) line.
        shape_container(1, 1025, &[(0x1bf, 0x10_0000), (0x1c0, 0x0000ff)]),
        // Textbox with fLine off and defaults otherwise.
        shape_container(202, 1026, &[(0x1ff, 0x8_0000)]),
    ]
    .concat();
    let mut content = record(0xf, 0, 0xf000, &[]);
    content.push(0);
    content.extend(record(0xf, 0, 0xf002, &record(0xf, 0, 0xf003, &shapes)));

    let styles = super::drawing::read(&content).styles;
    let (rect, textbox) = (styles[&1025], styles[&1026]);
    assert_eq!((rect.fill, rect.line), (None, Some(0xff0000)));
    assert_eq!((textbox.fill, textbox.line), (Some(0xffffff), None));
    assert!(textbox.z > rect.z);
}

#[test]
fn a_narrow_autofit_percentage_table_keeps_its_percentage_width() {
    use super::body::autofit_percent_width_of;
    // sprmTFAutofit (0x3615) on, sprmTTableWidth (0xF614) 44% (2200 fiftieths).
    let autofit = [0x15, 0x36, 1];
    let width = |w: i16| {
        let mut v = vec![0x14, 0xf6, 2];
        v.extend(w.to_le_bytes());
        v
    };
    let grpprl = |a: &[u8], w: i16| [a, &width(w)[..]].concat();
    assert_eq!(
        autofit_percent_width_of(&grpprl(&autofit, 2200), 1),
        Some(2200)
    );
    // Full width, several columns or a fixed-layout table keep fixed cell widths.
    assert_eq!(autofit_percent_width_of(&grpprl(&autofit, 5000), 1), None);
    assert_eq!(autofit_percent_width_of(&grpprl(&autofit, 2200), 2), None);
    assert_eq!(autofit_percent_width_of(&grpprl(&[], 2200), 1), None);
}

#[test]
fn an_unfilled_shape_does_not_draw_its_mapped_picture() {
    let shapes = [
        // Rectangle with fFilled off, rectangle left filled, picture frame with fFilled off.
        shape_container(1, 1025, &[(0x1bf, 0x10_0000)]),
        shape_container(1, 1026, &[]),
        shape_container(75, 1027, &[(0x1bf, 0x10_0000)]),
    ]
    .concat();
    let mut content = record(0xf, 0, 0xf000, &[]);
    content.push(0);
    content.extend(record(0xf, 0, 0xf002, &record(0xf, 0, 0xf003, &shapes)));
    let styles = super::drawing::read(&content).styles;
    let takes = |id| super::body::takes_mapped_picture(styles.get(&id));
    assert_eq!((takes(1025), takes(1026), takes(1027)), (false, true, true));
}

#[test]
fn direct_zero_autospacing_and_grid_snap_override_the_style() {
    // sprmPFDyaBeforeAuto 0, sprmPFDyaAfterAuto 0, sprmPFUsePgsuSettings 0.
    let ppr = super::props::ppr(
        &[0x5b, 0x24, 0, 0x5c, 0x24, 0, 0x47, 0x24, 0],
        &mut super::props::ParaFlags::default(),
    );
    assert!(ppr.contains("w:beforeAutospacing=\"0\""));
    assert!(ppr.contains("w:afterAutospacing=\"0\""));
    assert!(ppr.contains("<w:snapToGrid w:val=\"0\"/>"));
}

#[test]
fn table_lines_follow_the_grid_only_when_a_trusted_dop_asks() {
    let settings = |copts: u32, copy_noise: u32| {
        let mut b = fixture();
        let (w0, t0) = (1536, 5632);
        // DOP (pair 31) at table offset 1024, long enough to hold Copts at 512.
        put32(&mut b, w0 + 154 + 31 * 8, 1024);
        put32(&mut b, w0 + 158 + 31 * 8, 544);
        // Copts80 at 84 and its copy at 508 agree, so Copts at 512 is trusted.
        put32(&mut b, t0 + 1024 + 84, 0x8);
        put32(&mut b, t0 + 1024 + 508, 0x8 | copy_noise);
        put32(&mut b, t0 + 1024 + 512, copts);
        part(&convert(&b).unwrap().docx, "word/settings.xml")
    };
    assert!(settings(0, 0).contains("<w:adjustLineHeightInTable/>"));
    // fDontAdjustLineHeightInTable (bit 3).
    assert!(!settings(0x8, 0).contains("<w:adjustLineHeightInTable/>"));
    // A copy that disagrees means the DOP layout is not the one assumed.
    assert!(!settings(0, 0x10).contains("<w:adjustLineHeightInTable/>"));
}

#[test]
fn dop_typography_punctuation_compression_reaches_the_settings() {
    let settings = |typography: u16| {
        let mut b = fixture();
        let (w0, t0) = (1536, 5632);
        // DOP (pair 31) at table offset 1024; DopTypography starts at DOP offset 90.
        put32(&mut b, w0 + 154 + 31 * 8, 1024);
        put32(&mut b, w0 + 158 + 31 * 8, 544);
        put16(&mut b, t0 + 1024 + 90, typography);
        part(&convert(&b).unwrap().docx, "word/settings.xml")
    };
    // iJustification is bits 1-2; bit 0 (fKerningPunct) does not set it.
    assert!(settings(0x3).contains("w:val=\"compressPunctuation\""));
    assert!(settings(0x4).contains("w:val=\"compressPunctuationAndJapaneseKana\""));
    assert!(!settings(0x1).contains("characterSpacingControl"));
}

#[test]
fn group_members_keep_drawing_order_anchors_and_the_group_space() {
    let rect = |r: [i32; 4]| r.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
    // Group shape 1024 with coordinate space (0,0)-(1000,500), then two members.
    let mut group_shape = record(1, 0, 0xf009, &rect([0, 0, 1000, 500]));
    group_shape.extend(record(2, 0, 0xf00a, &[0, 4, 0, 0, 1, 0, 0, 0]));
    let member = |id: u32, anchor: [i32; 4]| {
        let mut body = record(
            2,
            202,
            0xf00a,
            &[id.to_le_bytes(), 0u32.to_le_bytes()].concat(),
        );
        body.extend(record(0, 0, 0xf00f, &rect(anchor)));
        record(0xf, 0, 0xf004, &body)
    };
    let mut group = record(0xf, 0, 0xf004, &group_shape);
    group.extend(member(1026, [500, 0, 1000, 250]));
    group.extend(member(1025, [0, 0, 500, 250]));
    let mut content = record(0xf, 0, 0xf000, &[]);
    content.push(0);
    let shapes = record(0xf, 0, 0xf003, &group);
    content.extend(record(0xf, 0, 0xf002, &record(0xf, 0, 0xf003, &shapes)));

    let drawing = super::drawing::read(&content);
    assert_eq!(drawing.groups[&1024], vec![1026, 1025]);
    assert_eq!(drawing.styles[&1024].group_space, Some([0, 0, 1000, 500]));
    assert_eq!(drawing.styles[&1025].child_anchor, Some([0, 0, 500, 250]));
}

#[test]
fn freeform_vertices_become_a_path_even_when_the_array_length_omits_its_header() {
    // OPT: pVertices (complex, length 8 = data only) and pSegmentInfo (length 12 with header).
    let mut fixed = Vec::new();
    for (op, value) in [(0xc145u16, 8u32), (0xc146, 12)] {
        fixed.extend(op.to_le_bytes());
        fixed.extend(value.to_le_bytes());
    }
    let mut complex = vec![2, 0, 2, 0, 0xf0, 0xff]; // two 16-bit-pair vertices
    complex.extend([0, 0, 0, 0, 100, 0, 50, 0]);
    complex.extend([3, 0, 3, 0, 2, 0]); // three segments: move, line, end
    complex.extend([0x01, 0x40, 0x01, 0x00, 0x00, 0x80]);
    let opt = record(3, 2, 0xf00b, &[fixed, complex].concat());
    let mut body = record(
        2,
        0,
        0xf00a,
        &[7u32.to_le_bytes(), 0u32.to_le_bytes()].concat(),
    );
    body.extend(opt);
    let mut content = record(0xf, 0, 0xf000, &[]);
    content.push(0);
    content.extend(record(0xf, 0, 0xf002, &record(0xf, 0, 0xf004, &body)));

    let path = &super::drawing::read(&content).paths[&7];
    assert!(path.contains("<a:moveTo><a:pt x=\"0\" y=\"0\"/></a:moveTo>"));
    assert!(path.contains("<a:lnTo><a:pt x=\"100\" y=\"50\"/></a:lnTo>"));
}

#[test]
fn unequal_columns_and_page_borders_reach_the_section() {
    let mut sepx = vec![0x0b, 0x50, 1, 0, 0x05, 0x30, 0]; // two columns, not evenly spaced
    for (sprm, index, value) in [
        (0xf203u16, 0u8, 3000u16),
        (0xf204, 0, 360),
        (0xf203, 1, 6000),
    ] {
        sepx.extend(sprm.to_le_bytes()); // spra 7: a fixed 3-byte operand
        sepx.push(index);
        sepx.extend(value.to_le_bytes());
    }
    sepx.extend([0x2b, 0x70, 8, 1, 0, 0]); // sprmSBrcTop80: single, 1pt
    sepx.extend([0x2f, 0x52, 0x20, 0]); // sprmSPgbProp: measured from the page edge

    let xml = super::props::section(&sepx).xml();
    assert!(
        xml.contains("w:equalWidth=\"0\"><w:col w:w=\"3000\" w:space=\"360\"/><w:col w:w=\"6000\"")
    );
    assert!(xml.contains(
        "<w:pgBorders w:offsetFrom=\"page\" w:display=\"allPages\"><w:top w:val=\"single\""
    ));
}

#[test]
fn section_page_number_format_reaches_the_section() {
    // sprmSNfcPgn 57: page numbers in dashes ("- 4 -"), without restarting.
    let xml = super::props::section(&[0x0e, 0x30, 57]).xml();
    assert!(
        xml.contains("<w:pgNumType w:fmt=\"numberInDash\"/>"),
        "{xml}"
    );
    // sprmSFPgnRestart with sprmSPgnStart: lower Roman from 3.
    let xml = super::props::section(&[0x0e, 0x30, 2, 0x11, 0x30, 1, 0x1c, 0x50, 3, 0]).xml();
    assert!(
        xml.contains("<w:pgNumType w:fmt=\"lowerRoman\" w:start=\"3\"/>"),
        "{xml}"
    );
}

#[test]
fn inline_picture_is_recovered_from_the_records_after_its_picf() {
    let png = b"\x89PNG\r\n\x1a\nrest-of-image";
    let mut blip_body = vec![0u8; 17]; // one UID and the tag byte
    blip_body.extend(png);
    let blip = record(0, 0x6e0, 0xf01e, &blip_body);
    let mut picf = vec![0u8; 68];
    picf[4] = 68; // cbHeader
    picf[6] = 100; // mapping mode without a name
    let mut data = picf;
    data.extend(record(0xf, 0, 0xf004, &[]));
    data.extend(blip);
    let len = data.len() as u32;
    data[..4].copy_from_slice(&len.to_le_bytes());

    let (format, bytes) = super::blips::inline(&data, 0, &[]).unwrap();
    assert_eq!((format, bytes.as_slice()), ("png", &png[..]));
}

#[test]
fn opposite_toggle_switches_off_a_property_the_style_turns_on() {
    let bold_opposite = [0x35, 0x08, 0x81];
    let in_bold_style = super::props::CharFlags {
        style_on: vec!["b"],
        ..Default::default()
    };
    let r = super::props::rpr(&bold_opposite, &[], &mut { in_bold_style });
    assert!(r.contains("<w:b w:val=\"0\"/>"));
    let r = super::props::rpr(&bold_opposite, &[], &mut Default::default());
    assert!(r.contains("<w:b/>"));
}

#[test]
fn style_toggles_resolve_along_the_based_on_chain() {
    let style = |base: Option<u16>, chpx: &[u8]| {
        Some(super::styles::Style {
            name: String::new(),
            kind: 1,
            base,
            next: 0,
            papx: Vec::new(),
            chpx: chpx.to_vec(),
        })
    };
    let styles = super::styles::Styles {
        // 0 sets bold; 1 flips it (0x81) and sets italic; 2 keeps bold as its base (0x80).
        styles: vec![
            style(None, &[0x35, 0x08, 1]),
            style(Some(0), &[0x35, 0x08, 0x81, 0x36, 0x08, 1]),
            style(Some(1), &[0x35, 0x08, 0x80]),
        ],
        default_fonts: [0, 0, 0],
    };
    assert_eq!(styles.toggles_on(0), vec!["b"]);
    assert_eq!(styles.toggles_on(2), vec!["i"]);
}

#[test]
fn comments_export_with_author_text_and_body_reference() {
    let mut b = fixture();
    let (w0, t0) = (1536, 5632);
    let spec: &[u8] = &[0x55, 0x08, 1];
    // Main story "A<ref>\r", then the annotation story "<mark>Hi\r" and a final mark.
    story_with_runs(
        &mut b,
        b"A\x05\r\x05Hi\r\r",
        &[(1, &[]), (2, spec), (3, &[]), (4, spec), (8, &[])],
    );
    put32(&mut b, w0 + 76, 3);
    put32(&mut b, w0 + 92, 5); // ccpAtn
    let mut refs = [1u32, 3]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<u8>>();
    refs.extend([0u8; 30]); // ATRD: no initials, author index 0
    let txt: Vec<u8> = [0u32, 4, 5].iter().flat_map(|v| v.to_le_bytes()).collect();
    let owners: Vec<u8> = [3u16]
        .iter()
        .chain("Ann".encode_utf16().collect::<Vec<_>>().iter())
        .flat_map(|v| v.to_le_bytes())
        .collect();
    for (pair, at, bytes) in [
        (4usize, 512usize, &refs),
        (5, 600, &txt),
        (36, 700, &owners),
    ] {
        b[t0 + at..t0 + at + bytes.len()].copy_from_slice(bytes);
        put32(&mut b, w0 + 154 + pair * 8, at as u32);
        put32(&mut b, w0 + 158 + pair * 8, bytes.len() as u32);
    }

    let docx = convert(&b).unwrap().docx;
    let comments = part(&docx, "word/comments.xml");
    assert!(comments.contains("w:author=\"Ann\""));
    assert!(comments.contains(">Hi</w:t>"));
    assert!(part(&docx, "word/document.xml").contains("<w:commentReference w:id=\"0\"/>"));
}

#[test]
fn behind_document_flag_is_read_only_when_marked_as_set() {
    let content = |value: u32| {
        let mut body = record(
            2,
            75,
            0xf00a,
            &[9u32.to_le_bytes(), 0u32.to_le_bytes()].concat(),
        );
        body.extend(record(
            3,
            1,
            0xf00b,
            &[&0x03bfu16.to_le_bytes()[..], &value.to_le_bytes()].concat(),
        ));
        let mut c = record(0xf, 0, 0xf000, &[]);
        c.push(0);
        c.extend(record(0xf, 0, 0xf002, &record(0xf, 0, 0xf004, &body)));
        super::drawing::read(&c).styles[&9].behind
    };
    assert_eq!(content(0x20_0020), Some(true));
    assert_eq!(content(0x20_0000), Some(false));
    assert_eq!(content(0x20), None);
}

#[test]
fn wordart_text_font_emphasis_and_rotation_are_read() {
    let text: Vec<u8> = "DRAFT\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let font: Vec<u8> = "Arial\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut opt = Vec::new();
    for (op, value) in [
        (0x0004u16, 315u32 << 16),
        (0x00ff, 0x20_0020),
        (0xc0c0, text.len() as u32),
        (0xc0c5, font.len() as u32),
    ] {
        opt.extend(op.to_le_bytes());
        opt.extend(value.to_le_bytes());
    }
    opt.extend(&text);
    opt.extend(&font);
    let mut body = record(
        2,
        136,
        0xf00a,
        &[3u32.to_le_bytes(), 0u32.to_le_bytes()].concat(),
    );
    body.extend(record(3, 4, 0xf00b, &opt));
    let mut content = record(0xf, 0, 0xf000, &[]);
    content.push(0);
    content.extend(record(0xf, 0, 0xf002, &record(0xf, 0, 0xf004, &body)));

    let drawing = super::drawing::read(&content);
    assert_eq!(
        drawing.wordart[&3],
        ("DRAFT".into(), "Arial".into(), (true, false))
    );
    assert_eq!(drawing.styles[&3].degrees(), 315.0);
}

#[test]
fn missing_fonts_get_an_alternate_from_their_name_or_family_class() {
    let mut b = fixture();
    let (w0, t0) = (1536, 5632);
    // FFN records: (name, charset, family byte with the class in bits 4-6).
    let mut sttb = vec![4u8, 0, 0, 0];
    for (name, charset, ff) in [
        ("CG Times", 0u8, 0x12u8),
        ("Calibri-Bold", 0, 0x22),
        ("仿宋", 134, 0x31),
        ("Odd", 0, 0x52),
    ] {
        let mut ffn = vec![0u8; 39];
        ffn[0] = ff;
        ffn[1] = 144;
        ffn[2] = 1;
        ffn[3] = charset;
        ffn.extend(name.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        sttb.push(ffn.len() as u8);
        sttb.extend(ffn);
    }
    b[t0 + 256..t0 + 256 + sttb.len()].copy_from_slice(&sttb);
    put32(&mut b, w0 + 154 + 15 * 8, 256);
    put32(&mut b, w0 + 158 + 15 * 8, sttb.len() as u32);

    let table = part(&convert(&b).unwrap().docx, "word/fontTable.xml");
    assert!(table.contains("w:name=\"CG Times\"><w:altName w:val=\"Times New Roman\"/>"));
    assert!(table.contains("w:name=\"Calibri-Bold\"><w:altName w:val=\"Calibri\"/>"));
    assert!(table.contains("w:name=\"仿宋\"><w:family"));
    assert!(table.contains("w:name=\"Odd\"><w:family w:val=\"decorative\"/>"));
}

#[test]
fn character_shading_reads_the_background_colour() {
    // sprmCShd: cb 10, cvFore auto, cvBack white (0x00FFFFFF), ipat clear.
    let grpprl = [0x71, 0xca, 10, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0, 0];
    let r = super::props::rpr(&grpprl, &[], &mut Default::default());
    assert!(r.contains("w:fill=\"FFFFFF\""), "{r}");
}

#[test]
fn shape_alignment_replaces_the_offset_except_vertically_against_a_paragraph() {
    use super::body::anchor_position;
    // Centred on the margin box both ways (posrelh/posrelv = margin).
    let (h, v) = anchor_position([Some(2), Some(0), Some(2), Some(0)], 0, 0, 100, 200);
    assert_eq!(
        h,
        "<wp:positionH relativeFrom=\"margin\"><wp:align>center</wp:align></wp:positionH>"
    );
    assert_eq!(
        v,
        "<wp:positionV relativeFrom=\"margin\"><wp:align>center</wp:align></wp:positionV>"
    );
    // No alignment: the FSPA frame and offset (twips to EMU) stay.
    let (h, _) = anchor_position([None; 4], 1, 2, 100, 200);
    assert_eq!(
        h,
        "<wp:positionH relativeFrom=\"page\"><wp:posOffset>63500</wp:posOffset></wp:positionH>"
    );
    // Vertical centre with the default paragraph frame keeps the offset.
    let (_, v) = anchor_position([None, None, Some(2), None], 2, 2, 0, 200);
    assert_eq!(
        v,
        "<wp:positionV relativeFrom=\"paragraph\"><wp:posOffset>127000</wp:posOffset></wp:positionV>"
    );
}

#[test]
fn cell_padding_sets_the_cells_top_and_bottom_margins() {
    let (mut b, _) = huge_papx_table();
    let fkp = 1536 + 3072;
    // sprmTCellPaddingDefault (top+bottom 40) then sprmTCellPadding for cell 0, all
    // four sides 170 twips: only its top and bottom reach the cell.
    let papx: &[u8] = &[
        0, 16, 0, 0, 0x16, 0x24, 1, 0x17, 0x24, 1, 0x34, 0xd6, 6, 0, 1, 5, 3, 40, 0, 0x32, 0xd6, 6,
        0, 1, 15, 3, 170, 0, 0x46, 0x66, 0, 0, 0, 0,
    ];
    b[fkp + 300..fkp + 300 + papx.len()].copy_from_slice(papx);
    b[fkp + 20 + 2 * 13] = 150;

    let document = part(&convert(&b).unwrap().docx, "word/document.xml");
    assert!(
        document.contains(
            "<w:tcMar><w:top w:w=\"170\" w:type=\"dxa\"/><w:bottom w:w=\"170\" w:type=\"dxa\"/></w:tcMar>"
        ),
        "{document}"
    );
    assert_eq!(document.matches("<w:tcMar>").count(), 1);
}

#[test]
fn default_cell_padding_sets_table_margins_and_indent() {
    let (mut b, _) = huge_papx_table();
    let fkp = 1536 + 3072;
    // Replace the row-end PAPX: fInTable, fTtp, then sprmTCellPaddingDefault twice
    // (top+bottom 40 twips, left+right 100 twips) and the huge PAPX reference.
    // cb' 16 words: istd, two toggles, two 9-byte padding sprms, sprmPHugePapx.
    let papx: &[u8] = &[
        0, 16, 0, 0, 0x16, 0x24, 1, 0x17, 0x24, 1, 0x34, 0xd6, 6, 0, 1, 5, 3, 40, 0, 0x34, 0xd6, 6,
        0, 1, 10, 3, 100, 0, 0x46, 0x66, 0, 0, 0, 0,
    ];
    b[fkp + 300..fkp + 300 + papx.len()].copy_from_slice(papx);
    b[fkp + 20 + 2 * 13] = 150; // row-end paragraph's PAPX moved to word offset 150

    let document = part(&convert(&b).unwrap().docx, "word/document.xml");
    assert!(document.contains(
        "<w:tblCellMar><w:top w:w=\"40\" w:type=\"dxa\"/><w:left w:w=\"100\" w:type=\"dxa\"/>\
         <w:bottom w:w=\"40\" w:type=\"dxa\"/><w:right w:w=\"100\" w:type=\"dxa\"/></w:tblCellMar>"
    ));
    // The first cell edge is at 0, so the indent is its left margin.
    assert!(document.contains("<w:tblInd w:w=\"100\" w:type=\"dxa\"/>"));
}

#[test]
fn default_cell_spacing_becomes_table_cell_spacing() {
    let (mut b, _) = huge_papx_table();
    let fkp = 1536 + 3072;
    // Row-end PAPX: fInTable, fTtp, sprmTCellSpacingDefault (all sides, 30 twips),
    // sprmTCellPaddingDefault (top+bottom 40) and the huge PAPX reference.
    let papx: &[u8] = &[
        0, 16, 0, 0, 0x16, 0x24, 1, 0x17, 0x24, 1, 0x33, 0xd6, 6, 0, 1, 15, 3, 30, 0, 0x34, 0xd6,
        6, 0, 1, 5, 3, 40, 0, 0x46, 0x66, 0, 0, 0, 0,
    ];
    b[fkp + 300..fkp + 300 + papx.len()].copy_from_slice(papx);
    b[fkp + 20 + 2 * 13] = 150;

    let document = part(&convert(&b).unwrap().docx, "word/document.xml");
    assert!(
        document.contains("<w:tblCellSpacing w:w=\"30\" w:type=\"dxa\"/>"),
        "{document}"
    );
}

#[test]
fn negative_line_spacing_is_exact_even_when_flagged_multiple() {
    // sprmPDyaLine: dyaLine -200, fMultLinespace 1.
    let ppr = super::props::ppr(&[0x12, 0x64, 0x38, 0xff, 1, 0], &mut Default::default());
    assert!(ppr.contains("w:line=\"200\" w:lineRule=\"exact\""), "{ppr}");
}

#[test]
fn shapes_carry_their_textbox_id_drawing_and_picture_crop() {
    let opt = |props: &[(u16, u32)]| {
        let bytes: Vec<u8> = props
            .iter()
            .flat_map(|&(o, v)| [o.to_le_bytes().to_vec(), v.to_le_bytes().to_vec()].concat())
            .collect();
        record(3, props.len() as u16, 0xf00b, &bytes)
    };
    let shape = |id: u32, props: &[(u16, u32)]| {
        let mut body = record(
            2,
            202,
            0xf00a,
            &[id.to_le_bytes(), 0u32.to_le_bytes()].concat(),
        );
        body.extend(opt(props));
        record(0xf, 0, 0xf004, &body)
    };
    let mut content = record(0xf, 0, 0xf000, &[]);
    // Main drawing (dgglbl 0): a textbox pointing at story entry 2; header drawing (1).
    content.push(0);
    content.extend(record(
        0xf,
        0,
        0xf002,
        &shape(1028, &[(0x80, 0x2_0000), (0x100, 0x8000)]),
    ));
    content.push(1);
    content.extend(record(0xf, 0, 0xf002, &shape(2050, &[(0x80, 0x1_0000)])));

    let drawing = super::drawing::read(&content);
    let (main, header) = (drawing.styles[&1028], drawing.styles[&2050]);
    assert_eq!((main.txid, main.header), (Some(0x2_0000), false));
    assert_eq!((header.txid, header.header), (Some(0x1_0000), true));
    assert_eq!(main.crop, [0x8000, 0, 0, 0]); // half cropped from the top
}

#[test]
fn inline_picture_crop_comes_from_the_shape_after_its_picf() {
    let mut data = vec![0u8; 68];
    data[4] = 68;
    data[6] = 100;
    let mut props = Vec::new();
    for (op, value) in [(0x101u16, 0x4000u32), (0x102, 0x2000)] {
        props.extend(op.to_le_bytes());
        props.extend(value.to_le_bytes());
    }
    data.extend(record(0xf, 0, 0xf004, &record(3, 2, 0xf00b, &props)));
    let len = data.len() as u32;
    data[..4].copy_from_slice(&len.to_le_bytes());
    // top, bottom, left, right
    assert_eq!(super::blips::inline_crop(&data, 0), [0, 0x4000, 0x2000, 0]);
}

#[test]
fn frame_modifiers_become_frame_properties() {
    // pc 0x50 (vertical page, horizontal margin), dxaAbs -4 (centre), dyaAbs 6805,
    // dxaWidth 3000, wr 1.
    let grpprl = [
        0x1b, 0x26, 0x50, 0x18, 0x84, 0xfc, 0xff, 0x19, 0x84, 0x95, 0x1a, 0x1a, 0x84, 0xb8, 0x0b,
        0x23, 0x24, 1,
    ];
    let ppr = super::props::ppr(&grpprl, &mut Default::default());
    assert!(
        ppr.starts_with(
            "<w:framePr w:w=\"3000\" w:wrap=\"notBeside\" w:hAnchor=\"margin\" w:vAnchor=\"page\" \
         w:xAlign=\"center\" w:y=\"6805\"/>"
        ),
        "{ppr}"
    );
    // No frame modifiers, no frame.
    assert!(!super::props::ppr(&[0x03, 0x24, 1], &mut Default::default()).contains("framePr"));
}

#[test]
fn paragraph_frame_keeps_the_anchors_of_its_style_frame() {
    // The base style anchors its frame to the page horizontally (pc 0xa0: page, text);
    // the derived style moves it; the paragraph sets only dxaAbs.
    let style = |base, papx: &[u8]| {
        Some(super::styles::Style {
            name: String::new(),
            kind: 1,
            base,
            next: 0,
            papx: papx.to_vec(),
            chpx: Vec::new(),
        })
    };
    let styles = super::styles::Styles {
        styles: vec![
            style(None, &[0x1b, 0x26, 0xa0, 0x18, 0x84, 0x64, 0x00]),
            style(Some(0), &[0x19, 0x84, 0xc8, 0x00, 0x03, 0x24, 1]),
        ],
        default_fonts: [0, 0, 0],
    };
    let mut grpprl = styles.frame_sprms(1);
    grpprl.extend_from_slice(&[0x18, 0x84, 0x2c, 0x01]);
    let ppr = super::props::ppr(&grpprl, &mut Default::default());
    assert!(
        ppr.starts_with(
            "<w:framePr w:wrap=\"around\" w:hAnchor=\"page\" w:vAnchor=\"text\" \
         w:x=\"300\" w:y=\"200\"/>"
        ),
        "{ppr}"
    );
}

#[test]
fn deletion_mark_with_the_opposite_of_style_operand_is_deleted() {
    // sprmCFRMarkDel (0x0800): 1 and 0x81 ("opposite of the style", which has none)
    // both mark deleted revision text; 0 and 0x80 do not.
    for (operand, deleted) in [(1u8, true), (0x81, true), (0, false), (0x80, false)] {
        let mut flags = Default::default();
        super::props::rpr(&[0x00, 0x08, operand], &[], &mut flags);
        assert_eq!(flags.deleted, deleted, "{operand:#x}");
    }
}

#[test]
fn copts80_no_leading_becomes_the_no_leading_compat_setting() {
    let settings = |copts80: u32| {
        let mut b = fixture();
        let (w0, t0) = (1536, 5632);
        put32(&mut b, w0 + 154 + 31 * 8, 1024);
        put32(&mut b, w0 + 158 + 31 * 8, 544);
        put32(&mut b, t0 + 1024 + 84, copts80);
        part(&convert(&b).unwrap().docx, "word/settings.xml")
    };
    // fNoLeading is bit 19 of Copts80.
    assert!(settings(0x8_0000).contains("<w:noLeading/>"));
    assert!(!settings(0x4_0000).contains("<w:noLeading/>"));
}

#[test]
fn shape_and_toc_field_results_are_found_by_instruction_name() {
    use super::body::field_results;
    let text: Vec<u16> = "\u{13} shape \\* MERGEFORMAT \u{14}\u{8}\u{15} \u{13}SHAPEX\u{14}x\u{15}\u{13}TOC \\o\u{14}t\u{15}"
        .encode_utf16()
        .collect();
    // (separator, end) of the SHAPE field only: case-insensitive, whole first word.
    assert_eq!(field_results(&text, "SHAPE"), [(23, 25)]);
    assert_eq!(field_results(&text, "TOC"), [(44, 46)]);
}

#[test]
fn a_table_framed_at_a_page_or_text_position_floats_there() {
    use super::body::float_framed_table;
    let table = |frame: &str| {
        let p = format!(r#"<w:p><w:pPr>{frame}</w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#);
        format!(
            r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tr><w:tc>{p}</w:tc><w:tc>{p}</w:tc></w:tr></w:tbl>"#
        )
    };
    let page = r#"<w:framePr w:wrap="around" w:hAnchor="margin" w:vAnchor="page" w:xAlign="center" w:y="1621" w:hSpace="180"/>"#;
    let out = float_framed_table(table(page));
    assert!(!out.contains("framePr"), "{out}");
    assert!(out.contains(r#"<w:tblPr><w:tblpPr w:leftFromText="180" w:rightFromText="180" w:vertAnchor="page" w:horzAnchor="margin" w:tblpXSpec="center" w:tblpY="1621"/>"#), "{out}");
    // A text anchor floats with the text; a margin anchor stays on the paragraphs, as
    // does a frame that only some of the table's paragraphs carry.
    let text = page.replace(r#"w:vAnchor="page""#, r#"w:vAnchor="text""#);
    let out = float_framed_table(table(&text));
    assert!(
        out.contains(r#"w:vertAnchor="text""#) && !out.contains("framePr"),
        "{out}"
    );
    let margin = page.replace(r#"w:vAnchor="page""#, r#"w:vAnchor="margin""#);
    assert_eq!(float_framed_table(table(&margin)), table(&margin));
    let mixed = table(page).replacen(page, "", 1);
    assert_eq!(float_framed_table(mixed.clone()), mixed);
}

#[test]
fn list_start_overrides_outside_the_doc_range_are_dropped() {
    let lvl = |flags: u32, start: i32| crate::office::doc::lists::LevelOverride {
        level: 0,
        raw_flags: flags,
        raw_start_at: start,
        start_override: None,
        format_override: None,
    };
    let start = super::body::exported_start;
    assert_eq!(start(&lvl(0x10, 0x7fff)), Some(0x7fff));
    assert_eq!(start(&lvl(0x30, 3)), Some(3));
    assert_eq!(start(&lvl(0x10, 134_954_608)), None);
    assert_eq!(start(&lvl(0x10, -1)), None);
    assert_eq!(start(&lvl(0x20, 3)), None);
}

#[test]
fn tight_and_through_shapes_keep_their_wrap() {
    let wrap = super::body::wrap_element;
    // A form box stored as "through" (5): Word keeps text off a narrow gap beside it in
    // compatibility mode 11, which square wrap would fill.
    assert!(wrap(5).starts_with("<wp:wrapThrough") && wrap(5).contains("<wp:wrapPolygon"));
    assert!(wrap(4).starts_with("<wp:wrapTight"));
    assert_eq!(wrap(1), "<wp:wrapTopAndBottom/>");
    assert!(wrap(0).starts_with("<wp:wrapSquare") && wrap(2).starts_with("<wp:wrapSquare"));
}

#[test]
fn a_custom_marked_footnote_keeps_its_reference() {
    // One footnote referenced at CP 0 whose mark is the body's own first character
    // (FRD nAuto 0), its text the whole footnote story. PlcffndRef at 128 and
    // PlcffndTxt at 160 in the table stream.
    let mut b = fixture();
    let refs: Vec<u8> = [0u32, 5]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .chain([0, 0])
        .collect();
    let texts: Vec<u8> = [0u32, 5].iter().flat_map(|v| v.to_le_bytes()).collect();
    b[5632 + 128..5632 + 128 + refs.len()].copy_from_slice(&refs);
    b[5632 + 160..5632 + 160 + texts.len()].copy_from_slice(&texts);
    for (index, at, len) in [(2usize, 128u32, refs.len()), (3, 160, texts.len())] {
        put32(&mut b, 1536 + 154 + index * 8, at);
        put32(&mut b, 1536 + 158 + index * 8, len as u32);
    }
    let document = part(&convert(&b).unwrap().docx, "word/document.xml");
    assert!(
        document.contains(r#"<w:footnoteReference w:customMarkFollows="1" w:id="2"/>"#),
        "{document}"
    );
}

#[test]
fn line_numbering_keeps_its_restart_mode() {
    let numbered = |lnc: u8| {
        // sprmSNLnnMod: every line, then sprmSLnc.
        super::props::section(&[0x15, 0x50, 1, 0, 0x13, 0x30, lnc]).xml()
    };
    assert!(numbered(2).contains("w:restart=\"continuous\""));
    assert!(numbered(1).contains("w:restart=\"newSection\""));
    assert!(numbered(0).contains("w:restart=\"newPage\""));
}

#[test]
fn copts60_balance_bit_clear_balances_single_and_double_byte_widths() {
    let settings = |copts60: u16| {
        let mut b = fixture();
        let (w0, t0) = (1536, 5632);
        // DOP (pair 31) at table offset 1024; Copts60 sits at DOP offset 8.
        put32(&mut b, w0 + 154 + 31 * 8, 1024);
        put32(&mut b, w0 + 158 + 31 * 8, 544);
        put16(&mut b, t0 + 1024 + 8, copts60);
        part(&convert(&b).unwrap().docx, "word/settings.xml")
    };
    assert!(settings(0).contains("<w:balanceSingleByteDoubleByteWidth/>"));
    // fDntBlnSbDbWid (bit 15) turns it off.
    assert!(!settings(0x8000).contains("balanceSingleByteDoubleByteWidth"));
}

#[test]
fn copts60_no_tab_for_indent_bit_drops_the_hanging_indent_tab_stop() {
    let settings = |copts60: u16| {
        let mut b = fixture();
        let (w0, t0) = (1536, 5632);
        put32(&mut b, w0 + 154 + 31 * 8, 1024);
        put32(&mut b, w0 + 158 + 31 * 8, 544);
        put16(&mut b, t0 + 1024 + 8, copts60);
        part(&convert(&b).unwrap().docx, "word/settings.xml")
    };
    // fNoTabForInd is bit 0 of Copts60.
    assert!(settings(0x0001).contains("<w:noTabHangInd/>"));
    assert!(!settings(0).contains("noTabHangInd"));
}

#[test]
fn a_missing_gb2312_font_draws_with_the_face_word_uses_for_doc_files() {
    // (name, stored alternate) per font index; runs use fonts 0, 1 and 2 as East Asian.
    let fonts = [
        ("楷体_GB2312", "楷体"),
        ("仿宋_GB2312", "宋体"),
        ("仿宋_GB2312", "仿宋"),
    ];
    let mut b = fixture();
    let (w0, t0) = (1536, 5632);
    let mut sttb = vec![fonts.len() as u8, 0, 0, 0];
    for (name, alt) in fonts {
        let mut ffn = vec![0u8; 39];
        ffn[1] = 144;
        ffn[2] = 1;
        ffn[3] = 134;
        ffn[4] = name.encode_utf16().count() as u8 + 1;
        ffn.extend(
            name.encode_utf16()
                .chain([0])
                .chain(alt.encode_utf16())
                .chain([0])
                .flat_map(u16::to_le_bytes),
        );
        sttb.push(ffn.len() as u8);
        sttb.extend(ffn);
    }
    b[t0 + 256..t0 + 256 + sttb.len()].copy_from_slice(&sttb);
    put32(&mut b, w0 + 154 + 15 * 8, 256);
    put32(&mut b, w0 + 158 + 15 * 8, sttb.len() as u32);
    let ea = |i: u8| [0x50, 0x4a, i, 0];
    story_with_runs(
        &mut b,
        b"abc\r",
        &[(1, &ea(0)), (2, &ea(1)), (3, &ea(2)), (4, &[])],
    );

    let document = part(&convert(&b).unwrap().docx, "word/document.xml");
    let east_asian = |text: &str| {
        let end = document.find(&format!(">{text}</w:t>")).unwrap();
        let run = &document[document[..end].rfind("<w:r>").unwrap()..end];
        let at = run.find("w:eastAsia=\"").unwrap() + 12;
        run[at..at + run[at..].find('"').unwrap()].to_string()
    };
    // Word for Mac renders of .doc files: 楷体_GB2312 draws with YaHei, a font stored
    // with 宋体 as its alternate with SimSun; other GB2312 fonts keep their name.
    assert_eq!(east_asian("a"), "Microsoft YaHei");
    assert_eq!(east_asian("b"), "宋体");
    assert_eq!(east_asian("c"), "仿宋_GB2312");
}

#[test]
fn a_missing_th_sarabun_it9_draws_with_word_s_thai_substitute() {
    // Word for Mac has no TH SarabunIT๙: a signed font-table entry draws with
    // Browallia New, an unsigned one with Cordia New.
    let ascii = |signed: bool| {
        let mut b = fixture();
        let (w0, t0) = (1536, 5632);
        let mut ffn = vec![0u8; 39];
        ffn[1] = 144;
        ffn[2] = 1;
        if signed {
            ffn[15] = 0x6f;
        }
        ffn.extend(
            "TH SarabunIT๙"
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes),
        );
        let mut sttb = vec![1u8, 0, 0, 0, ffn.len() as u8];
        sttb.extend(ffn);
        b[t0 + 256..t0 + 256 + sttb.len()].copy_from_slice(&sttb);
        put32(&mut b, w0 + 154 + 15 * 8, 256);
        put32(&mut b, w0 + 158 + 15 * 8, sttb.len() as u32);
        story_with_runs(&mut b, b"a\r", &[(1, &[0x4f, 0x4a, 0, 0]), (2, &[])]);
        let document = part(&convert(&b).unwrap().docx, "word/document.xml");
        let at = document.find("w:ascii=\"").unwrap() + 9;
        document[at..at + document[at..].find('"').unwrap()].to_string()
    };
    assert_eq!(ascii(true), "Browallia New");
    assert_eq!(ascii(false), "Cordia New");
}

/// The fixture with a `\x05SummaryInformation` stream naming `app` (PIDSI_APPNAME)
/// and a default header story "x" closed by its guard paragraph mark.
fn fixture_with_app_and_header(app: &str) -> Vec<u8> {
    let mut b = fixture();
    b.resize(27 * 512, 0);
    for id in 18..26 {
        put32(
            &mut b,
            512 + id * 4,
            if id == 25 { 0xffff_fffe } else { id as u32 + 1 },
        );
    }
    put32(&mut b, 1024 + 2 * 128 + 72, 3);
    let d = &mut b[1024 + 3 * 128..1024 + 4 * 128];
    let name: Vec<u16> = "\u{5}SummaryInformation".encode_utf16().collect();
    for (j, u) in name.iter().enumerate() {
        put16(d, j * 2, *u);
    }
    put16(d, 64, (name.len() * 2 + 2) as u16);
    d[66] = 2;
    d[67] = 1;
    for off in [68, 72, 76] {
        put32(d, off, 0xffff_ffff);
    }
    put32(d, 116, 18);
    put32(d, 120, 4096);
    // Property set: one section holding PIDSI_APPNAME (0x12) as a VT_LPSTR.
    let mut value = 0x1eu32.to_le_bytes().to_vec();
    value.extend(((app.len() + 1) as u32).to_le_bytes());
    value.extend(app.as_bytes());
    value.push(0);
    while !value.len().is_multiple_of(4) {
        value.push(0);
    }
    let mut section = ((16 + value.len()) as u32).to_le_bytes().to_vec();
    section.extend(1u32.to_le_bytes());
    section.extend(0x12u32.to_le_bytes());
    section.extend(16u32.to_le_bytes());
    section.extend(value);
    let mut set = vec![0xfe, 0xff, 0, 0, 0, 0, 0, 0];
    set.extend([0; 16]);
    set.extend(1u32.to_le_bytes());
    set.extend([0; 16]);
    set.extend(48u32.to_le_bytes());
    set.extend(section);
    b[19 * 512..19 * 512 + set.len()].copy_from_slice(&set);
    // Main text "a", then the header story "x" with its guard mark.
    story_with_runs(&mut b, b"a\rx\r\r", &[(5, &[])]);
    let (w0, t0) = (1536, 5632);
    put32(&mut b, w0 + 76, 2);
    put32(&mut b, w0 + 84, 3);
    // PlcfHdd (pair 11) at table offset 512: six separator stories, then the
    // six section slots with only the default header (slot 1) non-empty.
    let cps = [0u32, 0, 0, 0, 0, 0, 0, 0, 3, 3, 3, 3, 3, 3];
    for (k, cp) in cps.iter().enumerate() {
        put32(&mut b, t0 + 512 + k * 4, *cp);
    }
    put32(&mut b, w0 + 154 + 11 * 8, 512);
    put32(&mut b, w0 + 158 + 11 * 8, (cps.len() * 4) as u32);
    b
}

#[test]
fn a_wps_header_keeps_its_closing_empty_paragraph() {
    // WPS Office writes a header's last empty paragraph as content and Word for Mac
    // shows it; Word's own files end the story with a guard mark it hides.
    let paragraphs = |app: &str| {
        let docx = convert(&fixture_with_app_and_header(app)).unwrap().docx;
        let header = part(&docx, "word/header1.xml");
        header.matches("<w:p>").count() + header.matches("<w:p/>").count()
    };
    assert_eq!(paragraphs("WPS Office_11.1.0"), 2);
    assert_eq!(paragraphs("Microsoft Office Word"), 1);
}
