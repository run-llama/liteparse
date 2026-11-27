use super::*;

pub(crate) use crate::office::doc::fixtures::{fixture, put16, put32};

#[test]
fn extracts_separate_stories_with_source_controls() {
    let result = inspect(&fixture()).unwrap();
    assert_eq!(result.binary_version, 0x112);
    assert_eq!(result.status, "raw_stories_only");
    assert_eq!(result.stories[0].text, "Body\r");
    assert_eq!(result.stories[1].text, "Note\r");
    assert_eq!(result.stories[1].start_cp, 5);
    assert!(result.trailing_utf16_units.is_empty());
}
#[test]
fn rejects_encryption_and_old_versions() {
    let mut b = fixture();
    put16(&mut b, 1536 + 10, 0x300);
    assert!(matches!(
        inspect(&b),
        Err(Error::Unsupported("encrypted binary Word"))
    ));
    put16(&mut b, 1536 + 10, 0x200);
    put16(&mut b, 1536 + 2, 0x68);
    assert!(matches!(
        inspect(&b),
        Err(Error::Unsupported("binary Word revision"))
    ));
}
#[test]
fn rejects_cycles_and_truncated_sector_chains() {
    let mut b = fixture();
    put32(&mut b, 512 + 2 * 4, 2);
    assert!(matches!(
        inspect(&b),
        Err(Error::Invalid("sector chain cycle"))
    ));
    let mut b = fixture();
    put32(&mut b, 512 + 2 * 4, 0xffff_fffe);
    assert!(matches!(inspect(&b), Err(Error::Invalid("short stream"))));
}
#[test]
fn ignores_embedded_object_streams() {
    let mut b = fixture();
    b[1024 + 128 + 66] = 1;
    assert!(matches!(
        inspect(&b),
        Err(Error::Invalid("missing root stream"))
    ));
}
#[test]
fn rejects_directory_cycles_and_noncompound_inputs() {
    let mut b = fixture();
    put32(&mut b, 1024 + 128 + 68, 1);
    assert!(matches!(
        inspect(&b),
        Err(Error::Invalid("directory cycle"))
    ));
    assert!(matches!(
        inspect(b"{\\rtf1 something}"),
        Err(Error::Unsupported(_))
    ));
}
#[test]
fn malformed_prefixes_and_mutations_do_not_panic() {
    let b = fixture();
    for end in (0..b.len()).step_by(31) {
        drop(inspect(&b[..end]));
    }
    for off in (0..b.len()).step_by(47) {
        let mut mutated = b.clone();
        mutated[off..(off + 4).min(b.len())].fill(0xff);
        drop(inspect(&mutated));
    }
}

#[test]
fn reads_mini_stream_and_rejects_mini_chain_cycles() {
    let mut b = fixture();
    put32(&mut b, 1024 + 116, 10);
    put32(&mut b, 1024 + 120, 64);
    put32(&mut b, 1024 + 256 + 116, 0);
    put32(&mut b, 1024 + 256 + 120, 21);
    put32(&mut b, 60, 11);
    put32(&mut b, 64, 1);
    put32(&mut b, 512 + 10 * 4, 0xffff_fffe);
    put32(&mut b, 512 + 11 * 4, 0xffff_fffe);
    b[6144..6656].fill(0xff);
    put32(&mut b, 6144, 0xffff_fffe);
    assert_eq!(inspect(&b).unwrap().stories[1].text, "Note\r");
    put32(&mut b, 6144, 0);
    assert!(matches!(
        inspect(&b),
        Err(Error::Invalid("sector chain cycle"))
    ));
}

#[test]
fn version_four_uses_4096_byte_sectors() {
    let old = fixture();
    let mut b = vec![0; 5 * 4096];
    b[..512].copy_from_slice(&old[..512]);
    put16(&mut b, 26, 4);
    put16(&mut b, 30, 12);
    put32(&mut b, 40, 1);
    b[4096..8192].fill(0xff);
    for (id, next) in [
        (0, 0xffff_fffd),
        (1, 0xffff_fffe),
        (2, 0xffff_fffe),
        (3, 0xffff_fffe),
    ] {
        put32(&mut b, 4096 + id * 4, next);
    }
    b[8192..8704].copy_from_slice(&old[1024..1536]);
    put32(&mut b, 8192 + 256 + 116, 3);
    b[12288..16384].copy_from_slice(&old[1536..5632]);
    b[16384..20480].copy_from_slice(&old[5632..9728]);
    assert_eq!(inspect(&b).unwrap().stories[0].text, "Body\r");
}

#[test]
fn reads_a_short_final_sector_but_rejects_a_missing_one() {
    let mut b = fixture();
    b.extend_from_slice(b"tail");
    let result = inspect(&b).unwrap();
    assert_eq!(result.trailing_container_bytes, 4);
    assert_eq!(result.stories[0].text, "Body\r");
    // Cut inside the last sector: its missing bytes read as zeros.
    b.truncate(19 * 512 - 1);
    assert_eq!(inspect(&b).unwrap().stories[0].text, "Body\r");
    b.truncate(18 * 512);
    assert!(inspect(&b).is_err());
}

#[test]
fn tolerates_writer_quirks_in_fields_that_are_never_read() {
    let mut b = fixture();
    // Root name "\0R" with a two-byte length: no terminating zero.
    b[1024..1088].fill(0);
    put16(&mut b, 1024 + 2, 0x52);
    put16(&mut b, 1024 + 64, 2);
    // An unused entry whose size field is garbage.
    put32(&mut b, 1024 + 3 * 128 + 120, 0xf000_0000);
    // One DIFAT sector (sector 18) holding no FAT ids, its chain ended with FREE.
    b.resize(20 * 512, 0xff);
    put32(&mut b, 68, 18);
    put32(&mut b, 72, 1);
    let c = compound::Compound::open(&b).unwrap();
    assert_eq!(c.stream("WordDocument").unwrap().len(), 4096);
    // The same entry as a storage (never read; storages have no data of their own).
    b[1024 + 3 * 128 + 66] = 1;
    put16(&mut b, 1024 + 3 * 128 + 64, 4);
    put16(&mut b, 1024 + 3 * 128, 0x53);
    assert!(compound::Compound::open(&b).is_ok());
    // A used entry with an oversized stream is still refused.
    put32(&mut b, 1024 + 128 + 120, 0xf000_0000);
    assert!(compound::Compound::open(&b).is_err());
}
