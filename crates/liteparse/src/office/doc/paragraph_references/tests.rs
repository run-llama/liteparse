use super::*;

fn source(raw: Vec<u8>) -> ParagraphInspection {
    ParagraphInspection {
        schema_version: 1,
        status: "raw",
        binary_version: 0x10c,
        excluded_nonstory_cp_range: None,
        spans: vec![ParagraphSpan {
            start_cp: 3,
            end_cp: 9,
            style_index: Some(0),
            raw_modifiers: raw,
        }],
        unresolved: [""; 4],
    }
}
fn reference(sprm: u16, offset: u32) -> Vec<u8> {
    [
        sprm.to_le_bytes().as_slice(),
        offset.to_le_bytes().as_slice(),
    ]
    .concat()
}
fn props() -> Vec<u8> {
    vec![0x16, 0x24, 1, 0x17, 0x24, 1, 0x49, 0x66, 1, 0, 0, 0]
}

#[test]
fn replacement_discards_original_tail_and_preserves_cp_style() {
    let mut raw = vec![0x0b, 0x46, 2, 0];
    raw.extend(reference(0x646b, 10));
    raw.extend([0xff]); // The replacement terminates the original property array.
    let input = source(raw);
    let output = resolve(None, &input, |offset| {
        assert_eq!(offset, 10);
        Ok(props())
    })
    .unwrap();
    assert_eq!(
        output.spans[0].raw_modifiers,
        [vec![0x0b, 0x46, 2, 0], props()].concat()
    );
    assert_eq!(
        (
            output.spans[0].start_cp,
            output.spans[0].end_cp,
            output.spans[0].style_index
        ),
        (3, 9, Some(0))
    );
}
#[test]
fn huge_only_applies_at_start_and_requires_zero_style() {
    let mut raw = vec![0x0b, 0x46, 2, 0];
    raw.extend(reference(0x6646, 999));
    assert_eq!(
        resolve(None, &source(raw), |_| panic!("ignored reference"))
            .unwrap()
            .spans[0]
            .raw_modifiers,
        vec![0x0b, 0x46, 2, 0]
    );
    let mut input = source(reference(0x6646, 0));
    input.spans[0].style_index = Some(1);
    assert!(resolve(None, &input, |_| Ok(props())).is_err());
    input.spans[0].style_index = Some(0);
    assert_eq!(
        resolve(None, &input, |_| Ok(props())).unwrap().spans[0].raw_modifiers,
        props()
    );
}
#[test]
fn nested_references_terminate_and_cycles_rejected() {
    let input = source(reference(0x646b, 0));
    let output = resolve(None, &input, |offset| {
        if offset == 0 {
            Ok([reference(0x646b, 20), vec![0; 4]].concat())
        } else {
            Ok(props())
        }
    })
    .unwrap();
    assert_eq!(output.spans[0].raw_modifiers, props());
    assert!(
        resolve(None, &input, |_| Ok(
            [reference(0x646b, 0), vec![0; 4]].concat()
        ))
        .is_err()
    );
    assert!(
        resolve(None, &input, |offset| Ok([
            reference(0x646b, (offset + 1) as u32),
            vec![0; 4]
        ]
        .concat()))
        .is_err()
    );
}
#[test]
fn lengths_and_variable_operands_are_bounded() {
    let input = source(reference(0x646b, 0));
    assert!(resolve(None, &input, |_| Ok(vec![0; 9])).is_err());
    assert!(resolve(None, &input, |_| Ok(vec![0; 0x3fa3])).is_err());
    assert_eq!(operand_length(0xd608, &[3, 0, 0, 0]).unwrap(), 4);
    assert!(operand_length(0xd608, &[0, 0]).is_err());
    assert!(operand_length(0xc615, &[255]).is_err());
    assert!(resolve(None, &source(vec![0x08, 0xd6, 5, 0, 1]), |_| panic!()).is_err());
}
#[test]
fn aggregate_work_limit_bounds_repeated_references() {
    let mut input = source(reference(0x646b, 0));
    for cp in 1..600 {
        input.spans.push(ParagraphSpan {
            start_cp: cp,
            end_cp: cp + 1,
            style_index: Some(0),
            raw_modifiers: reference(0x646b, 0),
        });
    }
    let record = [0x16, 0x24, 1].repeat(5000);
    assert!(resolve(None, &input, |_| Ok(record.clone())).is_err());
}

#[test]
fn nonstory_modifier_tail_is_retained_raw_and_excluded_from_expansion() {
    let mut input = source(props());
    input.binary_version = 0x112;
    input.spans[0].start_cp = 0;
    input.spans[0].end_cp = 10;
    input.spans.push(ParagraphSpan {
        start_cp: 10,
        end_cp: 11,
        style_index: Some(0),
        raw_modifiers: vec![0x6b, 0x64, 0xff],
    });
    let result =
        inspect_paragraph_references(&crate::office::doc::tests::fixture(), &input).unwrap();
    assert_eq!(result.excluded_nonstory_cp_range, Some([10, 11]));
    assert_eq!(result.spans.len(), 1);
    assert_eq!(result.spans[0].raw_modifiers, props());
    assert_eq!(input.spans[1].raw_modifiers, [0x6b, 0x64, 0xff]);
    assert_eq!(input.excluded_nonstory_cp_range, None);
}

#[test]
fn crossing_span_clips_cp_range_but_validates_all_authored_modifiers() {
    let mut input = source(props());
    input.spans[0].start_cp = 0;
    input.spans[0].end_cp = 5;
    let result = resolve(Some(4), &input, |_| panic!("no reference")).unwrap();
    assert_eq!(result.spans[0].end_cp, 4);
    assert_eq!(result.spans[0].raw_modifiers, props());
    input.spans[0].raw_modifiers.push(0);
    assert!(resolve(Some(4), &input, |_| panic!("no reference")).is_err());
}

#[test]
fn authored_boundary_and_range_identity_are_strict() {
    let mut input = source(vec![0]);
    input.spans[0].start_cp = 0;
    input.spans[0].end_cp = 4;
    assert!(resolve(Some(4), &input, |_| panic!("no reference")).is_err());
    input.spans[0].raw_modifiers = props();
    assert!(
        resolve(Some(4), &input, |_| panic!("no reference"))
            .unwrap()
            .excluded_nonstory_cp_range
            .is_none()
    );
    assert!(resolve(Some(2), &input, |_| panic!("no reference")).is_err());
    assert!(resolve(Some(5), &input, |_| panic!("no reference")).is_err());
    input.spans[0].start_cp = 1;
    assert!(resolve(Some(4), &input, |_| panic!("no reference")).is_err());
}
