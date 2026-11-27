use super::*;

#[test]
fn raster_size_keeps_long_edge_and_aspect() {
    assert_eq!(raster_size(1000.0, 500.0), Some((2048, 1024)));
    assert_eq!(raster_size(10.0, 40.0), Some((512, 2048)));
    assert_eq!(raster_size(0.0, 5.0), None);
    let (w, h) = raster_size(1.0, 100_000.0).unwrap();
    assert_eq!((w, h), (1, 2048));
}

#[test]
fn reader_is_bounds_checked() {
    let r = Rd(&[1, 0, 0, 0, 0xFF]);
    assert_eq!(r.u32(0), Some(1));
    assert_eq!(r.u32(2), None);
    assert_eq!(r.u8(4), Some(0xFF));
    assert_eq!(r.u16(usize::MAX), None);
    assert_eq!(r.bytes(1, usize::MAX), None);
}

#[test]
fn garbage_is_not_a_metafile() {
    let fonts = FontSystem::new(&[], false, &Default::default());
    assert!(render(b"not a metafile at all", &fonts).is_none());
    assert!(render(&[], &fonts).is_none());
}

/// Dev harness: renders every metafile in `$METAFILE_DIR` to `$METAFILE_OUT/<name>.png`
/// and prints timings and failures. `cargo test -p liteparse --lib --release metafile_harness -- --ignored --nocapture`
#[test]
#[ignore]
fn metafile_harness() {
    let (Ok(dir), Ok(out)) = (std::env::var("METAFILE_DIR"), std::env::var("METAFILE_OUT")) else {
        return;
    };
    std::fs::create_dir_all(&out).unwrap();
    let filter = std::env::var("METAFILE_FILTER").unwrap_or_default();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().unwrap().to_string_lossy().contains(&filter))
        .collect();
    files.sort();
    let threads = 8;
    let handles: Vec<_> = (0..threads)
        .map(|i| -> Vec<std::path::PathBuf> {
            files.iter().skip(i).step_by(threads).cloned().collect()
        })
        .map(|chunk| {
            let out = out.clone();
            std::thread::spawn(move || {
                let fonts = FontSystem::new(&[], true, &Default::default());
                for path in chunk {
                    let bytes = std::fs::read(&path).unwrap();
                    let start = std::time::Instant::now();
                    let res = render_with_text(&bytes, &fonts);
                    let ms = start.elapsed().as_millis();
                    let name = path.file_name().unwrap().to_string_lossy().to_string();
                    match res {
                        Some(o) => {
                            let png = format!("{out}/{name}.png");
                            o.pixmap.save_png(&png).unwrap();
                            let texts: Vec<String> =
                                o.text.iter().take(3).map(|t| t.text.clone()).collect();
                            println!(
                                "OK {name} {}x{} {ms}ms texts={} {:?}",
                                o.pixmap.width(),
                                o.pixmap.height(),
                                o.text.len(),
                                texts
                            );
                        }
                        None => println!("FAIL {name} {ms}ms"),
                    }
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn ansi_decoding_by_charset() {
    use super::text::{decode_ansi, decode_ansi_spans, merge_dx};
    assert_eq!(
        decode_ansi(&[b'A', 0x80, 0x93], 0),
        vec!['A' as u32, 0x20AC, 0x201C]
    );
    // Symbol fonts keep byte values.
    assert_eq!(decode_ansi(&[0xE5], 2), vec![0xE5]);
    // Shift-JIS: "あA" is two bytes then one.
    let spans = decode_ansi_spans(&[0x82, 0xA0, b'A'], 128);
    assert_eq!(spans, vec![('あ' as u32, 2), ('A' as u32, 1)]);
    assert_eq!(merge_dx(&spans, &[5.0, 6.0, 7.0]), vec![11.0, 7.0]);
    assert_eq!(decode_ansi(&[0xC0], 204), vec!['А' as u32]);
}

#[test]
fn symbol_font_table() {
    use super::text::symbol_to_unicode;
    assert_eq!(symbol_to_unicode(0xE5), Some('∑'));
    assert_eq!(symbol_to_unicode(0xF0E5), Some('∑'));
    assert_eq!(symbol_to_unicode(0x53), Some('Σ'));
    assert_eq!(symbol_to_unicode(0x2D), Some('−'));
    assert_eq!(symbol_to_unicode(0xFE), Some('⎭'));
    assert_eq!(symbol_to_unicode(0x10), None);
}
