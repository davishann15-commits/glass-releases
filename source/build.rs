//! LAME, the MP3 encoder behind "Download as MP3", is linked as a library of its
//! own, so that (as its license, the LGPL, asks) anyone can swap in their own
//! build of it. On macOS it's built here from vendor/lame-3.100 and goes in the
//! app bundle (Contents/Frameworks); on Linux the system's libmp3lame is used.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").expect("cargo sets TARGET");
    if !target.contains("apple-darwin") {
        println!("cargo:rustc-link-lib=dylib=mp3lame");
        return;
    }
    let host = std::env::var("HOST").expect("cargo sets HOST");
    // LAME's export list names the decoder's functions (`hip_*`, `lame_decode*`) and
    // one 3.100 doesn't have (`lame_init_old`); built without the decoder, the macOS
    // linker refuses those. The list without them is passed instead.
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let symbols = std::fs::read_to_string("vendor/lame-3.100/include/libmp3lame.sym").expect("LAME's export list");
    let kept: String = symbols.lines().filter(|l| !l.starts_with("hip_") && !l.starts_with("lame_decode") && l.trim() != "lame_init_old").map(|l| format!("{l}\n")).collect();
    let list = out_dir.join("libmp3lame.sym");
    std::fs::write(&list, kept).expect("writing the export list");
    let mut lame = autotools::Config::new("vendor/lame-3.100");
    if host != target {
        // Building the other half of the universal app.
        lame.config_option("host", Some(if target.starts_with("aarch64") { "arm-apple-darwin" } else { "x86_64-apple-darwin" }));
    }
    // libtool keeps `-arch` when it links (it drops a `--target`), so each half of
    // the universal app gets a library of its own architecture.
    let arch = if target.starts_with("aarch64") { "arm64" } else { "x86_64" };
    let out = lame
        .env("MACOSX_DEPLOYMENT_TARGET", "12.0")
        .cflag("-mmacosx-version-min=12.0")
        .cflag(format!("-arch {arch}"))
        .ldflag(format!("-arch {arch}"))
        .make_args(vec![format!("libmp3lame_la_LDFLAGS=-version-info 0:0:0 -export-symbols {} -no-undefined", list.display())])
        .enable_shared()
        .disable_static()
        .disable("decoder", None)
        .disable("frontend", None)
        .disable("gtktest", None)
        .disable("rpath", None)
        .with("pic", None)
        .fast_build(true)
        .build();
    let lib = out.join("lib");
    // Found through the rpath: the bundle's Frameworks folder, or this build's own
    // copy while developing (scripts/package-macos.sh takes that one out).
    let status = std::process::Command::new("install_name_tool").arg("-id").arg("@rpath/libmp3lame.0.dylib").arg(lib.join("libmp3lame.0.dylib")).status();
    assert!(status.is_ok_and(|s| s.success()), "install_name_tool failed on libmp3lame");
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=mp3lame");
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
}
