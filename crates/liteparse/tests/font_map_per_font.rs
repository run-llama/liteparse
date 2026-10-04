//! A font's broken code-to-character map is judged per font, not per item.
//!
//! The PDFs here are synthetic: an embedded TrueType font, generated below, whose
//! glyph for character `c` is a rectangle `5 * c` units wide, so a test resolver
//! can name every glyph from its outline alone. The `/ToUnicode` map either tells
//! the truth or sends every code to the character 8 below it. Under that shifted
//! map the space (0x20) lands on a control character (0x18), so only items that
//! hold a space give the map away; the other items decode to wrong printable text.
use liteparse::{GlyphResolver, RawTextItem, extract_raw_text_items};
use pdfium::Library;

/// Characters the generated font draws, one glyph each.
const CHARS: std::ops::RangeInclusive<u8> = 0x20..=0x7E;

/// Names a glyph from its outline width: the generated glyph for `c` is
/// `5 * c` font units wide, and pdfium reports the outline in ems. `known`
/// limits which characters it recognises, as a real outline database knows
/// only some fonts' glyphs.
struct WidthResolver {
    known: fn(char) -> bool,
}

impl GlyphResolver for WidthResolver {
    fn resolve(&self, segments: &[(i32, f32, f32)]) -> Option<String> {
        let (min, max) = segments
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &(_, x, _)| {
                (lo.min(x), hi.max(x))
            });
        let units = ((max - min) * 1000.0).round() as u32;
        let c = char::from_u32(units / 5).filter(|_| units.is_multiple_of(5))?;
        (self.known)(c).then(|| c.to_string())
    }
}

fn every_char(_: char) -> bool {
    true
}

fn digits_only(c: char) -> bool {
    c.is_ascii_digit()
}

/// A minimal TrueType program: `.notdef` plus one rectangle per char of
/// [`CHARS`], reached through a (1,0) byte-encoding cmap.
fn truetype() -> Vec<u8> {
    let glyph_count = 1 + CHARS.count() as u16;
    let mut glyf = Vec::new();
    let mut loca = vec![0u32, 0]; // .notdef has no outline
    let mut hmtx = vec![0u8; 4]; // .notdef: zero advance, zero side bearing
    let mut cmap_ids = [0u8; 256];
    for (i, c) in CHARS.enumerate() {
        let width = 5 * i16::from(c);
        cmap_ids[usize::from(c)] = (i + 1) as u8;
        // One contour of four on-curve points; coordinates are deltas.
        for v in [1i16, 0, 0, width, 700] {
            glyf.extend_from_slice(&v.to_be_bytes()); // contours, xMin, yMin, xMax, yMax
        }
        glyf.extend_from_slice(&3u16.to_be_bytes()); // last point of the contour
        glyf.extend_from_slice(&0u16.to_be_bytes()); // no instructions
        glyf.extend_from_slice(&[1, 1, 1, 1]); // on-curve, 16-bit x and y deltas
        for dx in [0i16, width, 0, -width] {
            glyf.extend_from_slice(&dx.to_be_bytes());
        }
        for dy in [0i16, 0, 700, 0] {
            glyf.extend_from_slice(&dy.to_be_bytes());
        }
        loca.push(glyf.len() as u32);
        hmtx.extend_from_slice(&(width as u16 + 50).to_be_bytes());
        hmtx.extend_from_slice(&0i16.to_be_bytes());
    }
    let loca: Vec<u8> = loca.iter().flat_map(|o| o.to_be_bytes()).collect();

    let mut head = Vec::new();
    head.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // version
    head.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // fontRevision
    head.extend_from_slice(&0u32.to_be_bytes()); // checkSumAdjustment
    head.extend_from_slice(&0x5F0F_3CF5u32.to_be_bytes()); // magicNumber
    head.extend_from_slice(&0u16.to_be_bytes()); // flags
    head.extend_from_slice(&1000u16.to_be_bytes()); // unitsPerEm
    head.extend_from_slice(&[0; 16]); // created, modified
    for v in [0i16, 0, 5 * 0x7E, 700] {
        head.extend_from_slice(&v.to_be_bytes()); // font bounding box
    }
    for v in [0u16, 8, 2, 1, 0] {
        // macStyle, lowestRecPPEM, fontDirectionHint, long loca, glyphDataFormat
        head.extend_from_slice(&v.to_be_bytes());
    }

    let mut hhea = Vec::new();
    hhea.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    for v in [
        800i16,
        -200,
        0,
        5 * 0x7E + 50,
        0,
        0,
        5 * 0x7E,
        1,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ] {
        hhea.extend_from_slice(&v.to_be_bytes());
    }
    hhea.extend_from_slice(&glyph_count.to_be_bytes()); // numberOfHMetrics

    let mut maxp = Vec::new();
    maxp.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    for v in [glyph_count, 4, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        maxp.extend_from_slice(&v.to_be_bytes());
    }

    let mut cmap = Vec::new();
    for v in [0u16, 1, 1, 0] {
        cmap.extend_from_slice(&v.to_be_bytes()); // version, 1 subtable, platform 1, encoding 0
    }
    cmap.extend_from_slice(&12u32.to_be_bytes()); // subtable offset
    for v in [0u16, 262, 0] {
        cmap.extend_from_slice(&v.to_be_bytes()); // format 0, length, language
    }
    cmap.extend_from_slice(&cmap_ids);

    let mut tables = [
        (*b"cmap", cmap),
        (*b"glyf", glyf),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"maxp", maxp),
    ];
    tables.sort_by_key(|(tag, _)| *tag);
    let mut font = Vec::new();
    font.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    for v in [tables.len() as u16, 64, 2, 48] {
        font.extend_from_slice(&v.to_be_bytes()); // numTables, searchRange, entrySelector, rangeShift
    }
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in &tables {
        let checksum = data
            .chunks(4)
            .map(|c| {
                let mut word = [0u8; 4];
                word[..c.len()].copy_from_slice(c);
                u32::from_be_bytes(word)
            })
            .fold(0u32, u32::wrapping_add);
        font.extend_from_slice(tag);
        font.extend_from_slice(&checksum.to_be_bytes());
        font.extend_from_slice(&(offset as u32).to_be_bytes());
        font.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut padded = data.clone();
        padded.resize(data.len().div_ceil(4) * 4, 0);
        offset += padded.len();
        body.extend_from_slice(&padded);
    }
    font.extend_from_slice(&body);
    font
}

/// A `/ToUnicode` CMap sending each code of [`CHARS`] to `map(code)`.
fn to_unicode(map: fn(u8) -> u8) -> String {
    let entries: String = CHARS
        .map(|c| format!("<{c:02X}> <{:04X}>\n", map(c)))
        .collect();
    format!(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
         /CMapName /Test def /CMapType 2 def\n\
         1 begincodespacerange <00> <FF> endcodespacerange\n\
         {} beginbfchar\n{entries}endbfchar\n\
         endcmap CMapName currentdict /CMap defineresource pop end end",
        CHARS.count()
    )
}

/// The broken map: every code decodes to the character 8 below it.
fn shifted(c: u8) -> u8 {
    c - 8
}

/// A sound map, except that `~` decodes to a control character.
fn sound_but_tilde(c: u8) -> u8 {
    if c == b'~' { 0x01 } else { c }
}

/// One page drawing each of `lines` as its own text object with the generated
/// font, whose `/ToUnicode` map decodes code `c` to `map(c)`.
fn pdf(map: fn(u8) -> u8, lines: &[&str]) -> Vec<u8> {
    let font = truetype();
    let widths: Vec<String> = CHARS.map(|c| (5 * u32::from(c) + 50).to_string()).collect();
    let content: String = lines
        .iter()
        .enumerate()
        .map(|(i, line)| format!("BT /F1 10 Tf 50 {} Td ({line}) Tj ET\n", 700 - 30 * i))
        .collect();
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        format!(
            "<< /Type /Font /Subtype /TrueType /BaseFont /Glyphs /FirstChar 32 /LastChar 126 /Widths [{}] /FontDescriptor 6 0 R /ToUnicode 8 0 R >>",
            widths.join(" ")
        )
        .into_bytes(),
        stream(content.as_bytes()),
        b"<< /Type /FontDescriptor /FontName /Glyphs /Flags 4 /FontBBox [0 -200 680 800] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 /FontFile2 7 0 R >>".to_vec(),
        stream(&font),
        stream(to_unicode(map).as_bytes()),
    ];
    let mut data = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(data.len());
        data.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        data.extend_from_slice(object);
        data.extend_from_slice(b"\nendobj\n");
    }
    let xref = data.len();
    data.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        data.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    data.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    data
}

fn stream(data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

fn extract(bytes: &[u8], resolver: Option<&dyn GlyphResolver>) -> Vec<RawTextItem> {
    let lib = Library::init();
    let doc = lib.load_document_from_bytes(bytes, None).unwrap();
    let page = doc.page(0).unwrap();
    let text = page.text().unwrap();
    extract_raw_text_items(&page, &text, &page.view_box().unwrap(), resolver)
}

/// `(text, font_is_buggy)` of each non-blank item, text trimmed.
fn texts(items: &[RawTextItem]) -> Vec<(String, bool)> {
    items
        .iter()
        .filter(|item| !item.text.trim().is_empty())
        .map(|item| (item.text.trim().to_string(), item.font_is_buggy))
        .collect()
}

#[test]
fn shifted_map_is_recovered_in_every_item_of_the_font() {
    let bytes = pdf(shifted, &["Net 1,250", "2024", "-375", "Total"]);
    let items = extract(&bytes, Some(&WidthResolver { known: every_char }));
    // Only "Net 1,250" holds a space, the code that decodes to a control
    // character; the other items of the font are recovered all the same.
    assert_eq!(
        texts(&items),
        [
            ("Net 1,250".to_string(), true),
            ("2024".to_string(), true),
            ("-375".to_string(), true),
            ("Total".to_string(), true),
        ]
    );
}

#[test]
fn without_a_resolver_the_shifted_items_keep_the_map_text() {
    let bytes = pdf(shifted, &["Net 1,250", "2024"]);
    let items = extract(&bytes, None);
    // No outline evidence, no verdict beyond the item's own control decode.
    let unflagged: Vec<_> = texts(&items)
        .into_iter()
        .filter(|(_, buggy)| !buggy)
        .collect();
    assert_eq!(unflagged, [("*(*,".to_string(), false)]);
}

#[test]
fn sound_map_keeps_its_text_when_recovery_disagrees_on_nothing() {
    // A truthful map with one item decoding to a control character: the
    // resolver agrees with the map on every code it knows, so the other items
    // are not re-decoded. The resolver knows digits only; re-decoding "Total"
    // would blank it.
    let bytes = pdf(sound_but_tilde, &["Total", "2024", "1,250", "A~"]);
    let items = extract(&bytes, Some(&WidthResolver { known: digits_only }));
    let texts = texts(&items);
    assert!(texts.contains(&("Total".to_string(), false)), "{texts:?}");
    assert!(texts.contains(&("2024".to_string(), false)), "{texts:?}");
    assert!(texts.contains(&("1,250".to_string(), false)), "{texts:?}");
}

#[test]
fn shifted_map_with_too_little_evidence_keeps_the_per_item_verdict() {
    // Two distinct printable codes only, under the three needed to call a map
    // wrong: the unflagged item keeps the map's characters.
    let bytes = pdf(shifted, &["12 21", "12"]);
    let items = extract(&bytes, Some(&WidthResolver { known: every_char }));
    let texts = texts(&items);
    assert!(texts.contains(&(")*".to_string(), false)), "{texts:?}");
}
