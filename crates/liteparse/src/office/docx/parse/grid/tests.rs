use super::*;

fn validate(budget: &mut Budget, properties: &str, cells: &str) -> Option<Vec<RowGrid>> {
    let table = crate::office::pptx::xml::parse_word(&format!(
        r#"<w:tbl xmlns:w="{}"><w:tr><w:trPr>{properties}</w:trPr>{cells}</w:tr></w:tbl>"#,
        crate::office::docx::testutil::W
    ))
    .unwrap();
    let mut rows = Vec::new();
    collect_named(&table.children, "w:tr", &mut rows);
    budget.table(&table, &rows)
}

fn cell(span: &str) -> String {
    format!(r#"<w:tc><w:tcPr><w:gridSpan w:val="{span}"/></w:tcPr><w:p/></w:tc>"#)
}

#[test]
fn numeric_overflow_and_cumulative_extent_fail_without_truncation() {
    for span in [
        "9223372036854775808",
        "18446744073709551616",
        "99999999999999999999999999999999999999",
    ] {
        let mut budget = Budget::new(100, 10000);
        assert!(validate(&mut budget, "", &cell(span)).is_none());
        assert!(matches!(budget.check(), Err(DocError::LimitExceeded(_))));
    }
    for properties in [
        r#"<w:gridBefore w:val="100"/>"#,
        r#"<w:gridAfter w:val="100"/>"#,
    ] {
        assert!(validate(&mut Budget::new(100, 10000), properties, &cell("1")).is_none());
    }
    assert!(
        validate(
            &mut Budget::new(100, 10000),
            "",
            &(cell("70") + &cell("70"))
        )
        .is_none()
    );
}

#[test]
fn shared_budget_retains_valid_spans_and_normalizes_nonpositive_values() {
    let mut budget = Budget::new(100, 10000);
    let rows = validate(&mut budget, "", &(cell("70") + &cell("6"))).unwrap();
    assert_eq!(rows[0].spans, [70, 6]);
    for value in ["0", "-2", "nonnumeric", ""] {
        assert_eq!(
            validate(&mut budget, "", &cell(value)).unwrap()[0].spans,
            [1]
        );
    }
    let mut budget = Budget::new(100, 600);
    assert!(validate(&mut budget, "", &cell("76")).is_some());
    assert!(validate(&mut budget, "", &cell("76")).is_none());
    assert!(budget.check().is_err());
}
