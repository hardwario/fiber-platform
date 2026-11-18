//! Hands the pinned TOWER revision to the crate as the `FIBER_TOWER_REV` environment variable.
//!
//! The revision is read from the workspace `Cargo.lock`, the one place that records which commit
//! of github.com/hardwario/tower the build really used. The radio of a FIBER is compatible with
//! TOWER nodes built from exactly that commit, so the binary must be able to say which one it is
//! (`fiber version --all`). The build fails if the lock names more than one TOWER commit: there
//! is one pin (the root `Cargo.toml`).

use std::error::Error;
use std::path::PathBuf;
use std::{env, fs};

const TOWER_SOURCE: &str = "source = \"git+https://github.com/hardwario/tower?";

fn main() -> Result<(), Box<dyn Error>> {
    let lock = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());
    let text = fs::read_to_string(&lock).map_err(|e| format!("cannot read {}: {e}", lock.display()))?;

    // `source = "git+https://github.com/hardwario/tower?rev=<rev>#<commit>"`, once per TOWER crate.
    let mut commits: Vec<&str> = text
        .lines()
        .filter_map(|line| line.trim().strip_prefix(TOWER_SOURCE))
        .filter_map(|rest| rest.trim_end_matches('"').split_once('#'))
        .map(|(_, commit)| commit)
        .collect();
    commits.sort_unstable();
    commits.dedup();

    carry_firmware()?;

    match commits.as_slice() {
        [commit] => println!("cargo:rustc-env=FIBER_TOWER_REV={commit}"),
        [] => return Err("Cargo.lock names no github.com/hardwario/tower crate".into()),
        many => {
            return Err(format!(
                "Cargo.lock names {} TOWER commits, there must be one pin",
                many.len()
            )
            .into());
        }
    }
    Ok(())
}

/// The south bridge's firmware image, built into the program when `FIBER_SOUTH_IMAGE` names it
/// (`just build-fiber` does: the firmware as a binary image, made by llvm-objcopy). Without it the
/// program carries no image and never flashes the chip; that is the build for the tests.
fn carry_firmware() -> Result<(), Box<dyn Error>> {
    println!("cargo::rustc-check-cfg=cfg(fiber_south_image)");
    println!("cargo:rerun-if-env-changed=FIBER_SOUTH_IMAGE");
    let Ok(image) = env::var("FIBER_SOUTH_IMAGE") else {
        return Ok(());
    };
    println!("cargo:rerun-if-changed={image}");
    let out = PathBuf::from(env::var("OUT_DIR")?).join("south.bin");
    fs::copy(&image, &out).map_err(|e| format!("FIBER_SOUTH_IMAGE: cannot read {image}: {e}"))?;
    println!("cargo:rustc-cfg=fiber_south_image");
    Ok(())
}
