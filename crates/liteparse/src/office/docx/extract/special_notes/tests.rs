use super::*;
use crate::office::docx::testutil::{W, docx, para, with_related_part};

#[test]
fn settings_select_deferred_stories_without_losing_empty_or_unresolved_content() {
    for kind in ["footnote", "endnote"] {
        let plural = format!("{kind}s");
        let source = format!(
            r#"<n:{plural} xmlns:n="{W}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><n:{kind} n:id="-1" n:type="separator"><n:p/></n:{kind}><n:{kind} n:id="0" n:type="continuationSeparator"><n:p><n:r><n:continuationSeparator/><n:drawing r:embed="imageRelationship"/></n:r></n:p></n:{kind}><n:{kind} n:id="1" n:type="continuationNotice"/><n:{kind} n:id="2" n:type="continuationNotice"><n:p><n:r><n:t>Unselected</n:t></n:r></n:p></n:{kind}><n:{kind} n:id="3" n:type="normal"><n:p/></n:{kind}><n:{kind} n:id="4" n:type="unknown"><n:p/></n:{kind}><n:{kind} n:type="continuationNotice"/></n:{plural}>"#
        );
        let settings = format!(
            r#"<s:settings xmlns:s="{W}"><s:{kind}Pr><s:{kind} s:id="0"/><s:{kind} s:id="1"/><s:{kind} s:id="1"/><s:{kind} s:id="99"/><s:{kind} s:id="bad"/></s:{kind}Pr></s:settings>"#
        );
        for selected in [false, true] {
            let bytes = with_related_part(
                docx(&para("Body"), None, None),
                &plural,
                "special/stories.xml",
                &source,
                false,
            );
            let bytes = if selected {
                with_related_part(bytes, "settings", "preferences.xml", &settings, false)
            } else {
                bytes
            };
            let opened = crate::office::docx::extract::open(
                bytes,
                &crate::office::pptx::config::Config::default(),
            )
            .unwrap();
            let stories = if kind == "footnote" {
                &opened.footnote_specials
            } else {
                &opened.endnote_specials
            };
            assert_eq!(stories.definitions.len(), 4);
            assert!(
                stories
                    .definitions
                    .iter()
                    .all(|s| s.part == "word/special/stories.xml")
            );
            if selected {
                assert_eq!(stories.references, [0, 1, 1, 99]);
                let mark = stories
                    .selected(SpecialNoteKind::ContinuationSeparator)
                    .next()
                    .unwrap();
                assert!(
                    mark.source
                        .path(&["w:p", "w:r", "w:continuationSeparator"])
                        .is_some()
                );
                let notice = stories
                    .selected(SpecialNoteKind::ContinuationNotice)
                    .collect::<Vec<_>>();
                assert_eq!(notice.len(), 1);
                assert_eq!(notice[0].id, 1);
                assert!(notice[0].source.children.is_empty());
                assert_eq!(
                    mark.source
                        .path(&["w:p", "w:r", "w:drawing"])
                        .unwrap()
                        .attr("r:embed"),
                    Some("imageRelationship")
                );
                assert_eq!(stories.selected(SpecialNoteKind::Separator).count(), 0);
            } else {
                assert!(stories.references.is_empty());
                assert_eq!(
                    stories
                        .selected(SpecialNoteKind::ContinuationSeparator)
                        .count(),
                    0
                );
                assert_eq!(
                    stories
                        .selected(SpecialNoteKind::ContinuationNotice)
                        .count(),
                    0
                );
            }
            assert_eq!(opened.notes.len(), usize::from(kind == "footnote"));
            if kind == "footnote" {
                assert!(opened.notes.contains_key(&3));
            }
            assert_eq!(opened.sections[0].blocks.len(), 1);
        }
    }
}

#[test]
fn missing_definitions_and_external_settings_preserve_selection_boundaries() {
    for external in [false, true] {
        let settings = format!(
            r#"<w:settings xmlns:w="{W}"><w:footnotePr><w:footnote w:id="7"/></w:footnotePr><w:endnotePr><w:endnote w:id="8"/></w:endnotePr></w:settings>"#
        );
        let bytes = with_related_part(
            docx(&para("Body"), None, None),
            "settings",
            "settings.xml",
            &settings,
            external,
        );
        let opened = crate::office::docx::extract::open(
            bytes,
            &crate::office::pptx::config::Config::default(),
        )
        .unwrap();
        assert!(opened.footnote_specials.definitions.is_empty());
        assert!(opened.endnote_specials.definitions.is_empty());
        assert_eq!(
            opened.footnote_specials.references,
            if external { vec![] } else { vec![7] }
        );
        assert_eq!(
            opened.endnote_specials.references,
            if external { vec![] } else { vec![8] }
        );
    }
}

#[test]
fn duplicate_definitions_are_retained_without_choosing_precedence() {
    let xml = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="0" w:type="continuationNotice"><w:p><w:r><w:t>First</w:t></w:r></w:p></w:footnote><w:footnote w:id="0" w:type="continuationNotice"><w:p><w:r><w:t>Second</w:t></w:r></w:p></w:footnote></w:footnotes>"#
    );
    let settings = format!(
        r#"<w:settings xmlns:w="{W}"><w:footnotePr><w:footnote w:id="0"/></w:footnotePr></w:settings>"#
    );
    let root = crate::office::pptx::xml::parse_word(&xml).unwrap();
    let settings = crate::office::pptx::xml::parse_word(&settings).unwrap();
    let stories =
        SpecialNoteStories::read(Some(&root), "word/notes.xml", Some(&settings), "footnote");
    assert_eq!(stories.definitions.len(), 2);
    let selected = stories
        .selected(SpecialNoteKind::ContinuationNotice)
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 2);
    assert_eq!(
        selected[0]
            .source
            .path(&["w:p", "w:r", "w:t"])
            .unwrap()
            .text,
        "First"
    );
    assert_eq!(
        selected[1]
            .source
            .path(&["w:p", "w:r", "w:t"])
            .unwrap()
            .text,
        "Second"
    );
}
