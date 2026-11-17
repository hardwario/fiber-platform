//! The linker's arguments: cortex-m-rt's script (with embassy-stm32's memory map of the
//! STM32C031C6), and no padding between sections.
fn main() {
    println!("cargo:rustc-link-arg=--nmagic");
    println!("cargo:rustc-link-arg=-Tlink.x");
}
