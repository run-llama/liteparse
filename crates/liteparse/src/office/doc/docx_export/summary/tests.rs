use super::*;

/// A section from (id, typed value bytes) properties.
fn section(props: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let head = 8 + props.len() * 8;
    let mut table = Vec::new();
    let mut body = Vec::new();
    for (pid, v) in props {
        table.extend(pid.to_le_bytes());
        table.extend(((head + body.len()) as u32).to_le_bytes());
        body.extend(v);
        while body.len() % 4 != 0 {
            body.push(0);
        }
    }
    let mut out = ((head + body.len()) as u32).to_le_bytes().to_vec();
    out.extend((props.len() as u32).to_le_bytes());
    out.extend(table);
    out.extend(body);
    out
}

/// A property-set stream holding `secs`.
fn stream(secs: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0xfe, 0xff, 0, 0, 0, 0, 0, 0];
    out.extend([0; 16]);
    out.extend((secs.len() as u32).to_le_bytes());
    let mut at = 28 + secs.len() * 20;
    for s in secs {
        out.extend([0; 16]);
        out.extend((at as u32).to_le_bytes());
        at += s.len();
    }
    for s in secs {
        out.extend(s);
    }
    out
}

fn typed(vt: u16, payload: &[u8]) -> Vec<u8> {
    let mut v = (vt as u32).to_le_bytes().to_vec();
    v.extend(payload);
    v
}

fn lpstr(s: &[u8]) -> Vec<u8> {
    let mut p = ((s.len() + 1) as u32).to_le_bytes().to_vec();
    p.extend(s);
    p.push(0);
    typed(0x1e, &p)
}

#[test]
fn summary_strings_decode_in_their_code_page() {
    let mut wide = 3u32.to_le_bytes().to_vec();
    wide.extend([b'A', 0, b'n', 0, 0, 0]);
    let s = stream(&[section(&[
        (1, typed(0x02, &1252u16.to_le_bytes())),
        (2, lpstr(b"Plan \x93Q1\x94")),
        (4, typed(0x1f, &wide)),
        (9, typed(0x40, &[0; 8])),
    ])]);
    let sec = sections(&s)[0];
    assert_eq!(
        values(sec),
        [
            (2, "Plan \u{201c}Q1\u{201d}".to_string()),
            (4, "An".to_string())
        ]
    );
}

#[test]
fn user_defined_values_take_their_dictionary_names() {
    let mut dict = 2u32.to_le_bytes().to_vec();
    for (pid, name) in [(2u32, &b"Doc Number"[..]), (3, b"Rev")] {
        dict.extend(pid.to_le_bytes());
        dict.extend(((name.len() + 1) as u32).to_le_bytes());
        dict.extend(name);
        dict.push(0);
    }
    let sec = section(&[
        (0, dict),
        (1, typed(0x02, &1252u16.to_le_bytes())),
        (2, lpstr(b"N-42")),
        (3, typed(0x03, &7i32.to_le_bytes())),
    ]);
    let names = dictionary(&sec);
    assert_eq!(
        names,
        [(2, "Doc Number".to_string()), (3, "Rev".to_string())]
    );
    assert_eq!(
        values(&sec),
        [(2, "N-42".to_string()), (3, "7".to_string())]
    );
}

#[test]
fn core_parts_escape_values_and_list_their_relationships() {
    let props = Properties {
        core: vec![("dc:title", "R&D <draft>".into())],
        app: vec![],
        custom: vec![("Doc Number".into(), "N-42".into())],
    };
    let (parts, rels) = parts(&props);
    let core = String::from_utf8(parts[0].bytes.clone()).unwrap();
    assert!(core.contains("<dc:title>R&amp;D &lt;draft&gt;</dc:title>"));
    let custom = String::from_utf8(parts[1].bytes.clone()).unwrap();
    assert!(custom.contains("pid=\"2\" name=\"Doc Number\"><vt:lpwstr>N-42</vt:lpwstr>"));
    assert_eq!(
        rels.iter().map(|r| r.1).collect::<Vec<_>>(),
        ["docProps/core.xml", "docProps/custom.xml"]
    );
}
