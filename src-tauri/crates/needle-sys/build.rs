use std::env;
use std::path::PathBuf;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let platform = match (target_os.as_str(), target_arch.as_str()) {
        ("macos", "aarch64") => "macos-arm64",
        _ => panic!("needle-sys does not support {target_os}-{target_arch} yet"),
    };

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let lib_dir = manifest_dir.join("../../vendor/needle").join(platform);
    let lib_dir = lib_dir.canonicalize().unwrap_or(lib_dir);
    let lib = lib_dir.join("libneedle.a");
    if !lib.exists() {
        panic!(
            "{} is missing. Run scripts/fetch-native.sh from the repository root.",
            lib.display()
        );
    }

    println!("cargo:rerun-if-changed={}", lib.display());
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=needle");
    println!("cargo:rustc-link-lib=c++");
}
