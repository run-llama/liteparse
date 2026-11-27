use super::*;
fn fixture() -> (Vec<PlainParagraph>, FootnoteDocument) {
    let body = super::super::tests::paragraph("\u{2}\r");
    let mut note = super::super::tests::paragraph("A\r");
    note.source_cp = [2, 4];
    note.runs[0].source_cp = [2, 4];
    let plan = FootnoteDocument {
        kind: NoteKind::Footnote,
        source_cp: [2, 5],
        source_text: "A\r\r".into(),
        terminal_guard: [4, 5],
        position: Some(NotePosition::PageBottom),
        position_basis: Some("synthetic"),
        sections: vec![],
        coexisting_endnotes: vec![],
        coexisting_endnote_source: None,
        separators: vec![],
        notes: vec![Footnote {
            id: 1,
            body_cp: 0,
            raw_reference_index: 0,
            section_index: 0,
            marker_status: crate::office::doc::notes::FootnoteMarkerStatus::AutomaticSpecial,
            source_cp: [2, 4],
            marker_cp: None,
            authored_reference: None,
            label: "1".into(),
            paragraphs: vec![note],
        }],
    };
    (vec![body], plan)
}
#[test]
fn note_source_partition_rejects_text_loss_and_wrong_reference() {
    let (body, mut plan) = fixture();
    plan.validate(&body, 1).unwrap();
    plan.notes[0].paragraphs[0].runs[0].text = "B\r".into();
    assert!(plan.validate(&body, 1).is_err());
    plan.notes[0].paragraphs[0].runs[0].text = "A\r".into();
    plan.notes[0].body_cp = 1;
    assert!(plan.validate(&body, 1).is_err());
    plan.notes[0].body_cp = 0;
    plan.terminal_guard = [3, 5];
    assert!(plan.validate(&body, 1).is_err());
}
