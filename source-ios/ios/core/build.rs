//! 1. Marks the shared desktop modules (included from ../../src by path) as
//!    built for the mobile core: `#[cfg(glass_mobile)]` swaps their few
//!    desktop-only pieces (folders, the egui image loader, window helpers).
//! 2. Builds LAME (vendor/lame-3.100), the MP3 encoder behind "Download as MP3",
//!    for the iOS target (and for macOS, where the core's tests run). The desktop
//!    links it as a separate dylib (LGPL relinkability); on iOS it is linked
//!    statically for now. See
//!    ios/docs/PARITY.md ("LAME on iOS") for what that means for distribution.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(glass_mobile)");
    println!("cargo::rustc-cfg=glass_mobile");
    println!("cargo:rerun-if-changed=build.rs");

    let target = std::env::var("TARGET").expect("cargo sets TARGET");
    let lame_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/lame-3.100").canonicalize().expect("vendor/lame-3.100");
    if target.contains("apple-darwin") {
        // Host builds (the core's tests on the Mac): the same LAME, for macOS.
        let out = autotools::Config::new(&lame_dir)
            .env("CC", "/usr/bin/clang")
            .env("CFLAGS", "-O2")
            .enable_static()
            .disable_shared()
            .disable("decoder", None)
            .disable("frontend", None)
            .disable("gtktest", None)
            .disable("rpath", None)
            .with("pic", None)
            .fast_build(true)
            .build();
        println!("cargo:rustc-link-search=native={}", out.join("lib").display());
        println!("cargo:rustc-link-lib=static=mp3lame");
        return;
    }
    if !target.contains("apple-ios") {
        return;
    }
    let sim = target.ends_with("-sim");
    let sdk = if sim { "iphonesimulator" } else { "iphoneos" };
    let min = std::env::var("IPHONEOS_DEPLOYMENT_TARGET").unwrap_or_else(|_| "26.0".into());
    let triple = if sim { format!("arm64-apple-ios{min}-simulator") } else { format!("arm64-apple-ios{min}") };
    let sysroot = String::from_utf8(std::process::Command::new("xcrun").args(["--sdk", sdk, "--show-sdk-path"]).output().expect("xcrun").stdout).expect("sdk path").trim().to_string();
    let flags = format!("-target {triple} -isysroot {sysroot} -O2");
    let out = autotools::Config::new(&lame_dir)
        .config_option("host", Some("arm-apple-darwin"))
        .env("CC", "/usr/bin/clang")
        .env("CFLAGS", &flags)
        .env("LDFLAGS", format!("-target {triple} -isysroot {sysroot}"))
        .enable_static()
        .disable_shared()
        .disable("decoder", None)
        .disable("frontend", None)
        .disable("gtktest", None)
        .disable("rpath", None)
        .with("pic", None)
        .fast_build(true)
        .build();
    println!("cargo:rustc-link-search=native={}", out.join("lib").display());
    println!("cargo:rustc-link-lib=static=mp3lame");
}
