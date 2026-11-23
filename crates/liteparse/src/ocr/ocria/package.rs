//! The ocria-nano model package this build of LiteParse expects: version, where it is published,
//! and every file's size and sha256. `lit models pull` installs it natively; the WASM build's
//! `OcriaNano.fromUrl` fetches the same files and checks them against the same pins.

pub const OCRIA_NANO: &str = "ocria-nano";
/// The ocria-nano release this build of lit installs.
pub const OCRIA_NANO_VERSION: &str = "1.0.0";
/// Where released model files are fetched from (`<base>/<file>`).
pub const MODEL_BASE_URL: &str = "https://huggingface.co/llamaindex/OCRia-nano-1/resolve/v1.0.0";

pub struct PinnedFile {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub const OCRIA_NANO_FILES: &[PinnedFile] = &[
    PinnedFile {
        name: "det.onnx",
        size: 347580,
        sha256: "70ea641d74c4d7d3acb734002cc1507350d8350ad44aeedc6f8ac6017dbcff7a",
    },
    PinnedFile {
        name: "det.meta.json",
        size: 17,
        sha256: "ada6b3c5de0800eb43fa81dbec714d47cd2a712a22952ef23cf38c805b27350e",
    },
    PinnedFile {
        name: "router.onnx",
        size: 159029,
        sha256: "daccb2e3ce3891b7cb95037ada3861e05fd288d7678d2a1e662d9483e04dd361",
    },
    PinnedFile {
        name: "rec_latin.onnx",
        size: 943530,
        sha256: "7ac8f6815b4b127f244c14f86c51f65224c82e022c1ee71f867f2be955be4da4",
    },
    PinnedFile {
        name: "rec_latin.charset.json",
        size: 4933,
        sha256: "d386eea5691a4f3d9e40f48a3979cf859caf626c3ec43214c7911a713e1f4ed0",
    },
    PinnedFile {
        name: "rec_cjk.onnx",
        size: 5077778,
        sha256: "ad7154b54a549dbfead48dd49142bcc76ee168cb0cd874f6efacab543f0b9262",
    },
    PinnedFile {
        name: "rec_cjk.charset.json",
        size: 43274,
        sha256: "0e48eca81c1c0afa5a3e081fdf5fcbdc669e746e7e0a7f264e85ac76c496035b",
    },
    PinnedFile {
        name: "rec_rtl.onnx",
        size: 2842451,
        sha256: "9ccee36bb07b81ef310c6b50e707adabe8c6b92caea61105213686ea8977f4cf",
    },
    PinnedFile {
        name: "rec_rtl.charset.json",
        size: 1876,
        sha256: "8ad9b8f6c1bdc17b42a2351948e84a73c487f5bed7f783bc5e8474efad126a80",
    },
    PinnedFile {
        name: "rec_rtl.meta.json",
        size: 20,
        sha256: "31fd2893cc4d48ec50b4b21a1f113f527ae9a4ac1b3ea395b0cc5969f56d46b5",
    },
    PinnedFile {
        name: "rec_world.onnx",
        size: 3913987,
        sha256: "c57d01a945d741886d356bdedc636f0572d0810c6a7c2f26dbb9ce0466b92220",
    },
    PinnedFile {
        name: "rec_world.charset.json",
        size: 11602,
        sha256: "ebfd39142cec3f267eda2b81b42966ac9b0f73841a52285770f4db40a016d689",
    },
    PinnedFile {
        name: "tablenet.bin",
        size: 17084482,
        sha256: "27b5f3ff2d6842d397a21954026b1e6411e8b35085b0b32c616e67e33852d1b4",
    },
    // the same LayoutNet weights exported to ONNX, run through ONNX Runtime where it is faster
    // (one of the two is installed, see `wanted`)
    PinnedFile {
        name: "tablenet.onnx",
        size: 17182635,
        sha256: "3b913866067d6e43f73a4e4c9c1eace955b7959447043cc39ea1d30e4c552a95",
    },
];

/// Check `bytes` against the pinned size and sha256 of package file `name`.
pub fn check(name: &str, bytes: &[u8]) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let pin = OCRIA_NANO_FILES
        .iter()
        .find(|f| f.name == name)
        .ok_or_else(|| format!("{name} is not part of {OCRIA_NANO}@{OCRIA_NANO_VERSION}"))?;
    if bytes.len() as u64 != pin.size {
        return Err(format!(
            "{name}: {} bytes, expected {} ({OCRIA_NANO}@{OCRIA_NANO_VERSION})",
            bytes.len(),
            pin.size
        ));
    }
    let got = format!("{:x}", Sha256::digest(bytes));
    if got != pin.sha256 {
        return Err(format!(
            "{name}: sha256 {got}, expected {} ({OCRIA_NANO}@{OCRIA_NANO_VERSION})",
            pin.sha256
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn check_rejects_wrong_size_and_unknown_files() {
        assert!(
            super::check("det.meta.json", b"{}")
                .unwrap_err()
                .contains("bytes, expected")
        );
        assert!(
            super::check("nope.onnx", b"")
                .unwrap_err()
                .contains("not part of")
        );
    }
}
