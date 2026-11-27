use super::*;
use crate::office::pptx::xml::parse;

fn lin(body: &str) -> String {
    let xml = format!(
        r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math">{body}</m:oMath>"#
    );
    linearize(&parse(&xml).unwrap())
}

#[test]
fn linearizes_common_constructs() {
    assert_eq!(
        lin("<m:r><m:t>y</m:t></m:r><m:r><m:t>=13</m:t></m:r>"),
        "y=13"
    );
    assert_eq!(
        lin(
            "<m:f><m:num><m:r><m:t>a+b</m:t></m:r></m:num><m:den><m:r><m:t>c</m:t></m:r></m:den></m:f>"
        ),
        "(a+b)/c"
    );
    assert_eq!(
        lin(
            "<m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup>"
        ),
        "x^2"
    );
    assert_eq!(
        lin("<m:rad><m:radPr/><m:deg/><m:e><m:r><m:t>x+1</m:t></m:r></m:e></m:rad>"),
        "√(x+1)"
    );
    assert_eq!(
        lin("<m:d><m:dPr/><m:e><m:r><m:t>4,7</m:t></m:r></m:e></m:d>"),
        "(4,7)"
    );
    assert_eq!(
        lin(
            "<m:nary><m:naryPr><m:chr m:val=\"∑\"/></m:naryPr><m:sub><m:r><m:t>i=1</m:t></m:r></m:sub><m:sup><m:r><m:t>n</m:t></m:r></m:sup><m:e><m:r><m:t>i</m:t></m:r></m:e></m:nary>"
        ),
        "∑_(i=1)^n▒i"
    );
}
