use super::ctf::{self, Be};
use super::mtx::{self, Ahuff};
use super::*;
use rustybuzz::ttf_parser;

/// A minimal TrueType font with a right triangle as glyph 1 (mapped from 'A').
fn tiny_ttf(glyf: &[u8], loca: &[u8], num_glyphs: u16) -> Vec<u8> {
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[50..52].copy_from_slice(&1u16.to_be_bytes()); // long loca
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[34..36].copy_from_slice(&num_glyphs.to_be_bytes());
    let mut hmtx = Vec::new();
    for _ in 0..num_glyphs {
        hmtx.extend_from_slice(&500u16.to_be_bytes());
        hmtx.extend_from_slice(&0u16.to_be_bytes());
    }
    // cmap format 4: 'A' (0x41) → glyph 1.
    let mut cmap = vec![0, 0, 0, 1, 0, 3, 0, 1, 0, 0, 0, 12];
    let seg: &[u16] = &[
        4,
        32,
        0,
        4,
        4,
        1,
        0,
        0x41,
        0xFFFF,
        0,
        0x41,
        0xFFFF,
        (1i16 - 0x41) as u16,
        1,
        0,
        0,
    ];
    for v in seg {
        cmap.extend_from_slice(&v.to_be_bytes());
    }
    let tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"cmap", cmap),
        (b"glyf", glyf.to_vec()),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca.to_vec()),
        (b"maxp", maxp),
    ];
    let mut out = Vec::new();
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&[0, 64, 0, 2, 0, 48]);
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

/// A version 0x00020002 EOT header around `font_data`.
fn eot_wrap(font_data: &[u8], flags: u32) -> Vec<u8> {
    let mut h = Vec::new();
    let name = |s: &str| {
        let mut v = vec![0, 0];
        let units: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        v.extend_from_slice(&(units.len() as u16).to_le_bytes());
        v.extend_from_slice(&units);
        v
    };
    h.extend_from_slice(&0u32.to_le_bytes()); // EOTSize, patched below
    h.extend_from_slice(&(font_data.len() as u32).to_le_bytes());
    h.extend_from_slice(&0x0002_0002u32.to_le_bytes());
    h.extend_from_slice(&flags.to_le_bytes());
    h.extend_from_slice(&[0; 10]);
    h.extend_from_slice(&[1, 1]); // charset, italic
    h.extend_from_slice(&700u32.to_le_bytes());
    h.extend_from_slice(&0u16.to_le_bytes());
    h.extend_from_slice(&0x504Cu16.to_le_bytes());
    h.extend_from_slice(&[0; 44]);
    for s in [
        "Sniglet",
        "Bold Italic",
        "Version 1",
        "Sniglet Bold Italic",
        "",
    ] {
        h.extend_from_slice(&name(s));
    }
    h.extend_from_slice(&[0; 12]); // checksum, EUDC code page, padding, signature size
    h.extend_from_slice(&[0; 8]); // EUDC flags, EUDC size
    h.extend_from_slice(font_data);
    let n = h.len() as u32;
    h[0..4].copy_from_slice(&n.to_le_bytes());
    h
}

fn triangle_glyf() -> (Vec<u8>, Vec<u8>) {
    // Glyph 0 empty; glyph 1: one contour (0,0) (500,0) (0,700), no instructions.
    let mut g = Vec::new();
    for v in [1i16, 0, 0, 500, 700, 2, 0] {
        g.extend_from_slice(&v.to_be_bytes());
    }
    // Points as words: flags with no short/same bits.
    g.extend_from_slice(&[0x01, 0x01, 0x01]);
    for x in [0i16, 500, -500] {
        g.extend_from_slice(&x.to_be_bytes());
    }
    for y in [0i16, 0, 700] {
        g.extend_from_slice(&y.to_be_bytes());
    }
    let mut loca = Vec::new();
    for o in [0u32, 0, g.len() as u32] {
        loca.extend_from_slice(&o.to_be_bytes());
    }
    (g, loca)
}

fn assert_triangle(font: &[u8]) {
    let face = ttf_parser::Face::parse(font, 0).expect("parses");
    let gid = face.glyph_index('A').expect("cmap maps A");
    assert_eq!(gid.0, 1);
    let bbox = face
        .outline_glyph(gid, &mut NullOutline)
        .expect("glyph 1 has an outline");
    assert_eq!(
        (bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max),
        (0, 0, 500, 700)
    );
}

struct NullOutline;
impl ttf_parser::OutlineBuilder for NullOutline {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

#[test]
fn decodes_uncompressed_eot() {
    let (glyf, loca) = triangle_glyf();
    let ttf = tiny_ttf(&glyf, &loca, 2);
    assert_triangle(&ttf);
    let eot = eot_wrap(&ttf, 0);
    let parsed = parse(&eot).unwrap();
    assert_eq!(parsed.family_name, "Sniglet");
    assert_eq!(parsed.style_name, "Bold Italic");
    assert_eq!(parsed.full_name, "Sniglet Bold Italic");
    assert!(parsed.italic);
    assert_eq!(parsed.weight, 700);
    assert_eq!(decode(&eot).unwrap(), ttf);
}

#[test]
fn decodes_xor_obfuscated_eot() {
    let (glyf, loca) = triangle_glyf();
    let ttf = tiny_ttf(&glyf, &loca, 2);
    let xored: Vec<u8> = ttf.iter().map(|b| b ^ XOR_KEY).collect();
    assert_eq!(decode(&eot_wrap(&xored, FLAG_XOR)).unwrap(), ttf);
}

#[test]
fn rejects_bad_headers() {
    assert!(decode(b"short").is_err());
    let mut eot = eot_wrap(&[0, 1, 0, 0], 0);
    eot[34] = 0;
    assert!(decode(&eot).is_err());
    // Header fine, but the font data is not an sfnt.
    assert!(decode(&eot_wrap(b"junkjunk", 0)).is_err());
}

/// LZCOMP encoder for tests: emits literal, DUP and copy symbols.
enum Item {
    Lit(u8),
    Dup(usize),
    Copy { len: usize, dist: usize },
}

fn lz_encode(items: &[Item], out_len: usize) -> Vec<u8> {
    let mut bits = vec![false]; // no run-length layer
    for i in (0..24).rev() {
        bits.push(out_len >> i & 1 == 1);
    }
    let ranges = mtx::dist_ranges(out_len);
    let dup2 = 256 + 8 * ranges;
    let mut sym = Ahuff::new(dup2 + 3);
    let mut len_t = Ahuff::new(8);
    let mut dist_t = Ahuff::new(8);
    for item in items {
        match *item {
            Item::Lit(b) => sym.write(b as u16, &mut bits),
            Item::Dup(back) => sym.write((dup2 + back / 2 - 1) as u16, &mut bits),
            Item::Copy { len, dist } => {
                let coded_len = if dist >= 512 { len - 3 } else { len - 2 };
                let mut digits = vec![coded_len & 3];
                let mut v = coded_len >> 2;
                while v > 0 {
                    digits.push(v & 3);
                    v >>= 2;
                }
                digits.reverse();
                let mut dist_digits = Vec::new();
                let mut d = dist - 1;
                loop {
                    dist_digits.push(d & 7);
                    d >>= 3;
                    if d == 0 {
                        break;
                    }
                }
                dist_digits.reverse();
                let more = |i: usize| if i + 1 < digits.len() { 4 } else { 0 };
                let first = 256 + (dist_digits.len() - 1) * 8 + (digits[0] | more(0));
                sym.write(first as u16, &mut bits);
                for (i, d) in digits.iter().enumerate().skip(1) {
                    len_t.write((d | more(i)) as u16, &mut bits);
                }
                for dd in dist_digits {
                    dist_t.write(dd as u16, &mut bits);
                }
            }
        }
    }
    let mut bytes = vec![0u8; bits.len().div_ceil(8)];
    for (i, b) in bits.iter().enumerate() {
        if *b {
            bytes[i / 8] |= 0x80 >> (i % 8);
        }
    }
    bytes
}

#[test]
fn lzcomp_decodes_literals_dups_and_copies() {
    // 600 leading literals make the output long enough for 4 distance digits.
    let lead: Vec<u8> = (0..600).map(|i| (i % 7) as u8).collect();
    let mut items: Vec<Item> = lead
        .iter()
        .chain(b"abcdefgh")
        .map(|b| Item::Lit(*b))
        .collect();
    items.push(Item::Dup(2)); // g
    items.push(Item::Dup(4)); // f
    items.push(Item::Copy { len: 5, dist: 6 }); // last byte copied is 6 back
    // A long copy reaching into the preloaded history: "\0\0" pairs.
    items.push(Item::Copy { len: 20, dist: 700 });
    let mut expect = [&lead[..], b"abcdefghgf"].concat();
    for _ in 0..5 {
        let p = expect.len();
        expect.push(expect[p + 1 - 6 - 5]);
    }
    // History = preload ++ output; the 20-byte copy ends 700 bytes back.
    let mut hist = Vec::new();
    for k in 0..32u8 {
        for j in 0..96u8 {
            hist.extend_from_slice(&[k, j]);
        }
    }
    for j in 0..=255u8 {
        hist.extend_from_slice(&[j; 4]);
    }
    hist.extend_from_slice(&expect);
    let start = hist.len() - 700 - 20 + 1;
    let tail = hist[start..start + 20].to_vec();
    expect.extend_from_slice(&tail);
    let encoded = lz_encode(&items, expect.len());
    let out = mtx::lzcomp_unpack(&encoded, 3).unwrap();
    assert_eq!(out, expect);
}

#[test]
fn lzcomp_rejects_truncated_input() {
    let items: Vec<Item> = b"hello".iter().map(|b| Item::Lit(*b)).collect();
    let encoded = lz_encode(&items, 50);
    assert!(mtx::lzcomp_unpack(&encoded, 3).is_err());
}

#[test]
fn reads_255_codes() {
    let mut r = Be::new(&[7, 255, 3, 254, 0, 253, 0x12, 0x34]);
    assert_eq!(
        [
            r.u255().unwrap(),
            r.u255().unwrap(),
            r.u255().unwrap(),
            r.u255().unwrap()
        ],
        [7, 256, 506, 0x1234]
    );
    let mut r = Be::new(&[249, 250, 5, 255, 1, 250, 254, 2, 253, 0xFF, 0xFE]);
    let v: Vec<i16> = (0..5).map(|_| r.i255().unwrap()).collect();
    assert_eq!(v, [249, -5, 251, -502, -2]);
}

#[test]
fn expands_hop_codes() {
    // A, X1, Hop3 X2 → A X1 A X2 A; then Hop4 X3 X4 with A' = X2.
    let mut r = Be::new(&[10, 20, 251, 30, 252, 40, 50]);
    let v = ctf::read_push_values(&mut r, 10).unwrap();
    assert_eq!(v, [10, 20, 10, 30, 10, 30, 40, 30, 50, 30]);
}

#[test]
fn encodes_push_instructions() {
    let mut out = Vec::new();
    ctf::encode_push(&[1, 2, -1, 300], &mut out);
    assert_eq!(out, [0xB1, 1, 2, 0xB9, 0xFF, 0xFF, 0x01, 0x2C]);
    let mut out = Vec::new();
    ctf::encode_push(&[5; 9], &mut out);
    assert_eq!(&out[..2], &[0x40, 9]);
}

#[test]
fn decodes_cvt_deltas() {
    // Deltas: 10; code 249 = index 2: 2 × 238 + 4; code 240 = -(1 × 238 + 0); word -32768.
    let data = [0, 4, 10, 249, 4, 240, 0, 238, 0x80, 0x00];
    let out = ctf::cvt_to_ttf(&data).unwrap();
    let v: Vec<i16> = out
        .chunks(2)
        .map(|c| i16::from_be_bytes([c[0], c[1]]))
        .collect();
    assert_eq!(v, [10, 490, 252, 252i16.wrapping_add(i16::MIN)]);
}

#[test]
fn reads_magnitude_coded_values() {
    // The decoding example from the MTX submission, section 5.3.1.
    let data = [0x90, 0x48, 0x84];
    let mut r = ctf::MagReader::new(&data);
    let v: Vec<i32> = (0..11).map(|_| r.value().unwrap()).collect();
    assert_eq!(v, [0, 0, 0, 0, 1, 1, 0, 1, 1, 0, 1]);
}

/// The triangle font in CTF form: empty loca, CTF glyf with one empty glyph
/// and one simple glyph using a few triplet encodings, plus push/code blocks.
fn triangle_ctf() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut g = Vec::new();
    g.extend_from_slice(&0i16.to_be_bytes()); // glyph 0: empty
    g.extend_from_slice(&1i16.to_be_bytes()); // glyph 1: 1 contour
    g.push(2); // end point of the contour
    // Flags: triplet 1 (dy in 8 bits, +), 11 (dx in 8 bits, +; off-curve),
    // 126 (dx and dy in 16 bits; dx negative, dy positive).
    g.extend_from_slice(&[1, 11 | 0x80, 126]);
    g.push(0); // point 0: (0, 0)
    g.push(244); // point 1: (244, 0)
    g.extend_from_slice(&244u16.to_be_bytes()); // point 2: (0, 700)
    g.extend_from_slice(&700u16.to_be_bytes());
    g.push(3); // pushCount
    g.push(2); // codeSize
    (g, vec![1, 2, 3], vec![0x2B, 0x2B])
}

#[test]
fn rebuilds_sfnt_from_ctf() {
    let (ctf_glyf, push, code) = triangle_ctf();
    let ctf_font = tiny_ttf(&ctf_glyf, &[], 2);
    let font = ctf::to_sfnt(&ctf_font, &push, &code).unwrap();
    let face = ttf_parser::Face::parse(&font, 0).expect("parses");
    assert_eq!(face.number_of_glyphs(), 2);
    let gid = face.glyph_index('A').unwrap();
    let bbox = face.outline_glyph(gid, &mut NullOutline).unwrap();
    assert_eq!(
        (bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max),
        (0, 0, 244, 700)
    );
    assert!(
        face.outline_glyph(ttf_parser::GlyphId(0), &mut NullOutline)
            .is_none()
    );
    // Instructions: PUSHB[3] 1 2 3, then the two code bytes.
    let glyf = face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"glyf"))
        .unwrap();
    let ins_len_at = 10 + 2;
    assert_eq!(&glyf[ins_len_at..ins_len_at + 2], &[0, 6]);
    assert_eq!(
        &glyf[ins_len_at + 2..ins_len_at + 8],
        &[0xB2, 1, 2, 3, 0x2B, 0x2B]
    );
}

#[test]
fn rebuilds_composite_glyphs() {
    let mut g = Vec::new();
    g.extend_from_slice(&0i16.to_be_bytes());
    g.extend_from_slice(&(-1i16).to_be_bytes());
    for v in [0i16, 0, 10, 10] {
        g.extend_from_slice(&v.to_be_bytes());
    }
    // One component: ARGS_ARE_XY_VALUES | WE_HAVE_INSTRUCTIONS, glyph 0, byte args.
    g.extend_from_slice(&0x0102u16.to_be_bytes());
    g.extend_from_slice(&0u16.to_be_bytes());
    g.extend_from_slice(&[5, 6]);
    g.extend_from_slice(&[0, 1]); // pushCount 0, codeSize 1
    let font = ctf::to_sfnt(&tiny_ttf(&g, &[], 2), &[], &[0x2B]).unwrap();
    let face = ttf_parser::Face::parse(&font, 0).unwrap();
    let glyf = face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"glyf"))
        .unwrap();
    let expect: Vec<u8> = [
        &(-1i16).to_be_bytes()[..],
        &[0, 0, 0, 0, 0, 10, 0, 10],
        &[0x01, 0x02, 0, 0, 5, 6],
        &[0, 1, 0x2B],
    ]
    .concat();
    assert_eq!(&glyf[..expect.len()], &expect[..]);
}

#[test]
fn mtx_container_round_trip() {
    let (ctf_glyf, push, code) = triangle_ctf();
    let ctf_font = tiny_ttf(&ctf_glyf, &[], 2);
    let lit = |d: &[u8]| {
        lz_encode(
            &d.iter().map(|b| Item::Lit(*b)).collect::<Vec<_>>(),
            d.len(),
        )
    };
    let (b1, b2, b3) = (lit(&ctf_font), lit(&push), lit(&code));
    let mut mtx_data = vec![3, 0, 0x10, 0];
    let off2 = 10 + b1.len();
    let off3 = off2 + b2.len();
    for o in [off2, off3] {
        mtx_data.extend_from_slice(&(o as u32).to_be_bytes()[1..]);
    }
    mtx_data.extend_from_slice(&b1);
    mtx_data.extend_from_slice(&b2);
    mtx_data.extend_from_slice(&b3);
    let eot = eot_wrap(&mtx_data, FLAG_COMPRESSED);
    let font = decode(&eot).unwrap();
    assert!(ttf_parser::Face::parse(&font, 0).is_ok());
    // Same through the XOR layer (applied after compression).
    let xored: Vec<u8> = mtx_data.iter().map(|b| b ^ XOR_KEY).collect();
    assert_eq!(
        decode(&eot_wrap(&xored, FLAG_COMPRESSED | FLAG_XOR)).unwrap(),
        font
    );
}

/// Corpus check: `EOT_CORPUS=<dir of .pptx> cargo test --release -p office::pptx
/// eot_corpus -- --ignored --nocapture` decodes every `ppt/fonts/*.fntdata` and
/// reports, per variant, how many give a font with a cmap and outlined glyphs.
/// `EOT_DUMP=<dir>` also writes every decoded font there (for external checks).
#[test]
#[ignore]
fn eot_corpus() {
    use std::collections::BTreeMap;
    use std::io::Read;
    let dir = std::env::var("EOT_CORPUS").expect("EOT_CORPUS");
    let mut stats: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for path in entries {
        if path.extension().and_then(|e| e.to_str()) != Some("pptx") {
            continue;
        }
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let Ok(mut zip) = zip::ZipArchive::new(file) else {
            continue;
        };
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).unwrap();
            let Ok(name) = entry.name().map(String::from) else {
                continue;
            };
            if !(name.starts_with("ppt/fonts/") && name.ends_with(".fntdata")) {
                continue;
            }
            let mut bytes = Vec::new();
            if entry.read_to_end(&mut bytes).is_err() {
                continue;
            }
            let variant = match parse(&bytes) {
                Ok(e) if e.flags & FLAG_XOR != 0 && e.flags & FLAG_COMPRESSED != 0 => "mtx+xor",
                Ok(e) if e.flags & FLAG_XOR != 0 => "xor",
                Ok(e) if e.flags & FLAG_COMPRESSED != 0 => "mtx",
                Ok(_) => "uncompressed",
                Err(_) => "not-eot",
            };
            let result = decode(&bytes).and_then(|font| {
                if let Ok(d) = std::env::var("EOT_DUMP")
                    && let Err(e) = std::fs::write(
                        format!(
                            "{d}/{}_{}.ttf",
                            path.file_stem().unwrap().to_string_lossy(),
                            name.replace('/', "_")
                        ),
                        &font,
                    )
                {
                    eprintln!("dump: {e}");
                }
                let face =
                    ttf_parser::Face::parse(&font, 0).map_err(|e| format!("ttf_parser: {e}"))?;
                let cmap = face.tables().cmap.ok_or("no cmap")?;
                if cmap.subtables.is_empty() {
                    return Err("empty cmap".into());
                }
                let outlined = (0..face.number_of_glyphs())
                    .filter(|g| {
                        face.outline_glyph(ttf_parser::GlyphId(*g), &mut NullOutline)
                            .is_some()
                    })
                    .count();
                if outlined == 0 {
                    return Err("no outlined glyphs".into());
                }
                Ok(())
            });
            let s = stats.entry(variant).or_default();
            s.0 += 1;
            match result {
                Ok(()) => s.1 += 1,
                Err(e) => {
                    let n = failures.entry(format!("{variant}: {e}")).or_default();
                    if *n < 3 {
                        eprintln!("FAIL {} {name}: {e}", path.display());
                    }
                    *n += 1;
                }
            }
        }
    }
    for (variant, (total, ok)) in &stats {
        println!(
            "{variant:14} {ok}/{total} ({:.1}%)",
            100.0 * *ok as f64 / (*total).max(1) as f64
        );
    }
    for (reason, n) in &failures {
        println!("  {n:5} {reason}");
    }
}
