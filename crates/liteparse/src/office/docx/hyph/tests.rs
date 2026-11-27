use super::points;

fn show(w: &str, lang: &str) -> String {
    let pts = points(w, lang);
    let mut s = String::new();
    for (i, c) in w.chars().enumerate() {
        if pts.contains(&i) {
            s.push('-');
        }
        s.push(c);
    }
    s
}

#[test]
fn english_points() {
    assert_eq!(show("hyphenation", "en-US"), "hy-phen-ation");
    assert_eq!(show("implement", "en-US"), "im-ple-ment");
    assert_eq!(show("hyphenation", "en-GB"), "hy-phen-a-tion");
    assert_eq!(show("Zimbabwe", "en-AU"), "Zim-babwe");
    assert!(points("hyphenation", "fr-FR").is_empty());
}

#[test]
fn language_region_survives_script_case_and_extensions() {
    let british = points("hyphenation", "en-GB");
    let american = points("hyphenation", "en-US");
    assert_ne!(british, american);
    for tag in [
        "EN-gb",
        "en-Latn-GB",
        "en-GB-u-ca-gregory",
        "en-Latn-GB-x-source",
    ] {
        assert_eq!(points("hyphenation", tag), british, "{tag}");
    }
    for tag in ["en", "en-Latn-US", "en-x-gb", "en-u-rg-gbzzzz"] {
        assert_eq!(points("hyphenation", tag), american, "{tag}");
    }
    assert!(points("hyphenation", "fr-Latn-GB").is_empty());
}
