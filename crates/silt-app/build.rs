use std::path::{Path, PathBuf};

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    rune_i18n::codegen::generate(Path::new("locales"), &out).expect("locale catalogs");
}
