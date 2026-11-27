use super::*;
use crate::office::doc::{
    Story,
    transcription::{SourceSegment, TranscribedStory},
};
fn modifier(sprm: u16, operand: &[u8]) -> PropertyModifier {
    PropertyModifier {
        sprm,
        operand: operand.to_vec(),
    }
}
#[test]
fn font_and_language_aliases_are_applied_in_order() {
    let mut state = State::default();
    for p in [
        modifier(0x4873, &[9, 4]),
        modifier(0x486d, &[12, 4]),
        modifier(0x4a51, &[1, 0]),
        modifier(0x4a51, &[2, 0]),
    ] {
        assert!(property(&mut state, &p, 3).unwrap());
    }
    assert_eq!(state.font, Some(2));
    assert_eq!(state.language, Some(0x040c));
}
#[test]
fn font_selection_and_reset_operators_remain_unresolved() {
    for op in [
        0x286f, 0x085a, 0x0882, 0x2a33, 0x4a30, 0xca47, 0x2a86, 0x4a3d, 0x2a83,
    ] {
        assert!(!property(&mut State::default(), &modifier(op, &[0]), 2).unwrap());
    }
}
#[test]
fn malformed_font_and_appearance_operands_are_rejected() {
    for p in [
        modifier(0x4a51, &[1]),
        modifier(0x4a51, &[2, 0]),
        modifier(0x0835, &[2]),
        modifier(0x4a43, &[0, 0]),
        modifier(0x484b, &[255, 255]),
        modifier(0x2a48, &[3]),
        modifier(0x6870, &[1, 2, 3, 255]),
    ] {
        assert!(!property(&mut State::default(), &p, 2).unwrap());
    }
}
fn fixture() -> (Inspection, Transcription, OrdinarySymbols) {
    let text = "😀\u{f061}X".to_string();
    let source = Inspection {
        schema_version: 1,
        status: "fixture",
        binary_version: 0x101,
        trailing_container_bytes: 0,
        piece_count: 1,
        stories: vec![Story {
            kind: "body",
            start_cp: 0,
            utf16_units: 4,
            text: text.clone(),
        }],
        trailing_utf16_units: vec![],
    };
    let input = Transcription {
        schema_version: 1,
        status: "fixture",
        unresolved: vec![],
        stories: vec![TranscribedStory {
            kind: "body",
            text,
            segments: vec![SourceSegment {
                kind: "text",
                start_cp: 0,
                end_cp: 4,
                output_start: 0,
                output_end: 8,
            }],
            diagnostics: vec![],
            fields_balanced: true,
            unresolved_control_units: 0,
        }],
    };
    let evidence = OrdinarySymbols {
        status: "fixture",
        embedded_font_table_present: Some(false),
        spans: vec![OrdinarySymbol {
            story: "body",
            source_cp: 2,
            source_code: 0xf061,
            font_index: Some(1),
            paragraph_style: Some(0),
            unicode: Some('α'),
            basis: "fixture",
            unresolved: None,
        }],
    };
    (source, input, evidence)
}
#[test]
fn unicode_replacement_preserves_surrogate_and_source_partition() {
    let (source, input, evidence) = fixture();
    let output = apply(&source, &input, &evidence).unwrap();
    assert_eq!(output.stories[0].text, "😀αX");
    let s = &output.stories[0].segments;
    assert_eq!(
        (
            s[1].start_cp,
            s[1].end_cp,
            s[1].output_start,
            s[1].output_end
        ),
        (2, 3, 4, 6)
    );
    assert_eq!(source.stories[0].text, "😀\u{f061}X");
}
#[test]
fn unresolved_private_use_code_stays_verbatim() {
    let (source, input, mut evidence) = fixture();
    evidence.spans[0].unicode = None;
    evidence.spans[0].unresolved = Some("unknown_font");
    let output = apply(&source, &input, &evidence).unwrap();
    assert_eq!(output.stories[0].text, input.stories[0].text);
    assert_eq!(output.stories[0].segments.len(), 1);
}
#[test]
fn generated_anchors_and_suppressed_content_keep_their_ownership() {
    let (source, mut input, evidence) = fixture();
    let st = &mut input.stories[0];
    st.text = "1. 😀\u{f061}".into();
    st.segments = vec![
        SourceSegment {
            kind: "generated_list_label",
            start_cp: 0,
            end_cp: 0,
            output_start: 0,
            output_end: 3,
        },
        SourceSegment {
            kind: "text",
            start_cp: 0,
            end_cp: 3,
            output_start: 3,
            output_end: 10,
        },
        SourceSegment {
            kind: "revision_deleted",
            start_cp: 3,
            end_cp: 4,
            output_start: 10,
            output_end: 10,
        },
    ];
    let out = apply(&source, &input, &evidence).unwrap();
    assert_eq!(out.stories[0].text, "1. 😀α");
    assert_eq!(out.stories[0].segments[0].end_cp, 0);
    assert_eq!(
        out.stories[0].segments.last().unwrap().kind,
        "revision_deleted"
    );
}
#[test]
fn stale_evidence_and_ambiguous_mapping_fail_without_partial_output() {
    let (source, mut input, mut evidence) = fixture();
    evidence.spans[0].source_code = 0xf062;
    assert!(apply(&source, &input, &evidence).is_err());
    evidence.spans[0].source_code = 0xf061;
    input.stories[0].segments[0].kind = "field_display_argument";
    assert!(apply(&source, &input, &evidence).is_err());
}
#[test]
fn malformed_intervals_and_exhausted_work_fail() {
    assert!(ordered([(1, 1)].into_iter()).is_err());
    assert!(ordered([(0, 4), (3, 5)].into_iter()).is_err());
    let mut budget = 2;
    assert!(charge(&mut budget, 3).is_err());
}
struct ContextFixture {
    source: Inspection,
    formatting: FormattingInspection,
    paragraphs: ParagraphInspection,
    styles: Stylesheet,
    fonts: FontTable,
    revisions: RevisionEvidence,
    hidden: HiddenEvidence,
}
impl ContextFixture {
    fn new() -> Self {
        use crate::office::doc::{
            formatting::CharacterSpan,
            formatting_inspection::PieceProperties,
            hidden_visibility::{HiddenProperty, HiddenSpan},
            paragraph_formatting::ParagraphSpan,
            styles::{Style, StyleProperties},
            symbols::Font,
            visibility::{RevisionProperties, RevisionSpan},
        };
        let (source, _, _) = fixture();
        let style = |index, kind, properties| Style {
            index,
            builtin_id: 0,
            kind,
            base_style: None,
            next_style: index,
            linked_style: None,
            revision_marked: false,
            name: "Synthetic".into(),
            raw_flags: 0,
            properties,
            raw_revision: vec![],
        };
        let mut styles: Vec<_> = (0..11).map(|_| None).collect();
        styles[0] = Some(style(
            0,
            1,
            vec![
                StyleProperties {
                    kind: "paragraph",
                    bytes: vec![0, 0],
                },
                StyleProperties {
                    kind: "character",
                    bytes: vec![0x73, 0x48, 9, 4],
                },
            ],
        ));
        styles[10] = Some(style(
            10,
            2,
            vec![StyleProperties {
                kind: "character",
                bytes: vec![],
            }],
        ));
        Self {
            source,
            formatting: FormattingInspection {
                schema_version: 1,
                status: "fixture",
                binary_version: 0x101,
                pieces: vec![PieceProperties {
                    start_cp: 0,
                    end_cp: 4,
                    file_offset: 0,
                    compressed: false,
                    raw_flags: 0,
                    raw_prm: 0,
                }],
                authored_story_end_cp: 4,
                trailing_nonstory_cp_range: None,
                raw_property_groups: vec![],
                character_spans: vec![CharacterSpan {
                    start_cp: 0,
                    end_cp: 4,
                    properties: vec![modifier(0x4a51, &[0, 0])],
                }],
                unresolved: ["fixture"; 3],
            },
            paragraphs: ParagraphInspection {
                schema_version: 1,
                status: "reference_expanded_endpoint_selected_paragraphs",
                binary_version: 0x101,
                spans: vec![ParagraphSpan {
                    start_cp: 0,
                    end_cp: 4,
                    style_index: Some(0),
                    raw_modifiers: vec![],
                }],
                excluded_nonstory_cp_range: None,
                unresolved: ["fixture"; 4],
            },
            styles: Stylesheet {
                schema_version: 1,
                status: "fixture",
                std_base_bytes: 10,
                raw_header: vec![],
                styles,
                unresolved: ["fixture"; 3],
            },
            fonts: FontTable {
                status: "fixture",
                fonts: vec![Font {
                    index: 0,
                    name: "Symbol".into(),
                    alternate_name: None,
                    charset: 2,
                    family: 0,
                    weight: 400,
                    panose: vec![],
                    signature: vec![],
                }],
            },
            revisions: RevisionEvidence {
                status: "fixture",
                basis: "fixture",
                hidden_text: "fixture",
                spans: vec![RevisionSpan {
                    start_cp: 0,
                    end_cp: 4,
                    source_span_index: 0,
                    piece_index: 0,
                    properties: RevisionProperties::default(),
                    evidence: vec![],
                    unresolved: vec![],
                }],
            },
            hidden: HiddenEvidence {
                status: "fixture",
                view: "fixture",
                spans: vec![HiddenSpan {
                    start_cp: 0,
                    end_cp: 4,
                    source_span_index: 0,
                    piece_index: 0,
                    paragraph_end_cp: Some(3),
                    paragraph_table_depth: Some(0),
                    table_baseline_basis: "source_paragraph_depth_zero",
                    property: HiddenProperty {
                        hidden: Some(false),
                        paragraph_style: Some(0),
                        character_style: 10,
                        unresolved: vec![],
                    },
                }],
            },
        }
    }
    fn context(&self) -> Context<'_> {
        Context {
            source: &self.source,
            embedded_fonts: Some(false),
            formatting: &self.formatting,
            paragraphs: &self.paragraphs,
            styles: &self.styles,
            fonts: &self.fonts,
            revisions: &self.revisions,
            hidden: &self.hidden,
        }
    }
    fn evidence(&self) -> OrdinarySymbols {
        let (_, input, _) = fixture();
        resolve(&input, &self.context()).unwrap()
    }
}
#[test]
fn source_context_resolves_default_hint_and_direct_non_ascii_font() {
    let f = ContextFixture::new();
    let e = f.evidence();
    assert_eq!(e.spans[0].unicode, Some('α'));
    assert_eq!(e.spans[0].font_index, Some(0));
    assert_eq!(e.spans[0].unresolved, None);
}
#[test]
fn table_unknown_visibility_revision_and_special_state_are_not_symbol_proofs() {
    let mut f = ContextFixture::new();
    f.hidden.spans[0].paragraph_table_depth = Some(1);
    assert!(f.evidence().spans[0].unicode.is_none());
    f.hidden.spans[0].paragraph_table_depth = Some(0);
    f.hidden.spans[0].property.hidden = None;
    assert!(f.evidence().spans[0].unicode.is_none());
    f.hidden.spans[0].property.hidden = Some(false);
    f.revisions.spans[0].properties.deleted = Some(true);
    assert!(f.evidence().spans[0].unicode.is_none());
    f.revisions.spans[0].properties.deleted = Some(false);
    f.revisions.spans[0].properties.special = Some(true);
    assert!(f.evidence().spans[0].unicode.is_none());
}
#[test]
fn piece_style_overrides_and_unrecognized_font_effects_stay_unresolved() {
    let mut f = ContextFixture::new();
    f.formatting.pieces[0].raw_prm = 2;
    assert!(f.evidence().spans[0].unicode.is_none());
    f.formatting.pieces[0].raw_prm = 0;
    f.styles.styles[10].as_mut().unwrap().properties[0].bytes = vec![0x51, 0x4a, 0, 0];
    assert!(f.evidence().spans[0].unicode.is_none());
    f.styles.styles[10].as_mut().unwrap().properties[0]
        .bytes
        .clear();
    f.formatting.character_spans[0]
        .properties
        .push(modifier(0x2a86, &[0]));
    assert!(f.evidence().spans[0].unicode.is_none());
}
#[test]
fn unknown_font_charset_linked_or_revision_style_retains_source_code() {
    let mut f = ContextFixture::new();
    f.fonts.fonts[0].charset = 0;
    assert!(f.evidence().spans[0].unicode.is_none());
    f.fonts.fonts[0].charset = 2;
    f.fonts.fonts[0].name = "Unknown".into();
    assert!(f.evidence().spans[0].unicode.is_none());
    f.fonts.fonts[0].name = "Symbol".into();
    f.styles.styles[0].as_mut().unwrap().linked_style = Some(10);
    assert!(f.evidence().spans[0].unicode.is_none());
    f.styles.styles[0].as_mut().unwrap().linked_style = None;
    f.styles.styles[0].as_mut().unwrap().revision_marked = true;
    assert!(f.evidence().spans[0].unicode.is_none());
}
#[test]
fn character_coverage_gaps_and_overlaps_do_not_resolve_symbols() {
    let mut f = ContextFixture::new();
    f.formatting.character_spans[0].end_cp = 2;
    assert!(f.evidence().spans[0].unicode.is_none());
    f.formatting
        .character_spans
        .push(crate::office::doc::formatting::CharacterSpan {
            start_cp: 1,
            end_cp: 4,
            properties: vec![],
        });
    let (_, input, _) = fixture();
    assert!(resolve(&input, &f.context()).is_err());
}
#[test]
fn palette_and_underline_use_distinct_operand_grammars() {
    for value in [5, 8, 14, 15, 16] {
        assert!(property(&mut State::default(), &modifier(0x2a42, &[value]), 1).unwrap());
    }
    assert!(!property(&mut State::default(), &modifier(0x2a42, &[20]), 1).unwrap());
    assert!(property(&mut State::default(), &modifier(0x2a3e, &[43]), 1).unwrap());
    assert!(!property(&mut State::default(), &modifier(0x0868, &[129]), 1).unwrap());
}
#[test]
fn present_or_unchecked_embedded_fonts_prevent_published_mapping() {
    let f = ContextFixture::new();
    let (_, input, _) = fixture();
    for value in [None, Some(true)] {
        let mut ctx = f.context();
        ctx.embedded_fonts = value;
        assert!(resolve(&input, &ctx).unwrap().spans[0].unicode.is_none());
    }
}
#[test]
fn stale_context_versions_coverage_and_source_indices_are_rejected() {
    let mut f = ContextFixture::new();
    let (_, input, _) = fixture();
    f.formatting.binary_version = 0xc1;
    assert!(resolve(&input, &f.context()).is_err());
    f.formatting.binary_version = f.source.binary_version;
    f.formatting.authored_story_end_cp = 5;
    assert!(resolve(&input, &f.context()).is_err());
    f.formatting.authored_story_end_cp = 4;
    f.hidden.spans[0].source_span_index = 1;
    assert!(f.evidence().spans[0].unicode.is_none());
    f.hidden.spans[0].source_span_index = 0;
    f.revisions.spans[0].piece_index = 1;
    assert!(f.evidence().spans[0].unicode.is_none());
}
