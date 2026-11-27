use super::*;

/// A deck of `n` slides, each hidden (`show="0"`) when `hidden` is set.
pub(super) fn deck(n: usize, hidden: bool) -> Vec<u8> {
    use std::io::Write;
    let ns_r = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let ns_p = "http://schemas.openxmlformats.org/presentationml/2006/main";
    let rels = |body: String| {
        format!(
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
        )
    };
    let mut parts = vec![(
        "_rels/.rels".to_string(),
        rels(format!(
            r#"<Relationship Id="rId1" Type="{ns_r}/officeDocument" Target="ppt/presentation.xml"/>"#
        )),
    )];
    let ids: String = (1..=n)
        .map(|i| format!(r#"<p:sldId id="{}" r:id="rId{i}"/>"#, 255 + i))
        .collect();
    parts.push((
        "ppt/presentation.xml".into(),
        format!(
            r#"<p:presentation xmlns:p="{ns_p}" xmlns:r="{ns_r}"><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
        ),
    ));
    let targets: String = (1..=n)
        .map(|i| {
            format!(
                r#"<Relationship Id="rId{i}" Type="{ns_r}/slide" Target="slides/slide{i}.xml"/>"#
            )
        })
        .collect();
    parts.push(("ppt/_rels/presentation.xml.rels".into(), rels(targets)));
    let show = if hidden { r#" show="0""# } else { "" };
    for i in 1..=n {
        parts.push((
            format!("ppt/slides/slide{i}.xml"),
            format!(r#"<p:sld xmlns:p="{ns_p}"{show}><p:cSld><p:spTree/></p:cSld></p:sld>"#),
        ));
    }
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
    buf.into_inner()
}

fn index_for(bytes: Vec<u8>, tag: &str) -> Value {
    let dir = std::env::temp_dir().join(format!("pptx-read-extract-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.pptx");
    std::fs::write(&input, bytes).unwrap();
    let (index, pages) = (dir.join("index.json"), dir.join("pages.jsonl"));
    run_read(
        r#"{"text":{"enabled":true}}"#,
        input.to_str().unwrap(),
        index.to_str().unwrap(),
        pages.to_str().unwrap(),
    )
    .unwrap();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&index).unwrap()).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    v
}

fn codes(v: &Value) -> Vec<String> {
    v["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["code"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_deck_with_every_slide_hidden_emits_them_with_a_warning() {
    let v = index_for(deck(2, true), "hidden");
    assert_eq!(v["totalPages"], 2);
    assert_eq!(v["slides"][1]["page"], 1);
    assert_eq!(v["slides"][1]["hidden"], true);
    assert!(codes(&v).contains(&"ALL_SLIDES_HIDDEN".to_string()));

    let v = index_for(deck(2, false), "visible");
    assert_eq!(v["totalPages"], 2);
    assert!(!codes(&v).contains(&"ALL_SLIDES_HIDDEN".to_string()));
}

#[test]
fn clip_drops_text_items_wholly_off_the_page() {
    use crate::office::pptx::scene::GlyphRun;
    let run = |x: f32, text: &str| {
        Item::Text(GlyphRun {
            bookmarks: None,
            source: None,
            preserve_whitespace: false,
            face: fontdb::ID::dummy(),
            font_name: "Sans".into(),
            size: 12.0,
            color: crate::office::pptx::color::Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            glyphs: Vec::new(),
            transform: tiny_skia::Transform::from_translate(x, 50.0),
            text: text.into(),
            width: 30.0,
            ascent: 10.0,
            descent: 2.0,
            outline: None,
            fake_bold: false,
            fake_italic: false,
            shape: None,
            paragraph: 0,
            hyperlink: None,
            space_before: false,
            drawn: true,
            warp: None,
        })
    };
    let scene = Scene {
        items: vec![run(10.0, "on"), run(90.0, "edge"), run(130.0, "off")],
        ..Default::default()
    };
    let fonts = FontSystem::new(&[], false, &Default::default());
    let texts = |clip| -> Vec<String> {
        crate::office::pptx::output::text_json(&fonts, &scene, false, (0.0, 0.0), clip)["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["unicode"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(texts(Some((100.0, 100.0))), vec!["on", "edge"]);
    assert_eq!(texts(None), vec!["on", "edge", "off"]);
}

#[test]
fn records_with_raw_paths_serialize_like_values() {
    let paths = br#"{"shapes":[{"coords":{"x":1.5,"y":2.0,"w":3.0,"h":4.0}}],"lines":[]}"#;
    let mut obj = Map::new();
    obj.insert("page".into(), json!(3));
    obj.insert("paths".into(), Value::Null);
    obj.insert("text".into(), json!([{ "str": "a \"q\" é" }]));
    let record = Record {
        value: Value::Object(obj.clone()),
        paths: Some(paths.to_vec()),
    };
    obj.insert("paths".into(), serde_json::from_slice(paths).unwrap());
    assert_eq!(
        record.to_bytes().unwrap(),
        serde_json::to_vec(&Value::Object(obj)).unwrap()
    );
}

#[test]
fn paths_are_compact_json_in_value_order() {
    use crate::office::pptx::color::Rgba;
    use crate::office::pptx::scene::{LineCap, LineJoin, Paint, PathItem, Stroke};
    let rect = |x: f32| {
        tiny_skia::PathBuilder::from_rect(tiny_skia::Rect::from_xywh(x, 1.0, 10.0, 5.0).unwrap())
    };
    let red = Rgba {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    let mut scene = Scene::default();
    for (i, x) in [0.0f32, 20.0, 40.0].into_iter().enumerate() {
        scene.items.push(Item::Path(PathItem {
            path: rect(x),
            fill: (i != 1).then_some(Paint::Solid(red)),
            stroke: (i == 2).then(|| Stroke {
                paint: Paint::Solid(red),
                width: 1.25,
                dash: Vec::new(),
                cap: LineCap::Flat,
                join: LineJoin::Miter,
                head: None,
                tail: None,
            }),
            has_curve: false,
            shape: None,
            from_text: false,
        }));
    }
    let bytes = crate::office::pptx::output::paths_json(&scene, 100, 0.0, 0.0);
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    // Re-serializing the parsed value gives the same bytes: compact, same key order.
    assert_eq!(serde_json::to_vec(&v).unwrap(), bytes);
    assert_eq!(v["shapes"].as_array().unwrap().len(), 2);
    assert_eq!(v["lines"].as_array().unwrap().len(), 8);
    assert_eq!(v["shapes"][1]["strokeColor"], v["shapes"][1]["fillColor"]);
    let capped = crate::office::pptx::output::paths_json(&scene, 1, 0.0, 0.0);
    let v: Value = serde_json::from_slice(&capped).unwrap();
    assert_eq!(v["shapes"].as_array().unwrap().len(), 1);
    let empty = crate::office::pptx::output::paths_json(&Scene::default(), 100, 0.0, 0.0);
    assert_eq!(empty, br#"{"shapes":[],"lines":[]}"#);
}

#[test]
fn last_use_follows_relationships_but_not_slide_links() {
    let cfg: Config = serde_json::from_str("{}").unwrap();
    let doc = Document::open(deck(3, false), &cfg.limits).unwrap();
    let last = part_last_use(&doc, &[0, 1, 2]);
    assert_eq!(last.get(&doc.slides[0].part), Some(&0));
    assert_eq!(last.get(&doc.slides[2].part), Some(&2));
    // Reversed page order: the first slide is now used last.
    let last = part_last_use(&doc, &[2, 1, 0]);
    assert_eq!(last.get(&doc.slides[0].part), Some(&2));
}

#[test]
fn page_weights_count_slide_trees() {
    let doc = Document::open(deck(3, false), &Default::default()).unwrap();
    let weights = page_weights(&doc, &[0, 1, 2]);
    let xml = doc.pkg.part_size("ppt/slides/slide1.xml").unwrap();
    assert_eq!(weights, vec![xml * TREE_BYTES_PER_XML_BYTE; 3]);
    assert_eq!(
        doc.pkg.read_head("ppt/slides/slide1.xml", 5).unwrap(),
        b"<p:sl"
    );
}

#[test]
fn binary_powerpoint_input_is_converted_and_rendered() {
    use crate::office::ppt::fixtures::*;
    let shape = text_shape(client_anchor(0, 0, 100, 100), text(4, b"Slide text", true));
    let v = index_for(simple_ppt((5760, 4320), &[(vec![], vec![shape])]), "ppt");
    assert_eq!(v["totalPages"], 1, "{v}");
}

#[test]
fn encrypted_binary_powerpoint_is_protected() {
    use crate::office::ppt::fixtures::*;
    let mut s = Stream::default();
    let doc = s.push(document((5760, 4320), &[]));
    let d = s.directory(&[(1, doc)]);
    let e = s.user_edit(0, d, 1);
    let file = cfb(&[
        ("PowerPoint Document", s.0),
        // The token of a Current User atom written for an encrypted file.
        ("Current User", current_user(e, 0xF3D1_C4DF)),
    ]);
    assert_eq!(
        index_for(file, "ppt-encrypted")["error"],
        "PDF_IS_PROTECTED"
    );
}
