use super::*;

fn asset_package() -> Package {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        "__NATIVE_IMPORT_ASSETS__/0",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"original").unwrap();
    Package::open(zip.finish().unwrap().into_inner(), &Limits::default()).unwrap()
}

#[test]
fn imported_assets_share_reads_without_overwriting_original_parts() {
    let package = asset_package();
    let names = package.names().to_vec();
    let mut bytes = b"imported".to_vec();
    let name = package.import_asset(&bytes).unwrap();
    bytes.fill(0);
    assert_ne!(name, "__native_import_assets__/0");
    assert_eq!(package.names(), names);
    assert_eq!(
        package.read("__native_import_assets__/0").unwrap(),
        b"original"
    );
    assert!(package.exists(&name.to_uppercase()));
    assert_eq!(package.read(&name).unwrap(), b"imported");
    assert_eq!(package.part_size(&name), Some(8));
    assert_eq!(package.read_head(&name, 3).unwrap(), b"imp");
    assert_eq!(package.read_head(&name, u64::MAX).unwrap(), b"imported");
    assert_eq!(package.import_asset(b"imported").unwrap(), name);
    assert_eq!(package.imported.lock().unwrap().bytes, 8);
}

#[test]
fn imported_asset_budget_is_shared_and_failure_is_atomic() {
    let package = asset_package();
    let name = package.import_asset(&vec![7; MAX_IMPORTED_BYTES]).unwrap();
    assert_eq!(
        package.import_asset(&vec![7; MAX_IMPORTED_BYTES]).unwrap(),
        name
    );
    assert!(matches!(
        package.import_asset(&[1]),
        Err(DocError::LimitExceeded(_))
    ));
    assert_eq!(package.part_size(&name), Some(MAX_IMPORTED_BYTES as u64));
    assert_eq!(package.imported.lock().unwrap().parts.len(), 1);
    assert_eq!(
        package.read("__native_import_assets__/0").unwrap(),
        b"original"
    );
}

#[test]
fn imported_asset_count_bounds_small_distinct_values() {
    let package = asset_package();
    for i in 0..MAX_IMPORTED_ASSETS {
        package.import_asset(&i.to_le_bytes()).unwrap();
    }
    assert!(matches!(
        package.import_asset(b"additional"),
        Err(DocError::LimitExceeded(_))
    ));
    assert_eq!(
        package.import_asset(&0usize.to_le_bytes()).unwrap(),
        "__native_import_assets__/1"
    );
}

#[test]
fn resolves_relative_and_absolute_targets() {
    assert_eq!(
        resolve_target("ppt/slides", "../media/image1.png"),
        "ppt/media/image1.png"
    );
    assert_eq!(
        resolve_target("ppt/slides", "/ppt/media/a.png"),
        "ppt/media/a.png"
    );
    assert_eq!(
        resolve_target("ppt", "slides/slide1.xml"),
        "ppt/slides/slide1.xml"
    );
    assert_eq!(
        resolve_target("ppt/slides", "../media/a%20b.png"),
        "ppt/media/a b.png"
    );
}

#[test]
fn percent_before_multibyte_char_is_kept_literally() {
    assert_eq!(resolve_target("ppt", "media/%aé.png"), "ppt/media/%aé.png");
}

#[test]
fn rels_part_name_sits_next_to_the_part() {
    assert_eq!(
        rels_part_name("ppt/slides/slide1.xml"),
        "ppt/slides/_rels/slide1.xml.rels"
    );
    assert_eq!(
        rels_part_name("[Content_Types].xml"),
        "_rels/[Content_Types].xml.rels"
    );
}

#[test]
fn detects_encrypted_compound_file() {
    let mut bytes = CFB_MAGIC.to_vec();
    bytes.extend(vec![0u8; 512]);
    bytes.extend(
        "EncryptedPackage"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes()),
    );
    let err = Package::open(bytes, &Limits::default()).err().unwrap();
    assert!(matches!(err, DocError::Protected));
}

#[test]
fn trims_padding_after_eocd() {
    let mut bytes = vec![b'P', b'K', 3, 4];
    bytes.extend([0x50, 0x4B, 0x05, 0x06]);
    bytes.extend([0u8; 18]);
    bytes.extend([0u8; 10]);
    assert_eq!(trim_after_eocd(bytes).len(), 26);
}
