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
