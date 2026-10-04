fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    println!("cargo:rerun-if-changed=native/audio_tap.m");
    println!("cargo:rerun-if-changed=native/audio_tap.h");
    cc::Build::new()
        .file("native/audio_tap.m")
        .flag("-fobjc-arc")
        .flag("-Wall")
        .flag("-Wextra")
        .flag("-Werror")
        .compile("gc_audio_tap");
    for framework in ["AVFoundation", "CoreAudio", "Foundation"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
