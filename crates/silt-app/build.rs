use std::path::{Path, PathBuf};

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    rune_i18n::codegen::generate(Path::new("locales"), &out).expect("locale catalogs");

    // The exe's icon, version info and manifest (asInvoker, per-monitor DPI aware).
    println!("cargo:rerun-if-changed=silt.manifest");
    println!("cargo:rerun-if-changed=../../docs/brand/silt.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../docs/brand/silt.ico")
            .set_manifest_file("silt.manifest")
            .set("ProductName", "Silt")
            .set("FileDescription", "Silt")
            .set("LegalCopyright", "MIT OR Apache-2.0");
        res.compile().expect("windows resources");
    }
}
