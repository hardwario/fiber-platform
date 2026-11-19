//! The linker's arguments: cortex-m-rt's script with our memory map (memory.x, which leaves the
//! watermark's two pages out of the image), and no padding between sections.
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default());
    if std::fs::copy("memory.x", out.join("memory.x")).is_err() {
        panic!("cannot copy memory.x to OUT_DIR");
    }
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rustc-link-arg=--nmagic");
    println!("cargo:rustc-link-arg=-Tlink.x");
}
