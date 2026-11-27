//! Synthetic .ppt files: a minimal OLE compound container around hand-built
//! [MS-PPT] records, converted and checked in the output package.
use super::*;
use std::io::Read;

use crate::office::ppt::fixtures::*;

fn part(pptx: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(pptx)).unwrap();
    let mut out = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut out).unwrap();
    out
}

fn texts(xml: &str) -> Vec<String> {
    xml.split("<a:t>")
        .skip(1)
        .map(|t| t.split('<').next().unwrap().to_string())
        .collect()
}

#[test]
fn newest_edit_wins_over_older_persist_entries() {
    let mut s = Stream::default();
    let doc = s.push(document((5760, 4320), &[(2, vec![])]));
    let shape = |t: &[u8]| text_shape(client_anchor(0, 0, 100, 100), text(4, t, true));
    let old_slide = s.push(slide_container(&[shape(b"Old text")]));
    let d1 = s.directory(&[(1, doc), (2, old_slide)]);
    let e1 = s.user_edit(0, d1, 1);
    // A fast save appends the changed slide and a directory listing only that slide.
    let new_slide = s.push(slide_container(&[shape(b"New text")]));
    let d2 = s.directory(&[(2, new_slide)]);
    let e2 = s.user_edit(e1, d2, 1);
    let file = cfb(&[
        ("PowerPoint Document", s.0),
        ("Current User", current_user(e2, 0xE391_C05F)),
    ]);
    let out = convert(&file).unwrap();
    assert_eq!(
        texts(&part(&out.pptx, "ppt/slides/slide1.xml")),
        ["New text"]
    );
}

#[test]
fn slides_follow_the_slide_list_not_persist_ids() {
    let mut s = Stream::default();
    let shape = |t: &[u8]| text_shape(client_anchor(0, 0, 100, 100), text(4, t, true));
    let doc = s.push(document((5760, 4320), &[(7, vec![]), (3, vec![])]));
    let a = s.push(slide_container(&[shape(b"persist three")]));
    let b = s.push(slide_container(&[shape(b"persist seven")]));
    let d = s.directory(&[(1, doc), (3, a), (7, b)]);
    let e = s.user_edit(0, d, 1);
    let file = cfb(&[
        ("PowerPoint Document", s.0),
        ("Current User", current_user(e, 0xE391_C05F)),
    ]);
    let out = convert(&file).unwrap();
    assert_eq!(
        texts(&part(&out.pptx, "ppt/slides/slide1.xml")),
        ["persist seven"]
    );
    assert_eq!(
        texts(&part(&out.pptx, "ppt/slides/slide2.xml")),
        ["persist three"]
    );
}

#[test]
fn text_bytes_are_latin1_and_text_chars_are_utf16() {
    let shapes = vec![
        text_shape(
            client_anchor(0, 0, 100, 100),
            text(4, b"caf\xe9 na\xefve", true),
        ),
        text_shape(
            client_anchor(100, 0, 100, 200),
            text(4, &utf16("日本語 €"), false),
        ),
    ];
    let out = convert(&simple_ppt((5760, 4320), &[(vec![], shapes)])).unwrap();
    assert_eq!(
        texts(&part(&out.pptx, "ppt/slides/slide1.xml")),
        ["café naïve", "日本語 €"]
    );
}

#[test]
fn slide_size_converts_master_units_to_emu() {
    // 7680 x 4320 master units at 576 per inch is 13.333 x 7.5 in (16:9).
    let out = convert(&simple_ppt((7680, 4320), &[(vec![], vec![])])).unwrap();
    assert!(
        part(&out.pptx, "ppt/presentation.xml")
            .contains("<p:sldSz cx=\"12192000\" cy=\"6858000\"/>")
    );
}

#[test]
fn outline_text_ref_places_slide_list_text_at_the_shape_anchor() {
    // The anchor is top, left, right, bottom: one inch down, half an inch in.
    let shape = text_shape(
        client_anchor(576, 288, 1440, 1152),
        vec![rec(0x0F9E, 0, &u32s(&[1]))],
    );
    let mut list_texts = text(0, b"Title text", true);
    list_texts.extend(text(1, b"First\rSecond\x0bline", true));
    let out = convert(&simple_ppt((5760, 4320), &[(list_texts, vec![shape])])).unwrap();
    let xml = part(&out.pptx, "ppt/slides/slide1.xml");
    let first = xml.find("<a:t>First").expect("body text");
    let start = xml[..first].rfind("<p:sp>").unwrap();
    let body = &xml[start..first + xml[first..].find("</p:sp>").unwrap()];
    assert!(
        body.contains("<a:off x=\"457200\" y=\"914400\"/><a:ext cx=\"1828800\" cy=\"914400\"/>")
    );
    assert_eq!(texts(body), ["First", "Second", "line"]);
    assert_eq!(body.matches("<a:p>").count(), 2);
    // The title is referenced by no shape and still reaches the slide.
    assert!(texts(&xml).contains(&"Title text".to_string()));
}

#[test]
fn outline_text_numbering_comes_from_the_document_extension() {
    // Slide-list body text with one bulleted paragraph run; its numbering is stored at
    // document level, keyed by the slide id (256) and the text type (1, body).
    let shape = text_shape(
        client_anchor(576, 288, 1440, 1152),
        vec![rec(0x0F9E, 0, &u32s(&[0]))],
    );
    let mut style = u32s(&[5]);
    style.extend(0u16.to_le_bytes());
    style.extend(u32s(&[0x1]));
    style.extend(1u16.to_le_bytes());
    style.extend(u32s(&[5, 0]));
    let mut list_texts = text(1, b"Item", true);
    list_texts.push(rec(0x0FA1, 0, &style));
    // Scheme 6 (roman lower case, period) from 2.
    let mut pf9 = u32s(&[0x0300_0000]);
    pf9.extend(1u16.to_le_bytes());
    pf9.extend(6u16.to_le_bytes());
    pf9.extend(2i16.to_le_bytes());
    pf9.extend(u32s(&[0, 0]));
    let outline = container(
        0x0FAE,
        0,
        &[rec(0x0FAF, 1, &u32s(&[256, 1])), rec(0x0FAC, 0, &pf9)],
    );
    let tag = container(
        0x138A,
        0,
        &[rec(0x0FBA, 0, &utf16("___PPT9")), rec(0x138B, 0, &outline)],
    );
    let info = container(0x07D0, 0, &[container(0x1388, 0, &[tag])]);
    let mut s = Stream::default();
    let plain = document((5760, 4320), &[(2, list_texts)]);
    let doc = s.push(container(0x03E8, 0, &[plain[8..].to_vec(), info]));
    let slide = s.push(slide_container(&[shape]));
    let d = s.directory(&[(1, doc), (2, slide)]);
    let edit = s.user_edit(0, d, 1);
    let file = cfb(&[
        ("PowerPoint Document", s.0),
        ("Current User", current_user(edit, 0xE391_C05F)),
    ]);
    let out = convert(&file).unwrap();
    let xml = part(&out.pptx, "ppt/slides/slide1.xml");
    assert!(
        xml.contains(r#"<a:buAutoNum type="romanLcPeriod" startAt="2"/>"#),
        "{xml}"
    );
}

#[test]
fn group_members_map_through_the_group_frame() {
    // The group covers 576..1152 x 0..576 master units and gives members a 0..1000
    // space; a member at 500..1000 lands in the frame's lower-right quarter.
    let group = container(
        0xF003,
        0,
        &[
            container(
                0xF004,
                0,
                &[
                    rec(0xF009, 0, &u32s(&[0, 0, 1000, 1000])),
                    client_anchor(0, 576, 1152, 576),
                ],
            ),
            container(
                0xF004,
                0,
                &[
                    rec(0xF00F, 0, &u32s(&[500, 500, 1000, 1000])),
                    container(0xF00D, 0, &text(4, b"cell", true)),
                ],
            ),
        ],
    );
    let out = convert(&simple_ppt((5760, 4320), &[(vec![], vec![group])])).unwrap();
    let xml = part(&out.pptx, "ppt/slides/slide1.xml");
    assert!(
        xml.contains("<a:off x=\"1371600\" y=\"457200\"/><a:ext cx=\"457200\" cy=\"457200\"/>"),
        "{xml}"
    );
}

#[test]
fn encrypted_documents_are_rejected() {
    let mut s = Stream::default();
    let doc = s.push(document((5760, 4320), &[]));
    let d = s.directory(&[(1, doc)]);
    let e = s.user_edit(0, d, 1);
    let file = cfb(&[
        ("PowerPoint Document", s.0),
        ("Current User", current_user(e, 0xF3D1_C4DF)),
    ]);
    assert!(matches!(convert(&file), Err(Error::Encrypted)));
}

#[test]
fn converted_package_opens_in_pptx_read_with_a_full_chain() {
    let shapes = vec![text_shape(
        client_anchor(0, 0, 100, 100),
        text(4, b"hello", true),
    )];
    let out = convert(&simple_ppt(
        (5760, 4320),
        &[(vec![], shapes.clone()), (vec![], shapes)],
    ))
    .unwrap();
    let limits = crate::office::pptx::config::Limits::default();
    let doc = crate::office::pptx::document::Document::open(out.pptx, &limits).unwrap();
    assert_eq!(doc.slides.len(), 2);
    assert_eq!((doc.slide_w, doc.slide_h), (9_144_000, 6_858_000));
    let chain = doc.chain(&doc.slides[1].part).unwrap();
    assert!(chain.layout.is_some() && chain.master.is_some());
    assert_eq!(chain.theme_part, "ppt/theme/theme1.xml");
}

#[test]
fn masked_metro_text_takes_the_binary_characters() {
    let xml = "<p:sp xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" \
xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><p:txBody>\
<a:p><a:r><a:t>___ </a:t></a:r><a:r><a:t>__</a:t></a:r><a:br/><a:r><a:t>_</a:t></a:r></a:p>\
<a:p><a:r><a:t>____</a:t></a:r></a:p></p:txBody></p:sp>";
    let filled = metro::fill_text(xml, "One &x\u{b}y\rlast\r").unwrap();
    assert_eq!(texts(&filled), ["One ", "&amp;x", "y", "last"]);
    // One character short: the masked runs no longer line up with the binary text.
    assert_eq!(metro::fill_text(xml, "One &x\u{b}y\rlas"), None);
}

/// An OPT record with simple (id, value) properties.
fn opt(props: &[(u16, u32)]) -> Vec<u8> {
    let body: Vec<u8> = props
        .iter()
        .flat_map(|&(id, v)| {
            let mut e = id.to_le_bytes().to_vec();
            e.extend(v.to_le_bytes());
            e
        })
        .collect();
    rec(0xF00B, props.len() as u16, &body)
}

/// A shape of MSOSPT type `kind` with an anchor, properties and optional text records.
fn shape(kind: u16, anchor: Vec<u8>, props: &[(u16, u32)], textbox: Vec<Vec<u8>>) -> Vec<u8> {
    let mut kids = vec![
        rec(0xF00A, kind, &u32s(&[0x401, 0xA00])),
        opt(props),
        anchor,
    ];
    if !textbox.is_empty() {
        kids.push(container(0xF00D, 0, &textbox));
    }
    container(0xF004, 0, &kids)
}

/// ColorSchemeAtom (instance 1, the sheet's own scheme) from 0xRRGGBB colours.
fn scheme(colors: [u32; 8]) -> Vec<u8> {
    let bgr = colors.map(|c| ((c & 0xFF) << 16) | (c & 0xFF00) | (c >> 16));
    rec(0x07F0, 1, &u32s(&bgr))
}

/// A SlideContainer with extra atoms before its drawing. `flags` are the SlideAtom's
/// follow-master bits and `master` its masterIdRef.
fn sheet(kind: u16, master: u32, flags: u16, atoms: &[Vec<u8>], shapes: &[Vec<u8>]) -> Vec<u8> {
    let mut atom = vec![0u8; 24];
    atom[12..16].copy_from_slice(&master.to_le_bytes());
    atom[20..22].copy_from_slice(&flags.to_le_bytes());
    let mut tree = vec![patriarch()];
    tree.extend_from_slice(shapes);
    let drawing = container(
        0x040C,
        0,
        &[container(0xF002, 0, &[container(0xF003, 0, &tree)])],
    );
    let mut kids = vec![rec(0x03EF, 0, &atom)];
    kids.extend_from_slice(atoms);
    kids.push(drawing);
    container(kind, 0, &kids)
}

/// A file with one master (persist 2, slide id 0x80000001) and the given slides
/// (persist 3.., each a full SlideContainer).
fn deck_with_master(master: Vec<u8>, slides: &[Vec<u8>]) -> Vec<u8> {
    deck_with(&[], master, slides)
}

/// [`deck_with_master`] with extra DocumentContainer records (after the DocumentAtom).
fn deck_with(extras: &[Vec<u8>], master: Vec<u8>, slides: &[Vec<u8>]) -> Vec<u8> {
    let mut s = Stream::default();
    let mut atom = Vec::new();
    atom.extend(5760i32.to_le_bytes());
    atom.extend(4320i32.to_le_bytes());
    atom.resize(40, 0);
    atom[32..34].copy_from_slice(&1u16.to_le_bytes());
    let masters = container(
        0x0FF0,
        1,
        &[rec(0x03F3, 0, &u32s(&[2, 0, 0, 0x8000_0001, 0]))],
    );
    let list: Vec<Vec<u8>> = (0..slides.len())
        .map(|i| rec(0x03F3, 0, &u32s(&[i as u32 + 3, 0, 0, 256 + i as u32, 0])))
        .collect();
    let doc = s.push(container(
        0x03E8,
        0,
        &[
            vec![rec(0x03E9, 0, &atom)],
            extras.to_vec(),
            vec![masters, container(0x0FF0, 0, &list)],
        ]
        .concat(),
    ));
    let m = s.push(master);
    let mut dir = vec![(1, doc), (2, m)];
    for (i, sl) in slides.iter().enumerate() {
        dir.push((i as u32 + 3, s.push(sl.clone())));
    }
    let d = s.directory(&dir);
    let e = s.user_edit(0, d, 1);
    cfb(&[
        ("PowerPoint Document", s.0),
        ("Current User", current_user(e, 0xE391_C05F)),
    ])
}

const FILLS: [u32; 8] = [
    0xFFFFFF, 0x000000, 0x808080, 0x000000, 0x112233, 0x445566, 0x778899, 0xAABBCC,
];

#[test]
fn shape_fill_resolves_a_scheme_index_against_the_slide_scheme() {
    // fillColor 0x08000004: scheme entry 4; lineColor 0x08000005: entry 5.
    let rect = shape(
        1,
        client_anchor(0, 0, 576, 576),
        &[(0x181, 0x0800_0004), (0x1C0, 0x0800_0005), (0x1CB, 12700)],
        vec![],
    );
    let slide = sheet(0x03EE, 0x8000_0001, 0, &[scheme(FILLS)], &[rect]);
    let master = sheet(0x03F8, 0, 0, &[scheme([0; 8])], &[]);
    let out = convert(&deck_with_master(master, &[slide])).unwrap();
    let xml = part(&out.pptx, "ppt/slides/slide1.xml");
    assert!(xml.contains("<a:prstGeom prst=\"rect\">"), "{xml}");
    assert!(xml.contains("<a:solidFill><a:srgbClr val=\"112233\"/></a:solidFill>"));
    assert!(xml.contains("<a:ln w=\"12700\"><a:solidFill><a:srgbClr val=\"445566\"/>"));
}

#[test]
fn a_slide_following_the_master_scheme_uses_the_master_colours() {
    let rect = shape(
        1,
        client_anchor(0, 0, 576, 576),
        &[(0x181, 0x0800_0004)],
        vec![],
    );
    // Flag 2 (fMasterScheme): the slide's own scheme is ignored.
    let slide = sheet(0x03EE, 0x8000_0001, 2, &[scheme([0; 8])], &[rect]);
    let master = sheet(0x03F8, 0, 0, &[scheme(FILLS)], &[]);
    let out = convert(&deck_with_master(master, &[slide])).unwrap();
    assert!(part(&out.pptx, "ppt/slides/slide1.xml").contains("<a:srgbClr val=\"112233\"/>"));
}

#[test]
fn style_text_prop_runs_set_bold_size_and_colour() {
    // One paragraph run over the 9 characters plus the final mark, then two character
    // runs: "Bold " bold 20pt RGB 0xC00000, and "rest" with no overrides.
    let mut style = u32s(&[10]);
    style.extend(0u16.to_le_bytes());
    style.extend(u32s(&[0]));
    style.extend(u32s(&[5, 0x0006_0001]));
    style.extend(1u16.to_le_bytes());
    style.extend(20u16.to_le_bytes());
    style.extend(u32s(&[0xFE00_00C0, 5, 0]));
    let mut tb = text(4, b"Bold rest", true);
    tb.push(rec(0x0FA1, 0, &style));
    let s = shape(
        202,
        client_anchor(0, 0, 2000, 500),
        &[(0x1BF, 0x10_0000)],
        tb,
    );
    let out = convert(&simple_ppt((5760, 4320), &[(vec![], vec![s])])).unwrap();
    let xml = part(&out.pptx, "ppt/slides/slide1.xml");
    let bold = xml.find("<a:t>Bold </a:t>").unwrap();
    let rpr = &xml[xml[..bold].rfind("<a:rPr").unwrap()..bold];
    assert!(rpr.contains("sz=\"2000\" b=\"1\""), "{rpr}");
    assert!(rpr.contains("<a:srgbClr val=\"C00000\"/>"));
    let rest = xml.find("<a:t>rest</a:t>").unwrap();
    assert!(xml[xml[..rest].rfind("<a:rPr").unwrap()..rest].contains("b=\"0\""));
    // fFilled was turned off, so the text box has no fill.
    assert!(xml.contains("<a:noFill/>"));
}

#[test]
fn powerpoint_2000_auto_numbering_becomes_bu_auto_num() {
    // One bulleted paragraph run over "Item" and its mark (PFMasks hasBullet, flags on).
    let mut style = u32s(&[5]);
    style.extend(0u16.to_le_bytes());
    style.extend(u32s(&[0x1]));
    style.extend(1u16.to_le_bytes());
    style.extend(u32s(&[5, 0]));
    let mut tb = text(4, b"Item", true);
    tb.push(rec(0x0FA1, 0, &style));
    // StyleTextProp9: TextPFException9 with fBulletHasAutoNumber and scheme 3
    // (arabic, period) from 4, then empty TextCFException9 and TextSIException.
    let mut pf9 = u32s(&[0x0300_0000]);
    pf9.extend(1u16.to_le_bytes());
    pf9.extend(3u16.to_le_bytes());
    pf9.extend(4i16.to_le_bytes());
    pf9.extend(u32s(&[0, 0]));
    let tag = container(
        0x138A,
        0,
        &[
            rec(0x0FBA, 0, &utf16("___PPT9")),
            rec(0x138B, 0, &rec(0x0FAC, 0, &pf9)),
        ],
    );
    let client_data = container(0xF011, 0, &[container(0x1388, 0, &[tag])]);
    let s = container(
        0xF004,
        0,
        &[
            rec(0xF00A, 0, &u32s(&[0x401, 0xA00])),
            client_anchor(0, 0, 2000, 500),
            client_data,
            container(0xF00D, 0, &tb),
        ],
    );
    let out = convert(&simple_ppt((5760, 4320), &[(vec![], vec![s])])).unwrap();
    let xml = part(&out.pptx, "ppt/slides/slide1.xml");
    assert!(
        xml.contains(r#"<a:buAutoNum type="arabicPeriod" startAt="4"/>"#),
        "{xml}"
    );
}

#[test]
fn master_shapes_and_background_go_to_the_master_unless_the_slide_hides_them() {
    let bar = shape(
        1,
        client_anchor(0, 0, 5760, 300),
        &[(0x181, 0x0800_0004)],
        vec![],
    );
    let mut bg = container(
        0xF004,
        0,
        &[
            rec(0xF00A, 1, &u32s(&[0x400, 0xC00])),
            opt(&[(0x181, 0x0800_0005)]),
        ],
    );
    let master = {
        let mut m = sheet(0x03F8, 0, 0, &[scheme(FILLS)], &[bar]);
        // The background shape follows the shape tree inside the DgContainer.
        let dg = m.windows(2).rposition(|w| w == [0x02, 0xF0]).unwrap() - 2;
        let len = u32::from_le_bytes(m[dg + 4..dg + 8].try_into().unwrap()) as usize;
        m.splice(dg + 8 + len..dg + 8 + len, bg.drain(..));
        fix_lengths(&mut m);
        m
    };
    let shown = sheet(0x03EE, 0x8000_0001, 7, &[], &[]);
    let hidden = sheet(0x03EE, 0x8000_0001, 6, &[], &[]);
    let out = convert(&deck_with_master(master, &[shown, hidden])).unwrap();
    let m = part(&out.pptx, "ppt/slideMasters/slideMaster1.xml");
    assert!(
        m.contains("<p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"445566\"/>"),
        "{m}"
    );
    assert!(m.contains("<a:srgbClr val=\"112233\"/>"));
    assert!(!part(&out.pptx, "ppt/slides/slide1.xml").contains("showMasterSp"));
    assert!(part(&out.pptx, "ppt/slides/slide2.xml").contains("showMasterSp=\"0\""));
}

/// Recomputes the lengths of a SlideContainer > PPDrawing > DgContainer chain after
/// bytes were inserted at the end of the DgContainer.
fn fix_lengths(m: &mut [u8]) {
    let total = m.len();
    m[4..8].copy_from_slice(&((total - 8) as u32).to_le_bytes());
    let mut off = 8;
    while off + 8 <= total {
        let kind = u16::from_le_bytes([m[off + 2], m[off + 3]]);
        let len = u32::from_le_bytes(m[off + 4..off + 8].try_into().unwrap()) as usize;
        if kind == 0x040C || kind == 0xF002 {
            m[off + 4..off + 8].copy_from_slice(&((total - off - 8) as u32).to_le_bytes());
            off += 8;
        } else {
            off += 8 + len;
        }
    }
}

#[test]
fn slide_number_fields_show_the_slide_number() {
    let mut tb = text(4, b"Page *", true);
    tb.push(rec(0x0FD8, 0, &u32s(&[5])));
    let s = shape(202, client_anchor(0, 0, 2000, 500), &[], tb);
    let slides = [
        sheet(0x03EE, 0x8000_0001, 7, &[], &[]),
        sheet(0x03EE, 0x8000_0001, 7, &[], &[s]),
    ];
    let out = convert(&deck_with_master(sheet(0x03F8, 0, 0, &[], &[]), &slides)).unwrap();
    assert_eq!(texts(&part(&out.pptx, "ppt/slides/slide2.xml")), ["Page 2"]);
}

/// ClientData holding an OEPlaceholderAtom of placeholder type `t`.
fn placeholder(t: u8) -> Vec<u8> {
    let mut body = u32s(&[0]);
    body.extend([t, 0, 0, 0]);
    container(0xF011, 0, &[rec(0x0BC3, 0, &body)])
}

#[test]
fn slide_placeholders_take_unset_paint_from_the_master_placeholder() {
    // The master body placeholder turns fill and line off (bits 4 and 3 with their use
    // flags); the slide's body placeholder sets neither, so it draws no box either.
    let mut master_ph = shape(
        202,
        client_anchor(0, 0, 576, 576),
        &[(0x1BF, 0x10_0000), (0x1FF, 0x8_0000)],
        vec![],
    );
    let mut slide_ph = shape(
        202,
        client_anchor(0, 0, 576, 576),
        &[],
        text(1, b"Body", true),
    );
    for (sp, t) in [(&mut master_ph, 2u8), (&mut slide_ph, 14u8)] {
        let extra = placeholder(t);
        let len = u32::from_le_bytes(sp[4..8].try_into().unwrap()) as usize + extra.len();
        sp[4..8].copy_from_slice(&(len as u32).to_le_bytes());
        sp.extend(extra);
    }
    let master = sheet(0x03F8, 0, 0, &[scheme(FILLS)], &[master_ph]);
    let slide = sheet(0x03EE, 0x8000_0001, 7, &[], &[slide_ph]);
    let out = convert(&deck_with_master(master, &[slide])).unwrap();
    let xml = part(&out.pptx, "ppt/slides/slide1.xml");
    assert!(xml.contains("<a:noFill/><a:ln><a:noFill/></a:ln>"), "{xml}");
}

#[test]
fn background_fills_default_to_white_and_take_each_slides_scheme() {
    // PowerPoint renders of 1997-2003 decks: an unset background colour is white; a
    // scheme-indexed one resolves with the scheme of each slide that has its own.
    let master_with_bg = |fill: &[(u16, u32)]| {
        let mut props = vec![(0x1BF, 0x10_0010)];
        props.extend_from_slice(fill);
        let mut bg = container(
            0xF004,
            0,
            &[rec(0xF00A, 1, &u32s(&[0x400, 0xC00])), opt(&props)],
        );
        let mut m = sheet(
            0x03F8,
            0,
            0,
            &[scheme([0xEEDDCC, 0, 0, 0, 0x112233, 0, 0, 0])],
            &[],
        );
        let dg = m.windows(2).rposition(|w| w == [0x02, 0xF0]).unwrap() - 2;
        let len = u32::from_le_bytes(m[dg + 4..dg + 8].try_into().unwrap()) as usize;
        m.splice(dg + 8 + len..dg + 8 + len, bg.drain(..));
        fix_lengths(&mut m);
        m
    };
    let follows = sheet(0x03EE, 0x8000_0001, 7, &[], &[]);
    // Flags 5: follows the master's objects and background, keeps its own scheme.
    let own_scheme = sheet(
        0x03EE,
        0x8000_0001,
        5,
        &[scheme([0x445566, 0, 0, 0, 0, 0, 0, 0])],
        &[],
    );
    let out = convert(&deck_with_master(
        master_with_bg(&[]),
        std::slice::from_ref(&follows),
    ))
    .unwrap();
    let master = part(&out.pptx, "ppt/slideMasters/slideMaster1.xml");
    assert!(
        master.contains("<p:bgPr><a:solidFill><a:srgbClr val=\"FFFFFF\"/>"),
        "{master}"
    );
    let deck = deck_with_master(
        master_with_bg(&[(0x181, 0x0800_0000)]),
        &[follows, own_scheme],
    );
    let out = convert(&deck).unwrap();
    let master = part(&out.pptx, "ppt/slideMasters/slideMaster1.xml");
    assert!(master.contains("<a:srgbClr val=\"EEDDCC\"/>"), "{master}");
    assert!(!part(&out.pptx, "ppt/slides/slide1.xml").contains("<p:bg>"));
    let second = part(&out.pptx, "ppt/slides/slide2.xml");
    assert!(
        second.contains("<p:bgPr><a:solidFill><a:srgbClr val=\"445566\"/>"),
        "{second}"
    );
}

#[test]
fn pictures_have_no_outline_unless_they_set_one() {
    // The drawing group defaults to a line; a picture frame (type 75) ignores it.
    let pic = shape(75, client_anchor(0, 0, 576, 576), &[(0x104, 1)], vec![]);
    let framed = shape(
        75,
        client_anchor(0, 0, 576, 576),
        &[(0x104, 1), (0x1FF, 0x8_0008)],
        vec![],
    );
    let slide = sheet(0x03EE, 0x8000_0001, 7, &[], &[pic, framed]);
    let out = convert(&deck_with_master(sheet(0x03F8, 0, 0, &[], &[]), &[slide])).unwrap();
    let xml = part(&out.pptx, "ppt/slides/slide1.xml");
    let pics: Vec<&str> = xml.split("<p:pic>").skip(1).collect();
    assert_eq!(pics.len(), 2);
    assert!(pics[0].contains("<a:ln><a:noFill/></a:ln>"), "{xml}");
    assert!(pics[1].contains("<a:ln w=\"9525\"><a:solidFill>"));
}

#[test]
fn an_embedded_equation_becomes_an_ole_frame_with_its_prog_id() {
    // An OLE shape refers (ExObjRefAtom) to ExObjList entry 7, whose progId is
    // "Equation.3"; a picture with no object stays a plain picture.
    let utf16 = |t: &str| {
        t.encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<u8>>()
    };
    let ole_list = container(
        0x0409,
        0,
        &[container(
            0x0FCC,
            0,
            &[
                rec(0x0FC3, 0, &u32s(&[0, 0, 7, 0, 0, 0])),
                rec(0x0FBA, 2, &utf16("Equation.3")),
            ],
        )],
    );
    let equation = container(
        0xF004,
        0,
        &[
            rec(0xF00A, 75, &u32s(&[0x401, 0xA00])),
            opt(&[(0x104, 1)]),
            client_anchor(0, 0, 576, 576),
            container(0xF011, 0, &[rec(0x0BC1, 0, &u32s(&[7]))]),
        ],
    );
    let plain = shape(75, client_anchor(0, 0, 576, 576), &[(0x104, 1)], vec![]);
    let slide = sheet(0x03EE, 0x8000_0001, 7, &[], &[equation, plain]);
    let out = convert(&deck_with(
        &[ole_list],
        sheet(0x03F8, 0, 0, &[], &[]),
        &[slide],
    ))
    .unwrap();
    let xml = part(&out.pptx, "ppt/slides/slide1.xml");
    assert_eq!(
        xml.matches(r#"<p:oleObj progId="Equation.3">"#).count(),
        1,
        "{xml}"
    );
    assert_eq!(xml.matches("<p:pic>").count(), 2, "{xml}");
}

#[test]
fn an_exact_copy_of_a_master_shape_is_written_once() {
    let boxed = || {
        shape(
            202,
            client_anchor(0, 0, 2000, 500),
            &[],
            text(4, b"Copyright", true),
        )
    };
    let other = shape(
        202,
        client_anchor(0, 600, 2000, 1100),
        &[],
        text(4, b"Copyright", true),
    );
    let master = sheet(0x03F8, 0, 0, &[], &[boxed(), boxed(), other]);
    let slides = [sheet(0x03EE, 0x8000_0001, 7, &[], &[])];
    let out = convert(&deck_with_master(master, &slides)).unwrap();
    let m = part(&out.pptx, "ppt/slideMasters/slideMaster1.xml");
    // The second identical box is dropped; the one placed elsewhere stays.
    assert_eq!(m.matches("Copyright").count(), 2, "{m}");
}

#[test]
fn a_masters_round_trip_theme_and_colour_map_reach_its_slide_master() {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file(
        "theme/theme/theme1.xml",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    std::io::Write::write_all(&mut zip, b"<a:theme name=\"Kept\"/>").unwrap();
    let theme = zip.finish().unwrap().into_inner();
    let map = b"<a:clrMap xmlns:a=\"x\" bg1=\"dk1\" tx1=\"lt1\"/>";
    let master = sheet(
        0x03F8,
        0,
        0,
        &[rec(0x040E, 0, &theme), rec(0x040F, 0, map)],
        &[],
    );
    let out = convert(&deck_with_master(
        master,
        &[sheet(0x03EE, 0x8000_0001, 7, &[], &[])],
    ))
    .unwrap();
    assert_eq!(
        part(&out.pptx, "ppt/theme/theme2.xml"),
        "<a:theme name=\"Kept\"/>"
    );
    assert!(
        part(&out.pptx, "ppt/slideMasters/_rels/slideMaster1.xml.rels")
            .contains("../theme/theme2.xml")
    );
    assert!(
        part(&out.pptx, "ppt/slideMasters/slideMaster1.xml")
            .contains("<p:clrMap bg1=\"dk1\" tx1=\"lt1\"/>")
    );
    assert!(part(&out.pptx, "[Content_Types].xml").contains("/ppt/theme/theme2.xml"));
}

#[test]
fn drawing_group_defaults_do_not_paint_existing_shapes() {
    // The DggContainer's OPT holds the fill for new shapes (scheme entry 4). A
    // rectangle that sets no fill colour still fills with OfficeArt's white.
    let dgg = container(
        0x040B,
        0,
        &[container(0xF000, 0, &[opt(&[(0x181, 0x0800_0004)])])],
    );
    let rect = shape(1, client_anchor(0, 0, 576, 576), &[], vec![]);
    let slide = sheet(0x03EE, 0x8000_0001, 7, &[], &[rect]);
    let master = sheet(0x03F8, 0, 0, &[scheme(FILLS)], &[]);
    let out = convert(&deck_with(&[dgg], master, &[slide])).unwrap();
    let xml = part(&out.pptx, "ppt/slides/slide1.xml");
    assert!(
        xml.contains("<a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill>"),
        "{xml}"
    );
}

#[test]
fn a_linear_gradient_runs_from_the_fill_colour_to_the_back_colour_along_powerpoints_curve() {
    // Fill type 7 (shade scale), fill 0x112233, back 0x445566, focus 100. Halfway
    // along, the colour is 0.5^1.87 = 0.27 of the way to the back colour.
    let g = shape(
        1,
        client_anchor(0, 0, 576, 576),
        &[
            (0x180, 7),
            (0x181, 0x0033_2211),
            (0x183, 0x0066_5544),
            (0x18C, 100),
        ],
        vec![],
    );
    let slide = sheet(0x03EE, 0x8000_0001, 7, &[], &[g]);
    let out = convert(&deck_with_master(sheet(0x03F8, 0, 0, &[], &[]), &[slide])).unwrap();
    let xml = part(&out.pptx, "ppt/slides/slide1.xml");
    for stop in [
        "<a:gs pos=\"0\"><a:srgbClr val=\"112233\"/></a:gs>",
        "<a:gs pos=\"50000\"><a:srgbClr val=\"1F3041\"/></a:gs>",
        "<a:gs pos=\"100000\"><a:srgbClr val=\"445566\"/></a:gs>",
    ] {
        assert!(xml.contains(stop), "{stop} in {xml}");
    }
}

#[test]
fn hyperlinked_text_is_underlined_in_the_scheme_hyperlink_colour() {
    let mut tb = text(4, b"see site now", true);
    tb.push(rec(0x0FDF, 0, &u32s(&[4, 8])));
    let s = shape(202, client_anchor(0, 0, 2000, 500), &[], tb);
    let slide = sheet(0x03EE, 0x8000_0001, 7, &[], &[s]);
    let master = sheet(0x03F8, 0, 0, &[scheme(FILLS)], &[]);
    let out = convert(&deck_with_master(master, &[slide])).unwrap();
    let xml = part(&out.pptx, "ppt/slides/slide1.xml");
    assert_eq!(texts(&xml), ["see ", "site", " now"]);
    let link = xml.find("<a:t>site</a:t>").unwrap();
    let rpr = &xml[xml[..link].rfind("<a:rPr").unwrap()..link];
    assert!(
        rpr.contains("u=\"sng\"") && rpr.contains("<a:srgbClr val=\"778899\"/>"),
        "{rpr}"
    );
}

#[test]
fn a_turned_group_turns_and_moves_its_members() {
    // A group over 0..1000 x 0..500 master units turned 180 degrees (OPT rotation in
    // 16.16 degrees): its left-half member ends up in the right half, upside down.
    let group = container(
        0xF003,
        0,
        &[
            container(
                0xF004,
                0,
                &[
                    rec(0xF009, 0, &u32s(&[0, 0, 1000, 500])),
                    opt(&[(0x004, 180 << 16)]),
                    client_anchor(0, 0, 1000, 500),
                ],
            ),
            container(
                0xF004,
                0,
                &[
                    rec(0xF00F, 0, &u32s(&[0, 0, 500, 500])),
                    container(0xF00D, 0, &text(4, b"half", true)),
                ],
            ),
        ],
    );
    let out = convert(&simple_ppt((5760, 4320), &[(vec![], vec![group])])).unwrap();
    let xml = part(&out.pptx, "ppt/slides/slide1.xml");
    assert!(
        xml.contains("<a:xfrm rot=\"10800000\"><a:off x=\"793750\" y=\"0\"/><a:ext cx=\"793750\" cy=\"793750\"/>"),
        "{xml}"
    );
}

/// A zip holding one part, as round-trip atoms store DrawingML.
fn one_part_zip(name: &str, body: &[u8]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file(name, zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut zip, body).unwrap();
    zip.finish().unwrap().into_inner()
}

#[test]
fn kerning_comes_from_the_masters_round_trip_text_styles() {
    // The body style kerns from 12pt; the other style (plain text boxes) does not.
    let styles = b"<p:txStyles xmlns:a=\"a\" xmlns:p=\"p\"><p:bodyStyle><a:lvl1pPr>\
<a:defRPr kern=\"1200\"/></a:lvl1pPr></p:bodyStyle><p:otherStyle><a:lvl1pPr><a:defRPr/>\
</a:lvl1pPr></p:otherStyle></p:txStyles>";
    let zip = one_part_zip("drs/slideMasters/slideMaster1.xml", styles);
    let master = sheet(0x03F8, 0, 0, &[rec(0x0423, 0, &zip)], &[]);
    let body = shape(
        202,
        client_anchor(0, 0, 2000, 500),
        &[],
        text(1, b"Body", true),
    );
    let other = shape(
        202,
        client_anchor(600, 0, 2000, 1100),
        &[],
        text(4, b"Other", true),
    );
    let slide = sheet(0x03EE, 0x8000_0001, 7, &[], &[body, other]);
    let xml = part(
        &convert(&deck_with_master(master, &[slide])).unwrap().pptx,
        "ppt/slides/slide1.xml",
    );
    let rpr = |t: &str| {
        let at = xml.find(&format!("<a:t>{t}</a:t>")).unwrap();
        xml[xml[..at].rfind("<a:rPr").unwrap()..at].to_string()
    };
    assert!(rpr("Body").contains("kern=\"1200\""), "{xml}");
    assert!(!rpr("Other").contains("kern"));
}

/// An OPT whose only property is a metroBlob (complex property 0x03A9) zip.
fn metro_opt(blob: &[u8]) -> Vec<u8> {
    let mut body = (0x03A9u16 | 0x8000).to_le_bytes().to_vec();
    body.extend((blob.len() as u32).to_le_bytes());
    body.extend(blob);
    rec(0xF122, 1, &body)
}

#[test]
fn a_text_shape_draws_from_its_metro_copy_unless_it_is_a_placeholder() {
    let xml = |ph: &str| {
        format!(
            "<p:sp xmlns:p=\"p\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\">\
<p:nvSpPr><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:txBody><a:p><a:pPr marL=\"9\"/><a:r><a:t>____</a:t></a:r>\
</a:p></p:txBody></p:sp>"
        )
    };
    let with_blob = |x: String, t: &[u8]| {
        container(
            0xF004,
            0,
            &[
                rec(0xF00A, 202, &u32s(&[0x401, 0xA00])),
                metro_opt(&one_part_zip("drs/shapexml.xml", x.as_bytes())),
                client_anchor(0, 0, 576, 576),
                container(0xF00D, 0, &text(4, t, true)),
            ],
        )
    };
    let plain = with_blob(xml(""), b"Text");
    let placeholder = with_blob(xml("<p:ph type=\"body\"/>"), b"Body");
    let slide = sheet(0x03EE, 0x8000_0001, 7, &[], &[plain, placeholder]);
    let out = convert(&deck_with_master(sheet(0x03F8, 0, 0, &[], &[]), &[slide])).unwrap();
    let slide = part(&out.pptx, "ppt/slides/slide1.xml");
    assert!(
        slide.contains("<a:pPr marL=\"9\"/><a:r><a:t>Text</a:t>"),
        "{slide}"
    );
    assert!(!slide.contains("<p:ph"));
    assert!(texts(&slide).contains(&"Body".to_string()));
    let off = convert_with(
        &deck_with_master(
            sheet(0x03F8, 0, 0, &[], &[]),
            &[sheet(
                0x03EE,
                0x8000_0001,
                7,
                &[],
                &[with_blob(xml(""), b"Text")],
            )],
        ),
        Options {
            metro: false,
            metro_shapes: false,
            now: None,
        },
    )
    .unwrap();
    assert!(!part(&off.pptx, "ppt/slides/slide1.xml").contains("marL=\"9\""));
}

#[test]
fn a_round_trip_layout_gives_placeholder_copies_their_layout_and_frame() {
    let layout =
        b"<p:sldLayout xmlns:p=\"p\" xmlns:a=\"a\"><p:cSld><p:bg><p:bgPr/></p:bg><p:spTree>\
<p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:nvPr><p:ph idx=\"1\"/></p:nvPr></p:nvSpPr></p:sp>\
<p:pic><p:nvPicPr/></p:pic></p:spTree></p:cSld></p:sldLayout>";
    let styles = b"<p:txStyles xmlns:p=\"p\"><p:bodyStyle/></p:txStyles>";
    let master = sheet(
        0x03F8,
        0,
        0,
        &[
            rec(
                0x041E,
                3,
                &one_part_zip("drs/slideLayouts/slideLayout1.xml", layout),
            ),
            rec(
                0x0423,
                0,
                &one_part_zip("drs/slideMasters/slideMaster1.xml", styles),
            ),
        ],
        &[],
    );
    let ph_xml = "<p:sp xmlns:p=\"p\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\">\
<p:nvSpPr><p:nvPr><p:ph idx=\"4294967295\"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:p><a:r><a:t>____</a:t>\
</a:r></a:p></p:txBody></p:sp>";
    let mut ph = container(
        0xF004,
        0,
        &[
            rec(0xF00A, 202, &u32s(&[0x401, 0xA00])),
            metro_opt(&one_part_zip("drs/shapexml.xml", ph_xml.as_bytes())),
            client_anchor(0, 0, 576, 576),
            container(0xF00D, 0, &text(1, b"Body", true)),
        ],
    );
    let extra = placeholder(14);
    let len = u32::from_le_bytes(ph[4..8].try_into().unwrap()) as usize + extra.len();
    ph[4..8].copy_from_slice(&(len as u32).to_le_bytes());
    ph.extend(extra);
    // RoundTripContentMasterId12Atom: main master id, then layout instance 3.
    let link = rec(0x0422, 0, &[1, 0, 0, 0, 3, 0, 0, 0]);
    let slide = sheet(0x03EE, 0x8000_0001, 7, &[link], &[ph]);
    let out = convert(&deck_with_master(master, &[slide])).unwrap();
    assert!(part(&out.pptx, "ppt/slides/_rels/slide1.xml.rels").contains("slideLayout2.xml"));
    let l = part(&out.pptx, "ppt/slideLayouts/slideLayout2.xml");
    assert!(
        l.contains("<p:ph idx=\"1\"/>") && !l.contains("<p:pic>") && !l.contains("<p:bg>"),
        "{l}"
    );
    assert!(part(&out.pptx, "ppt/slideMasters/slideMaster1.xml").contains("<p:bodyStyle/>"));
    let s = part(&out.pptx, "ppt/slides/slide1.xml");
    assert!(
        s.contains("<p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"914400\" cy=\"914400\"/>"),
        "{s}"
    );
    assert!(s.contains("<a:t>Body</a:t>"));
}

#[test]
fn a_line_break_masked_inside_a_run_becomes_a_break() {
    let xml = "<p:sp xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" \
xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><p:txBody>\
<a:p><a:r><a:rPr b=\"1\"/><a:t>___ __ </a:t></a:r></a:p></p:txBody></p:sp>";
    let filled = metro::fill_text(xml, "Two\u{b}is\u{b}").unwrap();
    assert!(
        filled.contains(
            "<a:r><a:rPr b=\"1\"/><a:t>Two</a:t></a:r><a:br><a:rPr b=\"1\"/></a:br>\
<a:r><a:rPr b=\"1\"/><a:t>is</a:t></a:r><a:br><a:rPr b=\"1\"/></a:br></a:p>"
        ),
        "{filled}"
    );
}

/// A text shape that is a placeholder of type `t`.
fn placeholder_shape(t: u8, textbox: Vec<Vec<u8>>) -> Vec<u8> {
    let mut sp = shape(202, client_anchor(0, 0, 2000, 500), &[], textbox);
    let extra = placeholder(t);
    let len = u32::from_le_bytes(sp[4..8].try_into().unwrap()) as usize + extra.len();
    sp[4..8].copy_from_slice(&(len as u32).to_le_bytes());
    sp.extend(extra);
    sp
}

#[test]
fn master_footer_placeholders_show_on_slides_the_flags_turn_on() {
    let field = |kind: u16| {
        let mut tb = text(4, b"*", true);
        tb.push(rec(kind, 0, &u32s(&[0])));
        tb
    };
    let master = sheet(
        0x03F8,
        0,
        0,
        &[],
        &[
            placeholder_shape(9, field(0x0FFA)),
            placeholder_shape(8, field(0x0FD8)),
        ],
    );
    // The second slide has its own footer placeholder, which replaces the master's.
    let slides = [
        sheet(0x03EE, 0x8000_0001, 7, &[], &[]),
        sheet(
            0x03EE,
            0x8000_0001,
            7,
            &[],
            &[placeholder_shape(9, text(4, b"Own", true))],
        ),
    ];
    let footer: Vec<u8> = "Footer note"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let headers_footers = |flags: u16| {
        let mut atom = 0u16.to_le_bytes().to_vec();
        atom.extend(flags.to_le_bytes());
        container(0x0FD9, 3, &[rec(0x0FDA, 0, &atom), rec(0x0FBA, 2, &footer)])
    };
    // HeadersFootersAtom flags: 0x08 slide number, 0x20 footer.
    let out = convert(&deck_with(
        &[headers_footers(0x28)],
        master.clone(),
        &slides,
    ))
    .unwrap();
    let mut first = texts(&part(&out.pptx, "ppt/slides/slide1.xml"));
    first.sort();
    assert_eq!(first, ["1", "Footer note"]);
    let mut second = texts(&part(&out.pptx, "ppt/slides/slide2.xml"));
    second.sort();
    assert_eq!(second, ["2", "Own"]);
    // With the flags off, the master's placeholders stay off the slides.
    let out = convert(&deck_with(&[headers_footers(0)], master, &slides)).unwrap();
    assert!(texts(&part(&out.pptx, "ppt/slides/slide1.xml")).is_empty());
}

#[test]
fn a_slides_own_header_footer_flags_override_the_documents() {
    let field = |kind: u16| {
        let mut tb = text(4, b"*", true);
        tb.push(rec(kind, 0, &u32s(&[0])));
        tb
    };
    let master = sheet(0x03F8, 0, 0, &[], &[placeholder_shape(8, field(0x0FD8))]);
    let flags = |f: u16| {
        let mut atom = 0u16.to_le_bytes().to_vec();
        atom.extend(f.to_le_bytes());
        atom
    };
    // The document turns slide numbers off; the first slide turns them on for itself.
    let own = container(0x0FD9, 0, &[rec(0x0FDA, 0, &flags(0x08))]);
    let slides = [
        sheet(0x03EE, 0x8000_0001, 7, &[own], &[]),
        sheet(0x03EE, 0x8000_0001, 7, &[], &[]),
    ];
    let document = container(0x0FD9, 3, &[rec(0x0FDA, 0, &flags(0))]);
    let out = convert(&deck_with(&[document], master, &slides)).unwrap();
    assert_eq!(texts(&part(&out.pptx, "ppt/slides/slide1.xml")), ["1"]);
    assert!(texts(&part(&out.pptx, "ppt/slides/slide2.xml")).is_empty());
}

#[test]
fn date_fields_show_the_user_date_or_the_conversion_date() {
    let mut tb = text(4, b"*", true);
    tb.push(rec(0x0FF8, 0, &u32s(&[0])));
    let master = sheet(0x03F8, 0, 0, &[], &[placeholder_shape(7, tb)]);
    let slides = [sheet(0x03EE, 0x8000_0001, 7, &[], &[])];
    let date: Vec<u8> = "1/18/01"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    // HeadersFootersAtom: formatId 3 ("October 28, 2026"), then the flags.
    let deck = |flags: u16| {
        let mut atom = 3u16.to_le_bytes().to_vec();
        atom.extend(flags.to_le_bytes());
        let hf = container(0x0FD9, 3, &[rec(0x0FDA, 0, &atom), rec(0x0FBA, 0, &date)]);
        let opts = Options {
            // 2026-10-28 18:02:13 UTC.
            now: Some(1_793_210_533),
            ..Options::default()
        };
        convert_with(&deck_with(&[hf], master.clone(), &slides), opts).unwrap()
    };
    // 0x01 shows the date placeholder; 0x04 selects the user date over today's.
    let user = deck(0x05);
    assert_eq!(
        texts(&part(&user.pptx, "ppt/slides/slide1.xml")),
        ["1/18/01"]
    );
    let today = deck(0x03);
    assert_eq!(
        texts(&part(&today.pptx, "ppt/slides/slide1.xml")),
        ["October 28, 2026"]
    );
}

#[test]
fn a_date_time_field_shows_the_conversion_date_even_with_a_user_date() {
    // DateTimeMCAtom: position 0, format index 0 (short date).
    let mut tb = text(4, b"*", true);
    tb.push(rec(0x0FF7, 0, &[0, 0, 0, 0, 0, 0, 0, 0]));
    let master = sheet(0x03F8, 0, 0, &[], &[placeholder_shape(7, tb)]);
    let slides = [sheet(0x03EE, 0x8000_0001, 7, &[], &[])];
    let date: Vec<u8> = "1/18/01"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut atom = 0u16.to_le_bytes().to_vec();
    // Date shown, user date selected (Word-era decks: PowerPoint still shows today).
    atom.extend(0x05u16.to_le_bytes());
    let hf = container(0x0FD9, 3, &[rec(0x0FDA, 0, &atom), rec(0x0FBA, 0, &date)]);
    let opts = Options {
        // 2026-10-28 18:02:13 UTC.
        now: Some(1_793_210_533),
        ..Options::default()
    };
    let out = convert_with(&deck_with(&[hf], master, &slides), opts).unwrap();
    assert_eq!(
        texts(&part(&out.pptx, "ppt/slides/slide1.xml")),
        ["10/28/26"]
    );
}

#[test]
fn wordart_draws_its_text_in_the_fill_colour() {
    // gtextUNICODE (0xC0) and gtextFont (0xC5) are complex properties: the entry holds
    // the byte length and the strings follow the entries.
    let z = |s: &str| -> Vec<u8> {
        s.encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect()
    };
    let (text, font) = (z("Hello"), z("Impact"));
    let mut body = Vec::new();
    for (id, v) in [
        (0x80C0u16, text.len() as u32),
        (0x80C5, font.len() as u32),
        (0x0181, 0x0000_00FF),
    ] {
        body.extend(id.to_le_bytes());
        body.extend(v.to_le_bytes());
    }
    body.extend(&text);
    body.extend(&font);
    let sp = container(
        0xF004,
        0,
        &[
            rec(0xF00A, 136, &u32s(&[0x401, 0xA00])),
            rec(0xF00B, 3, &body),
            client_anchor(0, 0, 2000, 500),
        ],
    );
    let slides = [sheet(0x03EE, 0x8000_0001, 7, &[], &[sp])];
    let out = convert(&deck_with_master(sheet(0x03F8, 0, 0, &[], &[]), &slides)).unwrap();
    let xml = part(&out.pptx, "ppt/slides/slide1.xml");
    assert_eq!(texts(&xml), ["Hello"]);
    assert!(xml.contains("<a:srgbClr val=\"FF0000\"/>"), "{xml}");
    assert!(xml.contains("typeface=\"Impact\""));
}

#[test]
fn a_master_text_box_slide_number_becomes_a_slidenum_field() {
    // A non-placeholder master text box with a SlideNumberMCAtom: each slide shows its
    // own number, so the master carries an a:fld that pptx-read fills per slide.
    let mut tb = text(4, b"Page *", true);
    tb.push(rec(0x0FD8, 0, &u32s(&[5])));
    let master = sheet(
        0x03F8,
        0,
        0,
        &[],
        &[shape(202, client_anchor(0, 0, 2000, 500), &[], tb)],
    );
    let slides = [sheet(0x03EE, 0x8000_0001, 7, &[], &[])];
    let out = convert(&deck_with_master(master, &slides)).unwrap();
    let m = part(&out.pptx, "ppt/slideMasters/slideMaster1.xml");
    assert!(m.contains(r#"type="slidenum""#), "{m}");
    assert!(m.contains("Page "));
}

#[test]
fn picture_adjustments_become_blip_effects() {
    // A washed-out background picture (contrast 0.2, brightness 0x451F) and a
    // picture with white made transparent.
    let mut p = props::Props::default();
    let mut table = Vec::new();
    for (id, v) in [(0x108u16, 0x3333u32), (0x109, 0x451F), (0x107, 0x00FF_FFFF)] {
        table.extend(id.to_le_bytes());
        table.extend(v.to_le_bytes());
    }
    p.add(
        &table,
        crate::office::ppt::records::Rec {
            inst: 3,
            kind: crate::office::ppt::records::OPT,
            start: 0,
            end: table.len(),
        },
    );
    let xml = native::blip_effects(&p, &props::Scheme::DEFAULT);
    assert!(
        xml.contains("<a:lum bright=\"54001\" contrast=\"-80000\"/>"),
        "{xml}"
    );
    assert!(xml.contains("<a:clrFrom><a:srgbClr val=\"FFFFFF\"/></a:clrFrom><a:clrTo><a:srgbClr val=\"FFFFFF\"><a:alpha val=\"0\"/>"), "{xml}");
    assert_eq!(
        native::blip_effects(&props::Props::default(), &props::Scheme::DEFAULT),
        ""
    );
}
