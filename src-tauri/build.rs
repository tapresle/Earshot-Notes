fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        link_clang_runtime();
    }
    tauri_build::build()
}

/// Objective-C `@available` checks (in llama.cpp's Metal backend and the audio
/// tap) call `__isPlatformVersionAtLeast` when the deployment target is macOS
/// 14.4. That function lives in clang's runtime library, which Rust doesn't
/// link by default.
fn link_clang_runtime() {
    let output = std::process::Command::new("xcrun")
        .args(["clang", "--print-runtime-dir"])
        .output()
        .expect("xcrun is needed to build for macOS; install Xcode or its command line tools");
    let dir = String::from_utf8(output.stdout).expect("utf-8 path");
    println!("cargo:rustc-link-search=native={}", dir.trim());
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
