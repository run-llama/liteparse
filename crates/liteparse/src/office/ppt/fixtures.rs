//! Synthetic .ppt files: a minimal OLE compound container around hand-built
//! [MS-PPT] records. Shared by this crate's tests and, through the `fixtures`
//! feature, by crates that accept .ppt input.

pub const END: u32 = 0xFFFF_FFFE;
pub const FREE: u32 = 0xFFFF_FFFF;

pub fn rec(kind: u16, inst: u16, body: &[u8]) -> Vec<u8> {
    let mut out = (inst << 4).to_le_bytes().to_vec();
    out.extend(kind.to_le_bytes());
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

pub fn container(kind: u16, inst: u16, children: &[Vec<u8>]) -> Vec<u8> {
    let body = children.concat();
    let mut out = rec(kind, inst, &body);
    out[0] |= 0x0F;
    out
}

pub fn u32s(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

/// TextHeaderAtom (text type) followed by the text as TextCharsAtom (UTF-16) or,
/// with `bytes`, TextBytesAtom (one byte per character).
pub fn text(text_type: u32, body: &[u8], bytes: bool) -> Vec<Vec<u8>> {
    let kind = if bytes { 0x0FA8 } else { 0x0FA0 };
    vec![rec(0x0F9F, 0, &u32s(&[text_type])), rec(kind, 0, body)]
}

/// SmallRectStruct anchor: top, left, right, bottom as i16 master units.
pub fn client_anchor(top: i16, left: i16, right: i16, bottom: i16) -> Vec<u8> {
    let b: Vec<u8> = [top, left, right, bottom]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    rec(0xF010, 0, &b)
}

/// The patriarch group shape every drawing starts with.
pub fn patriarch() -> Vec<u8> {
    container(
        0xF004,
        0,
        &[rec(0xF009, 0, &[0; 16]), rec(0xF00A, 0, &u32s(&[0x400, 5]))],
    )
}

pub fn slide_container(shapes: &[Vec<u8>]) -> Vec<u8> {
    let mut tree = vec![patriarch()];
    tree.extend_from_slice(shapes);
    let drawing = container(
        0x040C,
        0,
        &[container(0xF002, 0, &[container(0xF003, 0, &tree)])],
    );
    container(0x03EE, 0, &[rec(0x03EF, 0, &[0; 24]), drawing])
}

/// A text shape: an anchor and a ClientTextbox with the given children.
pub fn text_shape(anchor: Vec<u8>, textbox: Vec<Vec<u8>>) -> Vec<u8> {
    container(
        0xF004,
        0,
        &[
            rec(0xF00A, 0, &u32s(&[0x401, 0xA00])),
            anchor,
            container(0xF00D, 0, &textbox),
        ],
    )
}

/// DocumentContainer: DocumentAtom with the slide size, and a SlideListWithText
/// whose entries are (persist id, texts) in presentation order.
pub fn document(size: (i32, i32), slides: &[(u32, Vec<Vec<u8>>)]) -> Vec<u8> {
    let mut atom = Vec::new();
    atom.extend(size.0.to_le_bytes());
    atom.extend(size.1.to_le_bytes());
    atom.resize(40, 0);
    let mut list = Vec::new();
    for (i, (persist, texts)) in slides.iter().enumerate() {
        list.push(rec(
            0x03F3,
            0,
            &u32s(&[*persist, 0, texts.len() as u32 / 2, 256 + i as u32, 0]),
        ));
        list.extend(texts.iter().cloned());
    }
    container(
        0x03E8,
        0,
        &[rec(0x03E9, 0, &atom), container(0x0FF0, 0, &list)],
    )
}

/// The PowerPoint Document stream under construction; `push` returns a record's offset.
#[derive(Default)]
pub struct Stream(pub Vec<u8>);

impl Stream {
    pub fn push(&mut self, record: Vec<u8>) -> u32 {
        let off = self.0.len() as u32;
        self.0.extend(record);
        off
    }
    /// PersistDirectoryAtom for `(id, offset)` pairs, one entry each.
    pub fn directory(&mut self, entries: &[(u32, u32)]) -> u32 {
        let body: Vec<u32> = entries
            .iter()
            .flat_map(|&(id, off)| [id | (1 << 20), off])
            .collect();
        self.push(rec(0x1772, 0, &u32s(&body)))
    }
    pub fn user_edit(&mut self, last_edit: u32, directory: u32, document: u32) -> u32 {
        let mut body = u32s(&[0, 0x0300_03F4, last_edit, directory, document, 10]);
        body.extend([1, 0, 0, 0]);
        self.push(rec(0x0FF5, 0, &body))
    }
}

pub fn current_user(edit: u32, token: u32) -> Vec<u8> {
    let mut body = u32s(&[0x14, token, edit]);
    body.resize(20, 0);
    rec(0x0FF6, 0, &body)
}

/// An OLE compound file (version 3, 512-byte sectors) holding `streams` at the root.
/// Streams are padded to 4096 bytes so they live in regular sectors.
pub fn cfb(streams: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut fat = vec![0xFFFF_FFFD, END];
    let mut data = Vec::new();
    let mut entries = Vec::new();
    for (name, bytes) in streams {
        let mut bytes = bytes.clone();
        bytes.resize(bytes.len().max(4096).next_multiple_of(512), 0);
        let start = fat.len() as u32;
        let n = bytes.len() / 512;
        for k in 0..n as u32 {
            fat.push(if k + 1 == n as u32 {
                END
            } else {
                start + k + 1
            });
        }
        entries.push((name.to_string(), 2u8, start, bytes.len() as u32));
        data.extend(bytes);
    }
    assert!(fat.len() <= 128, "test container needs one FAT sector");
    fat.resize(128, FREE);
    let dir_entry = |name: &str, kind: u8, right: u32, child: u32, start: u32, size: u32| {
        let mut e = vec![0u8; 128];
        let units = utf16(name);
        e[..units.len()].copy_from_slice(&units);
        e[64..66].copy_from_slice(&((units.len() + 2) as u16).to_le_bytes());
        e[66] = kind;
        e[68..72].copy_from_slice(&FREE.to_le_bytes());
        e[72..76].copy_from_slice(&right.to_le_bytes());
        e[76..80].copy_from_slice(&child.to_le_bytes());
        e[116..120].copy_from_slice(&start.to_le_bytes());
        e[120..124].copy_from_slice(&size.to_le_bytes());
        e
    };
    let mut dir = dir_entry("Root Entry", 5, FREE, 1, END, 0);
    for (i, (name, kind, start, size)) in entries.iter().enumerate() {
        let right = if i + 1 < entries.len() {
            i as u32 + 2
        } else {
            FREE
        };
        dir.extend(dir_entry(name, *kind, right, FREE, *start, *size));
    }
    dir.resize(512, 0);
    let mut header = vec![0u8; 512];
    header[..8].copy_from_slice(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
    let fields: [(usize, u32); 9] = [
        (44, 1),
        (48, 1),
        (56, 4096),
        (60, END),
        (64, 0),
        (68, END),
        (72, 0),
        (76, 0),
        (80, FREE),
    ];
    for (off, v) in fields {
        header[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    header[24..32].copy_from_slice(&[0x3E, 0, 3, 0, 0xFE, 0xFF, 9, 0]);
    header[32..34].copy_from_slice(&6u16.to_le_bytes());
    for off in (84..512).step_by(4) {
        header[off..off + 4].copy_from_slice(&FREE.to_le_bytes());
    }
    let mut out = header;
    out.extend(u32s(&fat));
    out.extend(dir);
    out.extend(data);
    out
}

/// A one-edit file: document container (persist id 1) and the given slides.
/// Slide-list text records and drawing shapes of one slide.
pub type SlideParts = (Vec<Vec<u8>>, Vec<Vec<u8>>);

pub fn simple_ppt(size: (i32, i32), slides: &[SlideParts]) -> Vec<u8> {
    let mut s = Stream::default();
    let list: Vec<_> = (0..slides.len())
        .map(|i| (i as u32 + 2, slides[i].0.clone()))
        .collect();
    let doc = s.push(document(size, &list));
    let mut dir = vec![(1, doc)];
    for (i, (_, shapes)) in slides.iter().enumerate() {
        dir.push((i as u32 + 2, s.push(slide_container(shapes))));
    }
    let d = s.directory(&dir);
    let edit = s.user_edit(0, d, 1);
    cfb(&[
        ("PowerPoint Document", s.0),
        ("Current User", current_user(edit, 0xE391_C05F)),
    ])
}
