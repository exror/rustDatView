fn main() {
    println!("cargo:rerun-if-changed=../../assets/brand/rustdatview.ico");
    println!("cargo:rerun-if-changed=rustdatview.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // Windows 11 24H2 and later skip the console window when the app is
    // started from Explorer (see the manifest, which link.exe embeds in place
    // of the toolchain's default one). MinGW always links its own default
    // manifest, so GNU builds rely on the fallback in `main.rs` instead.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rustdatview.manifest");
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
    let version = env!("CARGO_PKG_VERSION");
    let mut parts = [0u64; 4];
    for (slot, part) in parts.iter_mut().zip(version.split('.')) {
        *slot = part
            .split('-')
            .next()
            .unwrap_or("0")
            .parse()
            .unwrap_or(0)
            .min(u16::MAX as u64);
    }
    let numeric = parts
        .into_iter()
        .fold(0u64, |value, part| (value << 16) | part);
    // Task Manager and Explorer show FileDescription as the program name.
    // Resource compilers differ in how they read non-ASCII, so keep it ASCII.
    winresource::WindowsResource::new()
        .set_icon("../../assets/brand/rustdatview.ico")
        .set_language(0x0409)
        .set("ProductName", "rustDatView")
        .set("FileDescription", "rustDatView")
        .set("LegalCopyright", "MIT License")
        .set("OriginalFilename", "rustDatView.exe")
        .set("ProductVersion", version)
        .set("FileVersion", version)
        .set_version_info(winresource::VersionInfo::PRODUCTVERSION, numeric)
        .set_version_info(winresource::VersionInfo::FILEVERSION, numeric)
        .compile()
        .expect("failed to compile the Windows application icon and version resource");
}
