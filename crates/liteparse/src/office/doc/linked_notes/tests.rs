use super::*;
fn fixture() -> (Inspection, NoteInspection) {
    let stories = [
        ("body", 0, "*x*\r"),
        ("footnotes", 4, "A😀\rB\r\r"),
        ("endnotes", 12, "C\r\r"),
    ]
    .into_iter()
    .map(|(kind, start_cp, text)| crate::office::doc::Story {
        kind,
        start_cp,
        utf16_units: text.encode_utf16().count(),
        text: text.into(),
    })
    .collect();
    let source = Inspection {
        schema_version: 1,
        status: "decoded",
        binary_version: 0x112,
        trailing_container_bytes: 0,
        piece_count: 1,
        stories,
        trailing_utf16_units: vec![],
    };
    let link = |body_cp, note_start_cp, note_end_cp| crate::office::doc::notes::NoteLink {
        body_cp,
        note_start_cp,
        note_end_cp,
        automatic: false,
        raw_index: 0,
    };
    let links = NoteInspection {
        status: "source_note_relationships",
        footnotes: vec![link(0, 4, 8), link(2, 8, 10)],
        endnotes: vec![link(1, 12, 14)],
        unresolved: ["", "", ""],
    };
    (source, links)
}
#[test]
fn linked_content_preserves_unicode_and_reference_order_across_kinds() {
    let (source, links) = fixture();
    let t = crate::office::doc::transcribe(&source);
    let out = extract(&source, &t, &links, None).unwrap();
    assert_eq!(
        out.notes
            .iter()
            .map(|n| n.text.as_str())
            .collect::<Vec<_>>(),
        ["A😀\n", "C\n", "B\n"]
    );
    assert_eq!(out.notes[0].logical_utf8_range, [0, 6]);
    assert_eq!(out.notes[1].kind, "endnotes");
    assert!(
        out.notes
            .iter()
            .all(|n| n.unresolved.contains(&"hidden_visibility_unresolved"))
    );
}
#[test]
fn malformed_range_or_projected_partition_is_rejected() {
    let (source, mut links) = fixture();
    let mut t = crate::office::doc::transcribe(&source);
    links.footnotes[0].note_end_cp = 6;
    assert!(extract(&source, &t, &links, None).is_err());
    let (_, links) = fixture();
    t.stories[1].segments.remove(0);
    assert!(extract(&source, &t, &links, None).is_err());
}
#[test]
fn generated_anchor_belongs_to_following_note_and_tail_stays_outside() {
    let (source, links) = fixture();
    let mut t = crate::office::doc::transcribe(&source);
    let s = &mut t.stories[1];
    s.text.insert_str(6, "9. ");
    for segment in &mut s.segments {
        if segment.output_start >= 6 {
            segment.output_start += 3;
            segment.output_end += 3;
        }
    }
    let i = s.segments.iter().position(|x| x.start_cp == 8).unwrap();
    s.segments.insert(
        i,
        crate::office::doc::SourceSegment {
            kind: "generated_list_label",
            start_cp: 8,
            end_cp: 8,
            output_start: 6,
            output_end: 9,
        },
    );
    let out = extract(&source, &t, &links, None).unwrap();
    assert_eq!(out.notes[2].text, "9. B\n");
    assert_eq!(out.notes[0].text, "A😀\n");
}

#[test]
fn suppressed_text_and_unknown_reference_are_explicit() {
    let (source, links) = fixture();
    let mut t = crate::office::doc::transcribe(&source);
    let note = &mut t.stories[1];
    note.text.replace_range(0..5, "");
    note.segments[0].kind = "hidden_text";
    note.segments[0].output_end = 0;
    for segment in note.segments.iter_mut().skip(1) {
        segment.output_start -= 5;
        segment.output_end -= 5;
    }
    let out = extract(&source, &t, &links, None).unwrap();
    assert_eq!(out.notes[0].text, "\n");
    assert_eq!(out.notes[0].source_cp, [4, 8]);
    assert!(
        out.notes[0]
            .unresolved
            .contains(&"hidden_visibility_unresolved")
    );
}

#[test]
fn deleted_reference_retains_link_and_source_relationship_limitations() {
    let (source, mut links) = fixture();
    links.unresolved = [
        "reference_revision_visibility",
        "numbering",
        "label_projection",
    ];
    let mut t = crate::office::doc::transcribe(&source);
    let body = &mut t.stories[0];
    body.text.clear();
    for s in &mut body.segments {
        s.kind = "deleted_revision";
        s.output_start = 0;
        s.output_end = 0;
    }
    let out = extract(&source, &t, &links, None).unwrap();
    assert_eq!(out.source_relationship_unresolved, links.unresolved);
    assert!(
        out.notes[0]
            .unresolved
            .contains(&"body_reference_suppressed_or_unresolved")
    );
    assert_eq!(out.notes[0].text, "A😀\n");
}

#[test]
fn two_note_kinds_cannot_claim_the_same_body_reference() {
    let (source, mut links) = fixture();
    links.endnotes[0].body_cp = links.footnotes[0].body_cp;
    assert!(matches!(
        extract(
            &source,
            &crate::office::doc::transcribe(&source),
            &links,
            None
        ),
        Err(Error::Invalid("linked note duplicate body reference"))
    ));
}
