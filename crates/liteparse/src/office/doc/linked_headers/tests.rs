use super::*;
use crate::office::doc::headers::{HeaderReference, HeaderSection, HeaderStory, SourceRange};
fn range(a: usize, b: usize) -> SourceRange {
    SourceRange {
        start_cp: a,
        end_cp: b,
    }
}
fn fixture() -> (Inspection, HeaderInspection) {
    let source = Inspection {
        schema_version: 1,
        status: "decoded",
        binary_version: 0x112,
        trailing_container_bytes: 0,
        piece_count: 1,
        stories: vec![
            crate::office::doc::Story {
                kind: "body",
                start_cp: 0,
                utf16_units: 6,
                text: "ABCDEF".into(),
            },
            crate::office::doc::Story {
                kind: "headers",
                start_cp: 10,
                utf16_units: 7,
                text: "A😀\r\r\r\r".into(),
            },
        ],
        trailing_utf16_units: vec![],
    };
    let stories = vec![
        HeaderStory {
            index: 0,
            role: Some("odd_header"),
            section_index: Some(0),
            source: range(10, 15),
            content: Some(range(10, 14)),
            guard: Some(range(14, 15)),
            explicit_blank: false,
            unresolved: vec![],
        },
        HeaderStory {
            index: 1,
            role: Some("odd_header"),
            section_index: Some(2),
            source: range(15, 17),
            content: Some(range(15, 16)),
            guard: Some(range(16, 17)),
            explicit_blank: true,
            unresolved: vec![],
        },
    ];
    let sections = (0..3)
        .map(|i| HeaderSection {
            index: i,
            plc_end_cp: i * 2 + 2,
            source: range(i * 2, i * 2 + 2),
            sepx_offset: None,
            properties: vec![],
            different_first_page: Some(false),
            legacy_presence_flags: None,
            references: vec![HeaderReference {
                role: "odd_header",
                status: if i == 1 { "inherited" } else { "local" },
                local_story_index: if i == 1 {
                    None
                } else {
                    Some(usize::from(i == 2))
                },
                origin_story_index: Some(usize::from(i == 2)),
                origin_section_index: Some(if i == 2 { 2 } else { 0 }),
            }],
            unresolved: vec![],
        })
        .collect();
    let headers = HeaderInspection {
        status: "source_header_footer_ownership",
        binary_version: 0x112,
        source: range(10, 17),
        different_even_and_odd_pages: Some(false),
        dop_first_byte: None,
        stories,
        sections,
        terminal_sentinel: None,
        ignored_final_plc_cp: None,
        diagnostics: vec![],
        unresolved: ["visibility", "page_selection", "placement"],
    };
    (source, headers)
}
#[test]
fn inherited_content_is_deduplicated_and_explicit_blank_remains_local() {
    let (source, headers) = fixture();
    let out = extract(
        &source,
        &crate::office::doc::transcribe(&source),
        &headers,
        None,
    )
    .unwrap();
    assert_eq!(out.contents.len(), 2);
    assert_eq!(out.contents[0].text.as_deref(), Some("A😀\n"));
    assert_eq!(out.contents[1].text.as_deref(), Some("\n"));
    assert!(out.contents[1].explicit_blank);
    assert_eq!(out.sections[1].references[0].origin_story_index, Some(0));
    assert_eq!(out.sections[2].references[0].origin_story_index, Some(1));
    assert_eq!(out.page_instances, "not_established");
    assert!(
        out.sections[0]
            .unresolved
            .contains(&"section_roles_incomplete")
    );
    assert!(
        out.contents[0]
            .unresolved
            .contains(&"hidden_visibility_unresolved")
    );
}
#[test]
fn one_unresolved_projection_boundary_retains_other_content() {
    let (source, mut headers) = fixture();
    headers.stories[0].content = Some(range(11, 14));
    let out = extract(
        &source,
        &crate::office::doc::transcribe(&source),
        &headers,
        None,
    )
    .unwrap();
    assert!(out.contents[0].text.is_none());
    assert!(
        out.contents[0]
            .unresolved
            .contains(&"logical_projection_boundary_unresolved")
    );
    assert_eq!(out.contents[1].text.as_deref(), Some("\n"));
}
#[test]
fn wrong_role_or_future_origin_cannot_claim_inheritance() {
    let (source, mut headers) = fixture();
    headers.sections[0].references[0].origin_story_index = Some(1);
    assert!(
        extract(
            &source,
            &crate::office::doc::transcribe(&source),
            &headers,
            None
        )
        .is_err()
    );
    let (_, mut headers) = fixture();
    headers.sections[1].references[0].role = "odd_footer";
    assert!(
        extract(
            &source,
            &crate::office::doc::transcribe(&source),
            &headers,
            None
        )
        .is_err()
    );
}
#[test]
fn generated_start_label_is_owned_and_storage_guards_are_excluded() {
    let (source, headers) = fixture();
    let mut t = crate::office::doc::transcribe(&source);
    let s = &mut t.stories[1];
    s.text.insert_str(0, "1. ");
    for p in &mut s.segments {
        p.output_start += 3;
        p.output_end += 3;
    }
    s.segments.insert(
        0,
        crate::office::doc::SourceSegment {
            kind: "generated_list_label",
            start_cp: 10,
            end_cp: 10,
            output_start: 0,
            output_end: 3,
        },
    );
    let out = extract(&source, &t, &headers, None).unwrap();
    assert_eq!(out.contents[0].text.as_deref(), Some("1. A😀\n"));
    assert_eq!(out.contents[1].text.as_deref(), Some("\n"));
}

#[test]
fn malformed_body_partition_and_reference_status_are_rejected() {
    for case in 0..6 {
        let (source, mut headers) = fixture();
        match case {
            0 => headers.sections[0].source.start_cp = 1,
            1 => headers.sections[1].source.start_cp = 1,
            2 => headers.sections[2].source.end_cp = 7,
            3 => headers.sections[2].source.end_cp = 5,
            4 => headers.sections[1].references[0].status = "local",
            _ => headers.sections[0].references[0].origin_section_index = None,
        }
        assert!(
            extract(
                &source,
                &crate::office::doc::transcribe(&source),
                &headers,
                None
            )
            .is_err()
        );
    }
}

#[test]
fn inherited_reference_cannot_skip_a_newer_local_override() {
    let (source, mut headers) = fixture();
    headers.stories[1].section_index = Some(1);
    headers.sections[1].references[0] = HeaderReference {
        role: "odd_header",
        status: "local",
        local_story_index: Some(1),
        origin_story_index: Some(1),
        origin_section_index: Some(1),
    };
    headers.sections[2].references[0] = HeaderReference {
        role: "odd_header",
        status: "inherited",
        local_story_index: None,
        origin_story_index: Some(1),
        origin_section_index: Some(1),
    };
    assert!(
        extract(
            &source,
            &crate::office::doc::transcribe(&source),
            &headers,
            None
        )
        .is_ok()
    );
    headers.sections[2].references[0].origin_story_index = Some(0);
    headers.sections[2].references[0].origin_section_index = Some(0);
    assert!(
        extract(
            &source,
            &crate::office::doc::transcribe(&source),
            &headers,
            None
        )
        .is_err()
    );
}
