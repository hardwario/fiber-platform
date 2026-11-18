//! `fiber platform update`, `rollback` and `uninstall`: what a package manager would do for the
//! platform (docs/provisioning.md, "Update, rollback and uninstall").

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;

use super::apply;
use crate::paths::Paths;
use crate::release::Release;

/// Where the releases are, as GitHub serves their files.
const RELEASES: &str = "https://github.com/hardwario/fiber-platform/releases";

/// `provision/footprint.yaml`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Footprint {
    #[allow(dead_code)]
    pub schema: u32,
    pub paths: Vec<FootprintPath>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FootprintPath {
    pub path: String,
    #[allow(dead_code)]
    pub by: String,
    #[serde(default)]
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    File,
    Directory,
    /// Kept by an uninstall, and named in what it prints.
    Data,
    /// A package's own file the platform rewrote: kept, and named.
    Changed,
}

impl Footprint {
    pub fn read(path: &Path) -> Result<Footprint> {
        let text =
            std::fs::read_to_string(path).map_err(|e| anyhow!("cannot read {}: {e}", path.display()))?;
        serde_saphyr::from_str(&text).map_err(|e| anyhow!("{}: {}", path.display(), e.without_snippet()))
    }
}

fn this_release() -> Result<Release> {
    Release::of_this_program().ok_or_else(|| anyhow!("this fiber program is not inside a release"))
}

/// Runs a program, failing with a message when it cannot run or ends badly.
fn run(command: &mut Command, what: &str) -> Result<()> {
    let status = command.status().map_err(|e| anyhow!("cannot run {what}: {e}"))?;
    if !status.success() {
        bail!("{what} failed ({status})");
    }
    Ok(())
}

/// The sha256 of a file, by `sha256sum`.
fn sha256(path: &Path) -> Result<String> {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .map_err(|e| anyhow!("cannot run sha256sum: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace()
        .next()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("sha256sum said nothing"))
}

/// The value of `VERSION="..."` in an installer.
pub fn installer_version(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix("VERSION=\""))
        .and_then(|rest| rest.strip_suffix('"'))
        .map(str::to_owned)
}

/// The sha256 that `SHA256SUMS` gives for a file.
pub fn sum_of(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (sum, file) = line.split_once("  ")?;
        (file.trim() == name).then(|| sum.to_owned())
    })
}

/// `fiber platform update [--to <version>]`.
pub fn update(paths: &Paths, to: Option<&str>) -> Result<ExitCode> {
    let current = this_release()?;
    let base = std::env::var("FIBER_RELEASES_URL").unwrap_or_else(|_| RELEASES.to_owned());
    let from = match to {
        Some(version) => format!("{base}/download/v{}", version.trim_start_matches('v')),
        None => format!("{base}/latest/download"),
    };
    let dir = paths.platform_run_dir().join("update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow!("cannot write {}: {e}. Run it as root", dir.display()))?;
    for name in ["SHA256SUMS", "install.sh"] {
        run(
            Command::new("curl")
                .args(["-fsSL", "--retry", "3", "-o"])
                .arg(dir.join(name))
                .arg(format!("{from}/{name}")),
            &format!("downloading {from}/{name}"),
        )?;
    }
    let sums = std::fs::read_to_string(dir.join("SHA256SUMS"))?;
    let installer = dir.join("install.sh");
    let want = sum_of(&sums, "install.sh").ok_or_else(|| anyhow!("SHA256SUMS names no install.sh"))?;
    if sha256(&installer)? != want {
        bail!("install.sh does not match the release's SHA256SUMS; nothing was changed");
    }
    let version = installer_version(&std::fs::read_to_string(&installer)?)
        .ok_or_else(|| anyhow!("install.sh names no version"))?;
    if version == current.version {
        println!("Release {version} is the current one already. Nothing to do.");
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "Updating from release {} to {version}, with its installer.",
        current.version
    );
    let mut installer_run = Command::new("bash");
    installer_run.arg(&installer);
    if std::env::var("FIBER_RELEASES_URL").is_ok() {
        installer_run.env("FIBER_RELEASE_URL", format!("{base}/download/v{version}"));
    }
    if !paths.is_real() {
        installer_run.env("FIBER_ROOT", paths.under("/"));
    }
    let status = installer_run
        .status()
        .map_err(|e| anyhow!("cannot run the installer: {e}"))?;
    Ok(ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8))
}

/// Points `link` at `target` by renaming a new link over it.
fn relink(link: &Path, target: &Path) -> Result<()> {
    let name = link
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let new = link.with_file_name(format!(".{name}.new"));
    let _ = std::fs::remove_file(&new);
    std::os::unix::fs::symlink(target, &new)?;
    std::fs::rename(&new, link)?;
    Ok(())
}

/// `fiber platform rollback`.
pub fn rollback(paths: &Paths) -> Result<ExitCode> {
    let opt = paths.opt_dir();
    let current = std::fs::read_link(opt.join("current"))
        .map_err(|_| anyhow!("there is no current release in {}", opt.display()))?;
    let previous = std::fs::read_link(opt.join("previous"))
        .map_err(|_| anyhow!("there is no previous release to go back to"))?;
    if !opt.join(&previous).join(".complete").exists() {
        bail!(
            "the previous release, {}, is not whole; nothing was changed",
            previous.display()
        );
    }
    relink(&opt.join("current"), &previous)?;
    relink(&opt.join("previous"), &current)?;
    let name = |p: &PathBuf| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    println!(
        "Current release: {} (previous: {}). The fiber program, the roles and the units are those of {} \
         again; packages from other sources and data are not put back.",
        name(&previous),
        name(&current),
        name(&previous)
    );
    let mut apply = Command::new(opt.join("current/bin/fiber"));
    if !paths.is_real() {
        apply.arg("--root").arg(paths.under("/"));
    }
    let status = apply
        .args(["platform", "apply"])
        .status()
        .map_err(|e| anyhow!("cannot run the apply: {e}"))?;
    Ok(ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8))
}

/// Removes a path, file or directory; a path that is not there is fine.
fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
    }
}

/// `fiber platform uninstall --yes`.
pub fn uninstall(paths: &Paths, yes: bool) -> Result<ExitCode> {
    if !yes {
        bail!(
            "this takes the FIBER platform off this system: every component is turned off, and \
             /opt/fiber, /etc/fiber (with the login) and the platform's other files are removed. The \
             services' data stays. To do it: sudo fiber platform uninstall --yes"
        );
    }
    let release = this_release()?;
    let footprint = Footprint::read(&release.dir.join("provision/footprint.yaml"))?;

    println!("Turning every component off:");
    if !apply::turn_all_off(paths, &release)? {
        bail!("a component could not be turned off; nothing was removed. The lines above say which");
    }
    let _ = Command::new("systemctl")
        .args(["disable", "--quiet", "fiber-platform-apply.service"])
        .status();

    let mut kept = Vec::new();
    for entry in &footprint.paths {
        let path = paths.under(&entry.path);
        match entry.kind {
            Kind::File | Kind::Directory => {
                remove(&path).map_err(|e| anyhow!("cannot remove {}: {e}", path.display()))?
            }
            Kind::Data | Kind::Changed => {
                if path.exists() {
                    kept.push(entry.path.clone());
                }
            }
        }
    }
    for own in ["/etc/fiber", "/var/lib/fiber", "/var/log/fiber", "/run/fiber"] {
        let path = paths.under(own);
        remove(&path).map_err(|e| anyhow!("cannot remove {}: {e}", path.display()))?;
    }
    let _ = Command::new("systemctl").arg("daemon-reload").status();
    // Last: the releases, this program among them (Linux runs it on from memory).
    remove(&paths.bin_link())?;
    remove(&paths.opt_dir())?;

    println!("The FIBER platform is off this system.");
    if !kept.is_empty() {
        println!("Kept, as data or as a package's own file the platform changed:");
        for path in &kept {
            println!("  {path}");
        }
    }
    let passwd = std::fs::read_to_string(paths.under("/etc/passwd")).unwrap_or_default();
    println!(
        "The packages from other sources are still installed. To take them off too, with their data:\n  \
         sudo apt purge mosquitto mosquitto-clients nodejs chirpstack-sqlite chirpstack-mqtt-forwarder \
         redis-server\n  sudo rm -rf{}{}",
        kept.iter().map(|p| format!(" {p}")).collect::<String>(),
        users_left(&passwd)
            .iter()
            .map(|user| format!("\n  sudo userdel {user}"))
            .collect::<String>()
    );
    Ok(ExitCode::SUCCESS)
}

/// The users the roles made that are still there: an uninstall leaves them, as a package does.
fn users_left(passwd: &str) -> Vec<&'static str> {
    ["fiber-agent", "nodered"]
        .into_iter()
        .filter(|user| passwd.lines().any(|line| line.split(':').next() == Some(*user)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_users_left_are_named() {
        // `fiber` is the owner's login, as FIBER Setup suggests it: never named.
        let passwd = "root:x:0:0::/root:/bin/bash\nnodered:x:998:998::/var/lib/node-red:/usr/sbin/nologin\n\
                      fiber:x:1000:1000::/home/fiber:/bin/bash\nfiber-agent:x:997:997::/var/lib/fiber/agent:/usr/sbin/nologin\n";
        assert_eq!(users_left(passwd), ["fiber-agent", "nodered"]);
        assert_eq!(
            users_left("fiber:x:1000:1000::/home/fiber:/bin/bash\n"),
            Vec::<&str>::new()
        );
        assert!(users_left("").is_empty());
    }

    #[test]
    fn what_a_release_says() {
        let sums = "abc123  fiber-platform-0.1.0-linux-arm64.tar.gz\ndef456  install.sh\n";
        assert_eq!(sum_of(sums, "install.sh").as_deref(), Some("def456"));
        assert_eq!(sum_of(sums, "other"), None);
        assert_eq!(
            installer_version("#!/bin/bash\nVERSION=\"1.2.3\"\n").as_deref(),
            Some("1.2.3")
        );
        assert_eq!(installer_version("no version"), None);
    }

    #[test]
    fn the_footprint_file_is_good() {
        let footprint: Footprint =
            serde_saphyr::from_str(include_str!("../../../provision/footprint.yaml")).unwrap();
        for entry in &footprint.paths {
            assert!(entry.path.starts_with('/'), "{}", entry.path);
            for own in ["/opt/fiber", "/etc/fiber", "/var/lib/fiber"] {
                assert!(
                    !entry.path.starts_with(own),
                    "{} is in the platform's own places",
                    entry.path
                );
            }
        }
    }

    #[test]
    fn rollback_swaps_the_links() {
        let tmp = crate::testdir::TestDir::new("rollback");
        let paths = Paths::new(Some(tmp.path().to_path_buf()));
        let opt = paths.opt_dir();
        for v in ["1.0.0", "1.1.0"] {
            std::fs::create_dir_all(opt.join("releases").join(v)).unwrap();
            std::fs::write(opt.join("releases").join(v).join(".complete"), "").unwrap();
        }
        std::os::unix::fs::symlink("releases/1.1.0", opt.join("current")).unwrap();
        std::os::unix::fs::symlink("releases/1.0.0", opt.join("previous")).unwrap();
        // The apply cannot run here (no program in the release); the links are what is tested.
        let _ = rollback(&paths);
        assert_eq!(
            std::fs::read_link(opt.join("current")).unwrap(),
            PathBuf::from("releases/1.0.0")
        );
        assert_eq!(
            std::fs::read_link(opt.join("previous")).unwrap(),
            PathBuf::from("releases/1.1.0")
        );
    }
}
