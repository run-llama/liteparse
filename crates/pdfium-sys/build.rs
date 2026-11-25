use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const PDFIUM_RELEASE_TAG: &str = "chromium/8172";
const PDFIUM_RELEASE_URL: &str = "https://github.com/run-llama/pdfium-binaries/releases/download";

/// The release `PDFIUM_SHA256` describes; it must equal `PDFIUM_RELEASE_TAG`.
/// `node scripts/update-pdfium-sha256.mjs` rewrites it and every hash below
/// for the current `PDFIUM_RELEASE_TAG`. Renovate runs it when it bumps the
/// tag; after changing the tag by hand, run it yourself.
const PDFIUM_SHA256_TAG: &str = "chromium/8172";

/// sha256 of each `<asset>.tgz` in the `PDFIUM_SHA256_TAG` release. A download
/// that does not match is rejected. packages/python/scripts/download-pdfium.sh
/// reads this table too, so keep one `("<asset>", "<sha256>"),` per line.
#[rustfmt::skip]
const PDFIUM_SHA256: &[(&str, &str)] = &[
    ("pdfium-mac-arm64", "7d4b806918e178bdb535563f58753f7cd946b2336cd9a5e65c3889768fa4b44d"),
    ("pdfium-mac-x64", "27b848eaa96bb561fc5bce049bc7a4367715abbe1d1d7a422294e54b1aa4b311"),
    ("pdfium-linux-x64", "8562f2b56bff77b1a9f559807fb4133a3897d5228973b3593cf56fc537c3cb8d"),
    ("pdfium-linux-musl-x64", "78880dffefdde91e770fa6c83bad8020a064212a933e821fe6824ca9dc694a21"),
    ("pdfium-linux-arm64", "b14f40a2671ecaf6ef2c2119fa8dc0f6273b32d6144ba191ed22065dce00e367"),
    ("pdfium-linux-arm", "f532e2a1176b66c08704c6320425260b25a440fa208cbdbfe32a44fd4d19ba79"),
    ("pdfium-win-x64", "e09408aadee8acf729b4c8c852985f05e1ee40652620d251f7ac1eba2e70cd02"),
    ("pdfium-win-arm64", "2f6475b3f5e3293d5472b3ec4e8c85cf72d92c5c8cf8c509e1438ffcf0bc00ac"),
    ("pdfium-win-x86", "4921903223eef6587bfac5b91cbe506e91546e88acc7cf33c944f8112f4c10c5"),
    ("pdfium-wasi-wasm", "4c279f7b1068ff69ae44ead83e91385a92b16062c23a4459986791e126c70ccc"),
];

/// Download attempts before the build fails, and the wait before the first
/// retry (doubled after each failed attempt: 2 s, 4 s, 8 s).
const DOWNLOAD_ATTEMPTS: u32 = 4;
const DOWNLOAD_FIRST_BACKOFF_SECS: u64 = 2;

/// Upper bound on the archive size read into memory (the largest asset,
/// wasi-wasm, is about 10 MB).
const DOWNLOAD_MAX_BYTES: u64 = 256 * 1024 * 1024;

fn main() {
    let (lib_dir, include_dir) = resolve_pdfium_dirs();

    let lib_dir = lib_dir.canonicalize().unwrap_or_else(|e| {
        panic!(
            "pdfium lib dir does not exist at {}: {e}",
            lib_dir.display()
        )
    });
    let include_dir = include_dir.canonicalize().unwrap_or_else(|e| {
        panic!(
            "pdfium include dir does not exist at {}: {e}",
            include_dir.display()
        )
    });

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    if target_arch == "wasm32" {
        // For wasm targets, pdfium is shipped as a static archive (libpdfium.a)
        // and linked statically into the final .wasm module. There is no
        // dynamic loading and no need to copy any shared library.
        //
        // The WASI sysroot libraries (libc, libc++, libc++abi, etc.) must also
        // be linked so that all C/C++ runtime symbols are resolved. Without
        // them, those symbols appear as unresolved "env::" imports in the
        // browser, making the .wasm unusable.
        println!("cargo:rustc-link-lib=static=pdfium");
        println!("cargo:rustc-link-lib=static=c");
        println!("cargo:rustc-link-lib=static=c++");
        println!("cargo:rustc-link-lib=static=c++abi");
        println!("cargo:rustc-link-lib=static=wasi-emulated-mman");
        println!("cargo:rustc-link-lib=static=wasi-emulated-signal");
        // Newer pdfium-binaries releases ship the wasm setjmp/longjmp runtime
        // in a standalone libsetjmp.a, which pdfium's bundled libjpeg/freetype
        // reference for their __c_longjmp error handling. Older releases folded
        // these into libc.a, so only link it when the archive is present.
        //
        // NOTE: libsetjmp.a's single object also redefines the __wasm_setjmp/
        // __wasm_longjmp helpers that Rust's own codegen emits. The
        // --allow-multiple-definition link-arg needed to tolerate that lives in
        // the final cdylib's build script (crates/liteparse-wasm/build.rs),
        // because rustc-link-arg does not propagate to dependent crates.
        if lib_dir.join("libsetjmp.a").exists() {
            println!("cargo:rustc-link-lib=static=setjmp");
        }
        println!("cargo:lib_path={}", lib_dir.display());
    } else {
        // Non-wasm: pdfium is loaded at runtime via libloading (no link-time
        // dynamic dependency). We bake the lib directory path into the binary
        // so the runtime loader knows where to find the shared library.
        println!("cargo:lib_path={}", lib_dir.display());
        println!("cargo:rustc-env=PDFIUM_LIB_DIR={}", lib_dir.display());

        // Copy the dylib into target/<profile>/deps/ so that CI scripts and
        // packaging tools (copy-pdfium.sh, maturin wheel bundling) can find it
        // in the build tree. This is NOT for linking — just discoverability.
        let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
        let dll_dir = if target_os == "windows" {
            lib_dir
                .parent()
                .map(|p| p.join("bin"))
                .unwrap_or(lib_dir.clone())
        } else {
            lib_dir.clone()
        };
        copy_dylib_to_target_deps(&dll_dir);
    }

    run_bindgen(&include_dir);
}

/// Determine where pdfium lib and include dirs are.
/// Priority: env vars > vendor/ dir > auto-download to cache.
fn resolve_pdfium_dirs() -> (PathBuf, PathBuf) {
    // 1. Explicit env var override
    if let (Ok(lib), Ok(inc)) = (env::var("PDFIUM_LIB_PATH"), env::var("PDFIUM_INCLUDE_PATH")) {
        return (PathBuf::from(lib), PathBuf::from(inc));
    }

    // 2. Vendor directory relative to workspace root
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendor_lib = manifest.join("../../vendor/pdfium/release/lib");
    let vendor_include = manifest.join("../../vendor/pdfium/release/include");
    if vendor_lib.exists() && vendor_include.exists() {
        return (vendor_lib, vendor_include);
    }

    // 3. Auto-download to cache
    let cache_dir = pdfium_cache_dir();
    let lib_dir = cache_dir.join("lib");
    let include_dir = cache_dir.join("include");

    if lib_dir.exists() && include_dir.exists() {
        return (lib_dir, include_dir);
    }

    eprintln!("pdfium-sys: downloading pdfium from GitHub releases...");
    download_pdfium(&cache_dir);

    (lib_dir, include_dir)
}

/// ~/.cache/pdfium-rs/<tag>/<asset_stem>/
fn pdfium_cache_dir() -> PathBuf {
    let base = dirs_cache().join("pdfium-rs");
    let tag_safe = PDFIUM_RELEASE_TAG.replace('/', "_");
    let asset = pdfium_asset_stem();
    base.join(tag_safe).join(asset)
}

fn dirs_cache() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_CACHE_HOME") {
        return PathBuf::from(xdg);
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    // Windows -> USERPROFILE
    if target_os == "windows" {
        if let Ok(local_app_data) = env::var("LOCALAPPDATA") {
            return PathBuf::from(local_app_data);
        }
        let home = env::var("USERPROFILE").expect("USERPROFILE env var not set");
        return PathBuf::from(home).join("AppData\\Local");
    }

    let home = env::var("HOME").expect("HOME env var not set");
    if target_os == "macos" {
        PathBuf::from(&home).join("Library/Caches")
    } else {
        PathBuf::from(&home).join(".cache")
    }
}

/// Map target triple to the pdfium-binaries asset name (without .tgz).
fn pdfium_asset_stem() -> &'static str {
    let target = env::var("TARGET").unwrap();
    match target.as_str() {
        "aarch64-apple-darwin" => "pdfium-mac-arm64",
        "x86_64-apple-darwin" => "pdfium-mac-x64",
        // Universal macOS binary works for both, but we prefer arch-specific
        "x86_64-unknown-linux-gnu" => "pdfium-linux-x64",
        "x86_64-unknown-linux-musl" => "pdfium-linux-musl-x64",
        "aarch64-unknown-linux-gnu" | "aarch64-unknown-linux-musl" => "pdfium-linux-arm64",
        "armv7-unknown-linux-gnueabihf" => "pdfium-linux-arm",
        "x86_64-pc-windows-msvc" | "x86_64-pc-windows-gnu" => "pdfium-win-x64",
        "aarch64-pc-windows-msvc" => "pdfium-win-arm64",
        "i686-pc-windows-msvc" | "i686-pc-windows-gnu" => "pdfium-win-x86",
        "wasm32-unknown-unknown" | "wasm32-wasip1" => "pdfium-wasi-wasm",
        other => panic!("unsupported target for pdfium auto-download: {other}"),
    }
}

fn download_pdfium(dest: &Path) {
    let stem = pdfium_asset_stem();
    let asset = format!("{stem}.tgz");
    let tag_encoded = PDFIUM_RELEASE_TAG.replace('/', "%2F");
    let url = format!("{PDFIUM_RELEASE_URL}/{tag_encoded}/{asset}");
    let expected_sha256 = expected_sha256(stem);

    let bytes = download_with_retries(&url, expected_sha256);
    let reader = std::io::Cursor::new(bytes);
    let gz = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(gz);

    // Extract to a temp dir first, then rename atomically
    let tmp = dest.with_extension("tmp");
    if tmp.exists() {
        fs::remove_dir_all(&tmp).ok();
    }
    fs::create_dir_all(&tmp).expect("failed to create temp dir");
    archive
        .unpack(&tmp)
        .expect("failed to extract pdfium archive");

    // Fix dylib install name on macOS so @rpath resolution works
    fix_dylib_install_name(&tmp);

    // Atomic rename into place
    if dest.exists() {
        fs::remove_dir_all(dest).ok();
    }
    fs::rename(&tmp, dest).expect("failed to move pdfium to cache dir");

    eprintln!("pdfium-sys: cached pdfium at {}", dest.display());
}

fn expected_sha256(stem: &str) -> &'static str {
    if PDFIUM_SHA256_TAG != PDFIUM_RELEASE_TAG {
        panic!(
            "pdfium-sys: PDFIUM_SHA256 lists hashes for {PDFIUM_SHA256_TAG} but \
             PDFIUM_RELEASE_TAG is {PDFIUM_RELEASE_TAG}; run \
             `node scripts/update-pdfium-sha256.mjs` to refresh them"
        );
    }
    PDFIUM_SHA256
        .iter()
        .find(|(name, _)| *name == stem)
        .map(|(_, sha256)| *sha256)
        .unwrap_or_else(|| {
            panic!("pdfium-sys: no sha256 for {stem} in PDFIUM_SHA256 (crates/pdfium-sys/build.rs)")
        })
}

/// GET `url` and check the body against `expected_sha256`. Network errors,
/// error statuses, short reads and hash mismatches are retried with
/// exponential backoff, up to `DOWNLOAD_ATTEMPTS` attempts in all; then the
/// build fails with every attempt's error.
fn download_with_retries(url: &str, expected_sha256: &str) -> Vec<u8> {
    let mut errors = Vec::new();
    let mut backoff = std::time::Duration::from_secs(DOWNLOAD_FIRST_BACKOFF_SECS);
    for attempt in 1..=DOWNLOAD_ATTEMPTS {
        eprintln!("pdfium-sys: GET {url} (attempt {attempt}/{DOWNLOAD_ATTEMPTS})");
        match download_once(url, expected_sha256) {
            Ok(bytes) => return bytes,
            Err(e) => {
                eprintln!("pdfium-sys: attempt {attempt} failed: {e}");
                errors.push(format!("attempt {attempt}: {e}"));
            }
        }
        if attempt < DOWNLOAD_ATTEMPTS {
            std::thread::sleep(backoff);
            backoff *= 2;
        }
    }
    panic!(
        "failed to download pdfium from {url} after {DOWNLOAD_ATTEMPTS} attempts:\n  {}\n\
         Set PDFIUM_LIB_PATH and PDFIUM_INCLUDE_PATH to build against a local pdfium instead.",
        errors.join("\n  ")
    );
}

fn download_once(url: &str, expected_sha256: &str) -> Result<Vec<u8>, String> {
    use sha2::{Digest, Sha256};

    let response = ureq::get(url).call().map_err(|e| e.to_string())?;
    let bytes = response
        .into_body()
        .with_config()
        .limit(DOWNLOAD_MAX_BYTES)
        .read_to_vec()
        .map_err(|e| format!("reading the response body: {e}"))?;
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected_sha256 {
        return Err(format!(
            "sha256 mismatch: expected {expected_sha256}, got {actual} ({} bytes)",
            bytes.len()
        ));
    }
    Ok(bytes)
}

/// On macOS, pdfium-binaries ships dylibs with install name `./libpdfium.dylib`.
/// We need `@rpath/libpdfium.dylib` for rpath resolution to work.
fn fix_dylib_install_name(dir: &Path) {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" {
        return;
    }

    let dylib = dir.join("lib/libpdfium.dylib");
    if !dylib.exists() {
        return;
    }

    let status = Command::new("install_name_tool")
        .args(["-id", "@rpath/libpdfium.dylib"])
        .arg(&dylib)
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("pdfium-sys: install_name_tool exited with {s}"),
        Err(e) => eprintln!("pdfium-sys: failed to run install_name_tool: {e}"),
    }
}

/// Copy the pdfium shared library into `target/<profile>/deps/` so that
/// CI scripts and packaging tools can find it in the build tree.
/// This is NOT for linking — pdfium is loaded at runtime via libloading.
fn copy_dylib_to_target_deps(lib_dir: &Path) {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    let dylib_name = match target_os.as_str() {
        "macos" => "libpdfium.dylib",
        "windows" => "pdfium.dll",
        _ => "libpdfium.so",
    };

    let src = lib_dir.join(dylib_name);
    if !src.exists() {
        return;
    }

    // OUT_DIR is typically target/<profile>/build/<pkg>-<hash>/out
    // We want target/<profile>/deps which is 3 levels up then into deps
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    if let Some(build_dir) = out_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
    {
        let deps_dir = build_dir.join("deps");
        if deps_dir.is_dir() {
            let dst = deps_dir.join(dylib_name);
            fs::copy(&src, &dst).unwrap_or_else(|e| {
                panic!("failed to copy {} to {}: {e}", src.display(), dst.display())
            });
        }
    }
}

fn run_bindgen(include_dir: &Path) {
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    let out_file = out_path.join("bindings.rs");

    #[cfg(feature = "bindgen")]
    {
        let bindings = bindgen::Builder::default()
            .header("wrapper.h")
            .clang_arg(format!("-I{}", include_dir.display()))
            .allowlist_function("FPDF.*")
            .allowlist_function("FORM_.*")
            .allowlist_function("FPDFText_.*")
            .allowlist_function("FPDFPage.*")
            .allowlist_function("FPDFLink_.*")
            .allowlist_function("FPDFFont_.*")
            .allowlist_type("FPDF.*")
            .allowlist_type("FS_.*")
            .allowlist_var("FPDF.*")
            .allowlist_var("FLAT.*")
            .derive_debug(true)
            .derive_default(true)
            .layout_tests(false)
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
            .generate()
            .expect("Unable to generate bindings");

        bindings
            .write_to_file(&out_file)
            .expect("Couldn't write bindings!");
    }

    #[cfg(not(feature = "bindgen"))]
    {
        let _ = include_dir;
        let pregenerated = Path::new(env!("CARGO_MANIFEST_DIR")).join("bindings.rs");
        fs::copy(&pregenerated, &out_file).unwrap_or_else(|e| {
            panic!(
                "Failed to copy pre-generated bindings from {}: {e}",
                pregenerated.display()
            )
        });
    }
}
