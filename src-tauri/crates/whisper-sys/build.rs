use std::env;
use std::path::PathBuf;

fn main() {
    // The library is loaded at run time, not linked, so a missing one fails
    // only when Whisper is used. Debug builds can load it from the vendor
    // folder, where scripts/build-whisper.sh puts it.
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let lib = manifest_dir.join("../../vendor/whisper/macos-arm64/libearshot_whisper.dylib");
    let lib = lib.canonicalize().unwrap_or(lib);
    println!(
        "cargo:rustc-env=EARSHOT_WHISPER_VENDOR_LIBRARY={}",
        lib.display()
    );
    println!("cargo:rerun-if-changed=build.rs");
}
