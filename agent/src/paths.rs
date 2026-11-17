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

    /// Any path of the system, below the root.
    pub fn under(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    /// The real system, not a directory for the tests.
    pub fn is_real(&self) -> bool {
        self.root == std::path::Path::new("/")
    }

    /// The agent's settings; it needs none.
    pub fn agent_config(&self) -> PathBuf {
        self.under("/etc/fiber/agent.toml")
    }

    /// The releases, `current` and `previous`.
    pub fn opt_dir(&self) -> PathBuf {
        self.under("/opt/fiber")
    }

    /// The link to `current`'s program, on the PATH.
    pub fn bin_link(&self) -> PathBuf {
        self.under("/usr/local/bin/fiber")
    }

    /// The declared state: which components are wanted, and the settings.
    pub fn platform_file(&self) -> PathBuf {
        self.under("/etc/fiber/platform.yaml")
    }

    /// Root only: the login from the seed, and what the roles make (tokens, keys).
    pub fn secrets_dir(&self) -> PathBuf {
        self.under("/etc/fiber/secrets")
    }

    /// The user name and password typed into FIBER Setup, for the services (D16).
    pub fn login_file(&self) -> PathBuf {
        self.secrets_dir().join("login.json")
    }

    /// What the platform keeps between applies.
    pub fn platform_state_dir(&self) -> PathBuf {
        self.under("/var/lib/fiber/platform")
    }

    /// The last result of each component, written by `fiber platform apply`.
    pub fn platform_results(&self) -> PathBuf {
        self.platform_state_dir().join("results.json")
    }

    /// There while an apply is wanted and not yet done. It holds the id of the last request.
    pub fn platform_pending(&self) -> PathBuf {
        self.platform_state_dir().join("pending")
    }

    /// What a running apply keeps; gone at the next start.
    pub fn platform_run_dir(&self) -> PathBuf {
        self.under("/run/fiber/platform")
    }

    /// The progress of the apply that runs or ran last since the start.
    pub fn platform_status(&self) -> PathBuf {
        self.platform_run_dir().join("status.json")
    }

    /// Every apply's output, appended.
    pub fn platform_log(&self) -> PathBuf {
        self.under("/var/log/fiber/platform.log")
    }

    /// The boot partition, where the status and the log are copied for reading over USB.
    pub fn boot_dir(&self) -> PathBuf {
        self.under("/boot/firmware")
    }

    /// The service that runs the apply.
    pub fn apply_unit(&self) -> PathBuf {
        self.under("/etc/systemd/system/fiber-platform-apply.service")
    }

    /// The Avahi service whose TXT records carry the progress of an apply.
    pub fn avahi_service(&self) -> PathBuf {
        self.under("/etc/avahi/services/fiber-platform.service")
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
