//! A minimal binary Word file (two stories, "Body" and "Note") in a version 3
//! compound container, and the little-endian writers used to edit it. Shared by
//! this crate's tests and, through the `fixtures` feature, by crates that accept
//! .doc input.

pub fn put16(b: &mut [u8], off: usize, v: u16) {
    b[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
pub fn put32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn fixture() -> Vec<u8> {
    let mut b = vec![0; 19 * 512];
    b[..8].copy_from_slice(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
    for (off, v) in [(26, 3), (28, 0xfffe), (30, 9), (32, 6)] {
        put16(&mut b, off, v);
    }
    for (off, v) in [
        (44, 1),
        (48, 1),
        (56, 4096),
        (60, 0xffff_fffe),
        (68, 0xffff_fffe),
    ] {
        put32(&mut b, off, v);
    }
    for off in (76..512).step_by(4) {
        put32(&mut b, off, 0xffff_ffff);
    }
    put32(&mut b, 76, 0);
    for off in (512..1024).step_by(4) {
        put32(&mut b, off, 0xffff_ffff);
    }
    put32(&mut b, 512, 0xffff_fffd);
    put32(&mut b, 516, 0xffff_fffe);
    for id in 2..18 {
        put32(
            &mut b,
            512 + id * 4,
            if id == 9 || id == 17 {
                0xffff_fffe
            } else {
                (id + 1) as u32
            },
        );
    }
    for (i, name, kind, start, size) in [
        (0, "Root Entry", 5, 0xffff_fffe, 0),
        (1, "WordDocument", 2, 2, 4096),
        (2, "1Table", 2, 10, 4096),
    ] {
        let d = &mut b[1024 + i * 128..1024 + (i + 1) * 128];
        for (j, u) in name.encode_utf16().enumerate() {
            put16(d, j * 2, u);
        }
        put16(d, 64, (name.len() * 2 + 2) as u16);
        d[66] = kind;
        d[67] = 1;
        for off in [68, 72, 76] {
            put32(d, off, 0xffff_ffff);
        }
        if i == 0 {
            put32(d, 76, 1);
        }
        if i == 1 {
            put32(d, 72, 2);
        }
        put32(d, 116, start);
        put32(d, 120, size);
    }
    let w = &mut b[1536..5632];
    put16(w, 0, 0xa5ec);
    put16(w, 2, 0xc1);
    put16(w, 10, 0x200);
    put16(w, 32, 14);
    put16(w, 62, 22);
    put16(w, 152, 183);
    put16(w, 154 + 183 * 8, 5);
    put16(w, 156 + 183 * 8, 0x112);
    put32(w, 76, 5);
    put32(w, 80, 5);
    put32(w, 154 + 33 * 8, 0);
    put32(w, 158 + 33 * 8, 21);
    w[2048..2058].copy_from_slice(b"Body\rNote\r");
    let t = &mut b[5632..9728];
    t[0] = 2;
    put32(t, 1, 16);
    put32(t, 5, 0);
    put32(t, 9, 10);
    put32(t, 15, 0x4000_1000);
    b
}
