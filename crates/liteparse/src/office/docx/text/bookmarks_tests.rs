use super::*;
use crate::office::docx::testutil::docx;

fn compare(before: &str, after: &str) -> Vec<crate::office::docx::paginate::OutPage> {
    let config = crate::office::pptx::config::Config::default();
    let plain = crate::office::docx::extract::open(docx(before, None, None), &config).unwrap();
    let marked = crate::office::docx::extract::open(docx(after, None, None), &config).unwrap();
    let fonts = FontSystem::new(&[], true, &Default::default());
    let baseline = crate::office::docx::extract::layout(&plain, &fonts).pages;
    let candidate = crate::office::docx::extract::layout(&marked, &fonts).pages;
    assert_eq!(baseline.len(), candidate.len(), "page count");
    for (old, new) in baseline.iter().zip(&candidate) {
        let paint = |page: &crate::office::docx::paginate::OutPage| {
            let mut items = page.items.clone();
            items.retain(|item| !matches!(item, Item::Bookmark(_)));
            for item in &mut items {
                if let Item::Text(run) = item {
                    run.bookmarks = None;
                }
            }
            format!("{items:?}")
        };
        assert_eq!(
            paint(old),
            paint(new),
            "glyphs, transforms and paint remain exact"
        );
    }
    candidate
}

#[test]
fn bookmark_boundaries_preserve_coalesced_shaping_and_utf16_offsets() {
    for (left, right, offset) in [
        ("A", "V", 1),
        ("f", "fi", 1),
        ("س", "لام", 1),
        ("😀", "e\u{301}", 2),
        ("e", "\u{301}", 1),
    ] {
        let prefix = format!("<w:p><w:r><w:t>{left}</w:t></w:r>");
        let suffix = format!("<w:r><w:t>{right}</w:t></w:r></w:p>");
        let marker = r#"<w:bookmarkStart w:id="7" w:name="target"/><w:bookmarkEnd w:id="7"/>"#;
        let pages = compare(&(prefix.clone() + &suffix), &(prefix + marker + &suffix));
        let mut boundaries = Vec::new();
        let mut preceding = 0;
        for item in pages.iter().flat_map(|page| &page.items) {
            if let Item::Text(run) = item {
                boundaries.extend(
                    run.bookmarks
                        .iter()
                        .flat_map(|bookmarks| &bookmarks.boundaries)
                        .map(|marker| preceding + marker.offset),
                );
                preceding += run.text.encode_utf16().count();
            }
        }
        assert_eq!(
            boundaries,
            [offset],
            "logical UTF-16 boundary for {left}{right}"
        );
    }
}

#[test]
fn bookmark_empty_and_terminal_break_paragraphs_preserve_flow() {
    for content in [
        "",
        r#"<w:r><w:br w:type="page"/></w:r>"#,
        r#"<w:r><w:br w:type="column"/></w:r>"#,
    ] {
        let marker = r#"<w:bookmarkStart w:id="1" w:name="empty"/><w:bookmarkEnd w:id="1"/>"#;
        let baseline = format!("<w:p><w:r><w:t>BEFORE</w:t></w:r></w:p><w:p>{content}</w:p>");
        let candidate =
            format!("<w:p><w:r><w:t>BEFORE</w:t></w:r></w:p><w:p>{content}{marker}</w:p>");
        let pages = compare(&baseline, &candidate);
        let count = pages
            .iter()
            .flat_map(|page| &page.items)
            .filter(|item| matches!(item, Item::Bookmark(_)))
            .count();
        assert_eq!(count, 1, "one accepted empty boundary for {content}");
    }
}

#[test]
fn bookmark_internal_link_boundary_preserves_shaping() {
    for (left, right) in [("A", "V"), ("f", "fi"), ("س", "لام")] {
        let plain = format!("<w:p><w:r><w:t>{left}</w:t></w:r><w:r><w:t>{right}</w:t></w:r></w:p>");
        let linked = format!(
            "<w:p><w:r><w:t>{left}</w:t></w:r><w:hyperlink w:anchor=\"target\"><w:r><w:t>{right}</w:t></w:r></w:hyperlink></w:p>"
        );
        let config = crate::office::pptx::config::Config::default();
        let fonts = FontSystem::new(&[], true, &Default::default());
        let glyphs = |body: &str| {
            let opened =
                crate::office::docx::extract::open(docx(body, None, None), &config).unwrap();
            crate::office::docx::extract::layout(&opened, &fonts)
                .pages
                .into_iter()
                .flat_map(|page| page.items)
                .flat_map(|item| {
                    if let Item::Text(run) = item {
                        run.glyphs
                            .iter()
                            .map(|glyph| {
                                let mut point =
                                    tiny_skia::Point::from_xy(glyph.x as f32, glyph.y as f32);
                                run.transform.map_point(&mut point);
                                (glyph.id, point.x.to_bits(), point.y.to_bits())
                            })
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    }
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            glyphs(&plain),
            glyphs(&linked),
            "internal source association for {left}{right}"
        );
    }
}

#[test]
fn bookmark_suppressed_cell_paragraphs_preserve_zero_extent_and_targets() {
    let marker = r#"<w:bookmarkStart w:id="1" w:name="target"/><w:bookmarkEnd w:id="1"/>"#;
    let nested = r#"<w:tbl><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>NESTED</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    for (properties, prior) in [("<w:hideMark/>", ""), ("", nested)] {
        let prefix = format!(
            r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:tcPr>{properties}</w:tcPr>{prior}<w:p>"#
        );
        let suffix = "</w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>AFTER</w:t></w:r></w:p>";
        let pages = compare(&(prefix.clone() + suffix), &(prefix + marker + suffix));
        assert_eq!(
            pages
                .iter()
                .flat_map(|page| &page.items)
                .filter(|item| matches!(item, Item::Bookmark(_)))
                .count(),
            1
        );
    }
}

#[test]
fn bookmark_drop_cap_marker_preserves_frame_pairing() {
    let prefix = r#"<w:p><w:pPr><w:framePr w:dropCap="drop" w:lines="2" w:hSpace="80"/><w:rPr><w:sz w:val="58"/></w:rPr></w:pPr>"#;
    let rest = r#"<w:r><w:rPr><w:sz w:val="58"/></w:rPr><w:t>T</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:t>first</w:t><w:br/><w:t>second</w:t><w:br/><w:t>third</w:t></w:r></w:p>"#;
    let marker = r#"<w:bookmarkStart w:id="1" w:name="cap"/><w:bookmarkEnd w:id="1"/>"#;
    let pages = compare(
        &(prefix.to_owned() + rest),
        &(prefix.to_owned() + marker + rest),
    );
    assert_eq!(
        pages
            .iter()
            .flat_map(|page| &page.items)
            .filter_map(|item| if let Item::Text(run) = item {
                Some(
                    run.bookmarks
                        .as_ref()
                        .map_or(0, |bookmarks| bookmarks.boundaries.len()),
                )
            } else {
                None
            })
            .sum::<usize>(),
        1
    );
}

#[test]
fn bookmark_link_suffix_preserves_explicit_kerning_and_tracking() {
    let properties = r#"<w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="24"/><w:kern w:val="2"/><w:spacing w:val="-2"/></w:rPr>"#;
    let run = |text: &str| format!("<w:r>{properties}<w:t>{text}</w:t></w:r>");
    for (left, right, table) in [
        ("A", "V", false),
        ("e", "\u{301}", false),
        ("ل", "ا", false),
        ("A", "V", true),
        ("A ", "V", false),
        ("A", " V", false),
        ("A ", "V", true),
        ("A", " V", true),
        ("A", " ", false),
        ("A", " ", true),
    ] {
        let tail = if right == " " { "" } else { " suffix" };
        let plain = format!("<w:p>{}{}{}</w:p>", run(left), run(right), run(tail));
        let linked = format!(
            "<w:p><w:hyperlink w:anchor=\"target\">{}</w:hyperlink>{}{}</w:p>",
            run(left),
            run(right),
            run(tail)
        );
        let fonts = FontSystem::new(&[], true, &Default::default());
        let render = |body: &str| {
            let opened =
                crate::office::docx::extract::open(docx(body, None, None), &Default::default())
                    .unwrap();
            crate::office::docx::extract::layout(&opened, &fonts).pages
        };
        let positions = |pages: &[crate::office::docx::paginate::OutPage]| {
            pages
                .iter()
                .flat_map(|page| &page.items)
                .flat_map(|item| {
                    if let Item::Text(run) = item {
                        run.glyphs
                            .iter()
                            .map(|glyph| {
                                let mut point =
                                    tiny_skia::Point::from_xy(glyph.x as f32, glyph.y as f32);
                                run.transform.map_point(&mut point);
                                (glyph.id, point.x.to_bits(), point.y.to_bits())
                            })
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    }
                })
                .collect::<Vec<_>>()
        };
        let wrap = |body: &str| {
            if table {
                format!(
                    r#"<w:tbl><w:tblGrid><w:gridCol w:w="6000"/></w:tblGrid><w:tr><w:tc>{body}</w:tc></w:tr></w:tbl>"#
                )
            } else {
                body.to_owned()
            }
        };
        let before = render(&wrap(&plain));
        let after = render(&wrap(&linked));
        assert!(!positions(&before).is_empty());
        assert_eq!(
            positions(&before),
            positions(&after),
            "source ownership keeps explicit kerning and tracking"
        );
        let exported = crate::office::pptx::output::text_json(
            &fonts,
            &crate::office::pptx::scene::Scene {
                items: after[0].items.clone(),
                ..Default::default()
            },
            false,
            (0.0, 0.0),
            None,
        );
        let linked_text: String = exported["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item.pointer("/nativeSource/link/destination").is_some())
            .filter_map(|item| item["unicode"].as_str())
            .collect();
        assert_eq!(
            linked_text, left,
            "the suffix remains outside the source link"
        );
        let text: String = exported["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["unicode"].as_str())
            .collect();
        assert_eq!(text, format!("{left}{right}{tail}"));
    }
}

fn compare_link_paint(plain: &str, linked: &str) -> Vec<crate::office::docx::paginate::OutPage> {
    compare_link_paint_mode(plain, linked, false).0
}

fn compare_link_paint_mode(
    plain: &str,
    linked: &str,
    auto_hyphenation: bool,
) -> (Vec<crate::office::docx::paginate::OutPage>, FontSystem) {
    let fonts = FontSystem::new(&[], true, &Default::default());
    let layout = |body: &str| {
        let mut bytes = docx(body, None, None);
        if auto_hyphenation {
            bytes = crate::office::docx::testutil::with_related_part(
                bytes,
                "settings",
                "settings.xml",
                r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:autoHyphenation/><w:hyphenationZone w:val="0"/></w:settings>"#,
                false,
            );
        }
        let opened = crate::office::docx::extract::open(bytes, &Default::default()).unwrap();
        crate::office::docx::extract::layout(&opened, &fonts).pages
    };
    let before = layout(plain);
    let after = layout(linked);
    let paint = |pages: &[crate::office::docx::paginate::OutPage]| {
        pages
            .iter()
            .map(|page| {
                let mut items = page.items.clone();
                for item in &mut items {
                    if let Item::Text(run) = item {
                        run.source = None;
                        run.bookmarks = None;
                    }
                }
                format!("{items:?}")
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(paint(&before), paint(&after), "glyphs and decoration paths");
    (after, fonts)
}

#[test]
fn bookmark_link_language_boundary_preserves_underline_path() {
    let run = |text: &str, language: &str| {
        format!(
            r#"<w:r><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="22"/><w:u w:val="single"/><w:lang w:val="{language}"/></w:rPr><w:t>{text}</w:t></w:r>"#
        )
    };
    let text = run("Alpha", "en-AU") + &run(" beta", "en-US");
    let after = compare_link_paint(
        &format!("<w:p>{text}</w:p>"),
        &format!(r#"<w:p><w:hyperlink w:anchor="target">{text}</w:hyperlink></w:p>"#),
    );
    assert!(
        after[0]
            .items
            .iter()
            .any(|item| !matches!(item, Item::Text(_))),
        "visible underline path"
    );
}

#[test]
fn bookmark_link_boundary_preserves_unidentified_fit_text() {
    let run = |text: &str| {
        format!(r#"<w:r><w:rPr><w:fitText w:val="1440"/></w:rPr><w:t>{text}</w:t></w:r>"#)
    };
    let left = run("A");
    let right = run("V");
    compare_link_paint(
        &format!("<w:p>{left}{right}</w:p>"),
        &format!(r#"<w:p><w:hyperlink w:anchor="target">{left}</w:hyperlink>{right}</w:p>"#),
    );
}

#[test]
fn bookmark_link_boundary_preserves_hyphenation_ownership() {
    const CHILD: &str = "DOCX_BOOKMARK_HYPHEN_TEST_CHILD";
    // Hyphenation settings are process-global; the child runs this test alone.
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "text::bookmarks_tests::bookmark_link_boundary_preserves_hyphenation_ownership",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let run = |text: &str| {
        format!(
            r#"<w:r><w:rPr><w:kern w:val="2"/><w:lang w:val="en-US"/></w:rPr><w:t>{text}</w:t></w:r>"#
        )
    };
    let section = r#"<w:sectPr><w:pgSz w:w="1200" w:h="10000"/><w:pgMar w:left="0" w:right="0" w:top="0" w:bottom="0"/></w:sectPr>"#;
    let prefix = run("X ");
    for (word, boundary) in [
        ("representation", 3),
        ("representation", 4),
        ("identification", 7),
    ] {
        for link_left in [true, false] {
            let left = run(&word[..boundary]);
            let right = run(&word[boundary..]);
            let marker =
                r#"<w:bookmarkStart w:id="1" w:name="boundary"/><w:bookmarkEnd w:id="1"/>"#;
            let content = if link_left {
                format!(r#"<w:hyperlink w:anchor="target">{left}</w:hyperlink>{marker}{right}"#)
            } else {
                format!(r#"{left}{marker}<w:hyperlink w:anchor="target">{right}</w:hyperlink>"#)
            };
            let (after, fonts) = compare_link_paint_mode(
                &format!("<w:p>{prefix}{left}{right}</w:p>{section}"),
                &format!("<w:p>{prefix}{content}</w:p>{section}"),
                true,
            );
            let mut all = String::new();
            let mut owned = String::new();
            let mut markers = Vec::new();
            for item in after.iter().flat_map(|page| &page.items) {
                if let Item::Text(run) = item {
                    if let Some(meta) = &run.bookmarks {
                        for marker in &meta.boundaries {
                            markers.push(all.encode_utf16().count() + marker.offset);
                        }
                    }
                    all.push_str(&run.text);
                    if let Some(spans) = run
                        .bookmarks
                        .as_ref()
                        .map(|meta| &meta.source_spans)
                        .filter(|spans| !spans.is_empty())
                    {
                        for span in spans {
                            if span
                                .source
                                .as_ref()
                                .is_some_and(|source| source.link.is_some())
                            {
                                owned.push_str(&run.text[span.text.clone()]);
                            }
                        }
                    } else if run
                        .source
                        .as_ref()
                        .is_some_and(|source| source.link.is_some())
                    {
                        owned.push_str(&run.text);
                    }
                }
            }
            let mut exported_all = String::new();
            let mut exported_owned = String::new();
            for page in &after {
                let output = crate::office::pptx::output::text_json(
                    &fonts,
                    &crate::office::pptx::scene::Scene {
                        items: page.items.clone(),
                        ..Default::default()
                    },
                    false,
                    (0.0, 0.0),
                    None,
                );
                for item in output["items"].as_array().unwrap() {
                    let text = item["unicode"].as_str().unwrap();
                    exported_all.push_str(text);
                    if item.pointer("/nativeSource/link/destination").is_some() {
                        exported_owned.push_str(text);
                    }
                }
            }
            assert_eq!(exported_all, all, "exported raw text");
            assert_eq!(exported_owned, owned, "exported exact link ownership");
            assert!(
                all.contains('-'),
                "the control must actually hyphenate: {all}"
            );
            assert_eq!(all.replace('-', ""), format!("X {word}"));
            assert_eq!(
                owned.replace('-', ""),
                if link_left {
                    &word[..boundary]
                } else {
                    &word[boundary..]
                },
                "reshaping preserves exact source ownership"
            );
            assert_eq!(markers.len(), 1, "one accepted marker through reshaping");
            assert_eq!(
                all[..markers[0]].replace('-', ""),
                format!("X {}", &word[..boundary]),
                "marker remains at its source boundary"
            );
        }
    }
}
