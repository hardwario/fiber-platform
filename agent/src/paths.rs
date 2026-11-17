//! Every path the `fiber` program reads or writes on the FIBER, in one place.
//!
//! The program stays in its own places (`AGENTS.md`): `/opt/fiber`, `/etc/fiber`,
//! `/var/lib/fiber`, `/run/fiber`. The tests put a root directory in front of all of them
//! (`fiber --root <dir>`), so that every command can run against a temporary directory.

use std::path::PathBuf;

/// The paths of the FIBER platform, below a root directory (`/` on a FIBER).
#[derive(Clone, Debug)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    /// The paths below `root`; `None` means the real ones, below `/`.
    pub fn new(root: Option<PathBuf>) -> Self {
        Self {
            root: root.unwrap_or_else(|| PathBuf::from("/")),
        }
    }

    fn under(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    /// The declared state: which components are wanted, and the settings.
    pub fn platform_file(&self) -> PathBuf {
        self.under("/etc/fiber/platform.yaml")
    }

    /// What the platform keeps between applies.
    pub fn platform_state_dir(&self) -> PathBuf {
        self.under("/var/lib/fiber/platform")
    }

    /// The last result of each component, written by `fiber platform apply`.
    pub fn platform_results(&self) -> PathBuf {
        self.platform_state_dir().join("results.json")
    }

    /// There while an apply is wanted and not yet done.
    pub fn platform_pending(&self) -> PathBuf {
        self.platform_state_dir().join("pending")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_paths() {
        let paths = Paths::new(None);
        assert_eq!(paths.platform_file(), PathBuf::from("/etc/fiber/platform.yaml"));
        assert_eq!(
            paths.platform_results(),
            PathBuf::from("/var/lib/fiber/platform/results.json")
        );
        assert_eq!(
            paths.platform_pending(),
            PathBuf::from("/var/lib/fiber/platform/pending")
        );
    }

    #[test]
    fn paths_below_a_root() {
        let paths = Paths::new(Some(PathBuf::from("/tmp/root")));
        assert_eq!(
            paths.platform_file(),
            PathBuf::from("/tmp/root/etc/fiber/platform.yaml")
        );
    }
}
