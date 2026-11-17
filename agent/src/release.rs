//! Which release of the platform this program belongs to.
//!
//! A release is unpacked as `/opt/fiber/releases/<version>/`, with this program in its `bin/`
//! and the roles in its `provision/` (`docs/provisioning.md`). The program finds its release
//! from its own path, never through `/opt/fiber/current`: that link may already point at another
//! release, and a program must never run another release's roles.

use std::path::{Path, PathBuf};

/// One unpacked release.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// `/opt/fiber/releases/<version>`.
    pub dir: PathBuf,
    /// The name of that directory.
    pub version: String,
}

impl Release {
    /// The release this program was started from, if it was started from one. The links on the
    /// way (`/usr/local/bin/fiber`, `/opt/fiber/current`) are followed to the real file first.
    pub fn of_this_program() -> Option<Release> {
        let exe = std::env::current_exe().ok()?;
        Self::holding(&std::fs::canonicalize(exe).ok()?)
    }

    /// The release that holds the program at `exe`: the directory above its `bin/`, when that
    /// directory has a `provision/`.
    pub fn holding(exe: &Path) -> Option<Release> {
        let bin = exe.parent()?;
        if bin.file_name()? != "bin" {
            return None;
        }
        let dir = bin.parent()?;
        if !dir.join("provision").is_dir() {
            return None;
        }
        let version = dir.file_name()?.to_str()?.to_owned();
        Some(Release {
            dir: dir.to_path_buf(),
            version,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;

    #[test]
    fn a_program_in_a_release() {
        let tmp = TestDir::new("release");
        let dir = tmp.path().join("opt/fiber/releases/1.2.0");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::create_dir_all(dir.join("provision")).unwrap();
        let release = Release::holding(&dir.join("bin/fiber")).unwrap();
        assert_eq!(release.dir, dir);
        assert_eq!(release.version, "1.2.0");
    }

    #[test]
    fn a_program_outside_a_release() {
        let tmp = TestDir::new("release");
        std::fs::create_dir_all(tmp.path().join("bin")).unwrap();
        // A `bin/` without a `provision/` beside it, as in `target/` or `/usr/local/bin`.
        assert_eq!(Release::holding(&tmp.path().join("bin/fiber")), None);
        assert_eq!(Release::holding(Path::new("/usr/local/bin/fiber")), None);
        assert_eq!(Release::holding(Path::new("fiber")), None);
    }
}
