fn main() {
    println!("cargo:rerun-if-changed=../../assets/brand/rustdatview.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
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
