extern crate napi_build;

use std::env;

fn main() {
    napi_build::setup();

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    // Set rpath so the .node binary finds libpdfium next to itself at runtime.
    // In the npm package, libpdfium is bundled alongside the .node file.
    match target_os.as_str() {
        "macos" => {
            // @loader_path = directory containing the .node file
            println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path");
        }
        "linux" => {
            // $ORIGIN = directory containing the .node file
            println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
        }
        _ => {
            // Windows: DLLs are found via PATH or same directory automatically
        }
    }

    // Don't add the build-time pdfium dir as an rpath: pdfium-sys already tries
    // it at runtime (PDFIUM_LIB_DIR), and it would end up in release artifacts.
}
