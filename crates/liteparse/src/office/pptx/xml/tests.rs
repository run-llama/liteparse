use super::*;

const SAMPLE: &str = r#"<q:sld xmlns:q="http://schemas.openxmlformats.org/presentationml/2006/main"
  xmlns:d="http://schemas.openxmlformats.org/drawingml/2006/main"
  xmlns:rel="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
  xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
  <q:cSld><q:spTree>
    <mc:AlternateContent><mc:Choice Requires="a14"><q:sp id="choice"/></mc:Choice>
      <mc:Fallback><q:pic><d:blip rel:embed="rId2"/></q:pic><q:sp/></mc:Fallback></mc:AlternateContent>
    <q:sp><d:t>Hello</d:t><d:t> world</d:t></q:sp>
  </q:spTree></q:cSld></q:sld>"#;

#[test]
fn canonicalizes_prefixes_and_resolves_alternate_content() {
    let root = parse(SAMPLE).unwrap();
    assert_eq!(root.name, "p:sld");
    let tree = root.path(&["p:cSld", "p:spTree"]).unwrap();
    let names: Vec<&str> = tree.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["p:pic", "p:sp", "p:sp"]);
    let blip = tree.children[0].child("a:blip").unwrap();
    assert_eq!(blip.attr("r:embed"), Some("rId2"));
    assert_eq!(tree.children[2].all_text(), "Hello world");
}

#[test]
fn parses_loose_integers() {
    assert_eq!(parse_i64("12700"), Some(12700));
    assert_eq!(parse_i64("12700.6"), Some(12701));
    assert_eq!(parse_i64("50%"), Some(50000));
    assert_eq!(parse_i64("x"), None);
}

#[test]
fn root_attr_agrees_with_the_parsed_root() {
    let p = "http://schemas.openxmlformats.org/presentationml/2006/main";
    let cases = [
        format!(r#"<p:sld xmlns:p="{p}" show="0"><p:cSld/></p:sld>"#),
        format!(r#"<p:sld xmlns:p="{p}" showMasterSp="0"/>"#),
        format!(r#"<p:sld xmlns:p="{p}" p:show = '0'/>"#),
        format!(r#"<p:sld xmlns:p="{p}"><p:sp show="0"/></p:sld>"#),
        format!(r#"<p:sld xmlns:p="{p}" show="0">"#),
        format!(r#"<p:sld xmlns:p="{p}"/>"#),
        SAMPLE.to_string(),
    ];
    for c in &cases {
        let want = parse(c)
            .ok()
            .and_then(|r| r.attr("show").map(str::to_string));
        assert_eq!(root_attr(c, "show"), want, "{c}");
    }
    assert_eq!(root_attr(&cases[0], "show").as_deref(), Some("0"));
}
