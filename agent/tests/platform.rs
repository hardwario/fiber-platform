//! `fiber platform` as a user runs it: the program itself, against a temporary root directory.

// Tests may unwrap (`clippy.toml`), but clippy sees that only inside `#[test]` functions, and this
// file has helpers too.
#![allow(clippy::unwrap_used)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

const FIBER: &str = env!("CARGO_BIN_EXE_fiber");

/// A temporary root directory, removed when it goes out of scope.
struct Root(PathBuf);

impl Root {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("fiber-platform-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, rest: &str) -> PathBuf {
        self.0.join(rest)
    }

    fn platform_file(&self) -> PathBuf {
        self.path("etc/fiber/platform.yaml")
    }

    fn write(&self, rest: &str, text: &str) {
        let path = self.path(rest);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(&self, rest: &str) -> String {
        std::fs::read_to_string(self.path(rest)).unwrap()
    }

    /// Runs `fiber --root <root> <args>`: the exit status, what it printed and its errors.
    fn run(&self, args: &str) -> (i32, String, String) {
        run(Path::new(FIBER), &self.0, args)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(self.path("etc/fiber"), std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(program: &Path, root: &Path, args: &str) -> (i32, String, String) {
    let mut attempts = 0;
    let output = loop {
        match Command::new(program)
            .arg("--root")
            .arg(root)
            .args(args.split_whitespace())
            .output()
        {
            // A program the test has just copied can be "busy" on Linux for a moment: a process
            // that another test thread forks meanwhile holds the copy open until it starts its
            // own program (seen in CI, 2026-11-17).
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 100 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            other => break other.unwrap(),
        }
    };
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn list_without_a_file() {
    let root = Root::new();
    let (code, out, err) = root.run("platform list");
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(
        out.starts_with("COMPONENT   STATE       REQUIRES         WHAT\nbase        left alone"),
        "{out}"
    );
    assert!(out.ends_with("yet: nothing is chosen.\n"), "{out}");
}

#[test]
fn status_without_anything() {
    let root = Root::new();
    let (code, out, _) = root.run("platform status");
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("Release  none: this fiber program is not inside a release\n"),
        "{out}"
    );
    assert!(out.contains("Apply    nothing waiting\n"), "{out}");
}

#[test]
fn enable_makes_the_file() {
    let root = Root::new();
    let (code, out, err) = root.run("platform enable node-red");
    assert_eq!((code, err.as_str()), (0, ""));
    let file = root.platform_file();
    assert_eq!(
        out,
        format!(
            "mosquitto: on, node-red requires it\nnode-red: on\nChanged {}.\n",
            file.display()
        )
    );
    let text = root.read("etc/fiber/platform.yaml");
    assert!(text.starts_with("# `fiber platform enable` and `disable` change only the lines under"));
    assert!(
        text.contains("\ncomponents:\n  mosquitto: true\n  node-red: true\nsettings:\n"),
        "{text}"
    );
    // Nothing is left behind from writing it.
    let names: Vec<_> = std::fs::read_dir(root.path("etc/fiber"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["platform.yaml"]);

    let (code, out, _) = root.run("platform list");
    assert_eq!(code, 0);
    assert!(out.contains("\nmosquitto   on          "), "{out}");
    assert!(!out.contains("nothing is chosen"), "{out}");

    let (code, out, _) = root.run("platform enable node-red mosquitto");
    assert_eq!(
        (code, out.as_str()),
        (0, "node-red and mosquitto are on already. Nothing changed.\n")
    );
}

#[test]
fn enable_keeps_what_a_person_wrote() {
    let root = Root::new();
    let before = "# My FIBER, set up by hand.\nschema: 1\ncomponents:\n  base: true      # the board\n  \
                  agent: false    # not yet\n\n# Port 1884 for the lab.\nsettings:\n  mqtt:\n    lan: true\n";
    root.write("etc/fiber/platform.yaml", before);
    std::fs::set_permissions(root.platform_file(), std::fs::Permissions::from_mode(0o640)).unwrap();

    let (code, out, err) = root.run("platform enable agent");
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(
        out.starts_with("mosquitto: on, agent requires it\nagent: on\n"),
        "{out}"
    );
    assert_eq!(
        root.read("etc/fiber/platform.yaml"),
        "# My FIBER, set up by hand.\nschema: 1\ncomponents:\n  base: true      # the board\n  \
         agent: true    # not yet\n  mosquitto: true\n\n# Port 1884 for the lab.\nsettings:\n  mqtt:\n    lan: true\n"
    );
    let mode = std::fs::metadata(root.platform_file())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o640);
}

#[test]
fn disable_what_another_needs() {
    let root = Root::new();
    root.write(
        "etc/fiber/platform.yaml",
        "schema: 1\ncomponents:\n  agent: true\n  node-red: true\n",
    );
    let (code, out, err) = root.run("platform disable mosquitto");
    assert_eq!((code, out.as_str()), (1, ""));
    assert_eq!(
        err,
        "error: mosquitto is needed by agent and node-red, which are on. \
         Turn them off with it: fiber platform disable agent node-red mosquitto\n"
    );
    let (code, out, _) = root.run("platform disable agent node-red mosquitto");
    assert_eq!(code, 0);
    assert!(
        out.starts_with("mosquitto: off\nagent: off\nnode-red: off\nChanged "),
        "{out}"
    );
    assert_eq!(
        root.read("etc/fiber/platform.yaml"),
        "schema: 1\ncomponents:\n  agent: false\n  node-red: false\n  mosquitto: false\n"
    );
}

#[test]
fn mistakes_are_named() {
    let root = Root::new();
    let (code, _, err) = root.run("platform enable nodered");
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "error: there is no component `nodered`. The components are base, mosquitto, agent, node-red and chirpstack\n"
    );

    root.write("etc/fiber/platform.yaml", "schema: 1\ncomponents:\n  base: yes\n");
    for command in ["platform list", "platform status", "platform enable base"] {
        let (code, out, err) = root.run(command);
        assert_eq!((code, out.as_str()), (1, ""), "{command}");
        assert!(
            err.starts_with(&format!(
                "error: {}: line 3 column 9",
                root.platform_file().display()
            )),
            "{err}"
        );
        assert!(err.contains("true or false"), "{err}");
    }
    // A command that is refused leaves the file as it was.
    assert_eq!(
        root.read("etc/fiber/platform.yaml"),
        "schema: 1\ncomponents:\n  base: yes\n"
    );
}

#[test]
fn components_not_one_per_line() {
    let root = Root::new();
    root.write("etc/fiber/platform.yaml", "schema: 1\ncomponents: {base: true}\n");
    let (code, _, err) = root.run("platform enable agent");
    assert_eq!(code, 1);
    assert!(
        err.contains("line 2: the components can only be changed when they are written one per line"),
        "{err}"
    );
    assert_eq!(
        root.read("etc/fiber/platform.yaml"),
        "schema: 1\ncomponents: {base: true}\n"
    );
}

#[test]
fn usage_mistakes() {
    let root = Root::new();
    assert_eq!(root.run("platform enable").0, 2);
    assert_eq!(root.run("platform frobnicate").0, 2);
}

#[test]
fn status_with_results() {
    let root = Root::new();
    root.write(
        "etc/fiber/platform.yaml",
        "schema: 1\ncomponents:\n  mosquitto: true\n",
    );
    root.write(
        "var/lib/fiber/platform/results.json",
        r#"{"schema": 1, "components": {"mosquitto": {"wanted": "on", "result": "ok", "finished": "2026-11-17T21:04:05Z"}}}"#,
    );
    let (code, out, _) = root.run("platform status");
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("\nmosquitto   on          ok, 2026-11-17T21:04:05Z\n"),
        "{out}"
    );

    root.write("var/lib/fiber/platform/pending", "");
    let (code, out, _) = root.run("platform status");
    assert_eq!(code, 3, "{out}");
    assert!(
        out.contains("Apply    waiting: an apply was asked for and has not finished\n"),
        "{out}"
    );

    std::fs::remove_file(root.path("var/lib/fiber/platform/pending")).unwrap();
    root.run("platform enable node-red");
    let (code, out, _) = root.run("platform status");
    assert_eq!(code, 3, "{out}");
    assert!(
        out.contains("\nnode-red    on          not applied yet\n"),
        "{out}"
    );
}

#[test]
fn status_names_the_release_the_program_is_in() {
    let root = Root::new();
    let release = root.path("opt/fiber/releases/1.2.0");
    std::fs::create_dir_all(release.join("bin")).unwrap();
    std::fs::create_dir_all(release.join("provision")).unwrap();
    std::fs::copy(FIBER, release.join("bin/fiber")).unwrap();
    // Through a link, as `/usr/local/bin/fiber` is on a FIBER.
    std::fs::create_dir_all(root.path("usr/local/bin")).unwrap();
    std::os::unix::fs::symlink(release.join("bin/fiber"), root.path("usr/local/bin/fiber")).unwrap();

    let (code, out, _) = run(&root.path("usr/local/bin/fiber"), &root.0, "platform status");
    assert_eq!(code, 0, "{out}");
    let real = std::fs::canonicalize(&release).unwrap();
    assert!(
        out.starts_with(&format!("Release  1.2.0, in {}\n", real.display())),
        "{out}"
    );
}

#[test]
fn a_file_that_cannot_be_written() {
    let root = Root::new();
    root.write("etc/fiber/platform.yaml", "schema: 1\n");
    std::fs::set_permissions(root.path("etc/fiber"), std::fs::Permissions::from_mode(0o555)).unwrap();
    // Root may write anyway (in a container); then there is nothing to see here.
    if std::fs::write(root.path("etc/fiber/probe"), "").is_ok() {
        return;
    }
    let (code, _, err) = root.run("platform enable base");
    assert_eq!(code, 1);
    assert!(
        err.ends_with("permission denied. Run it as root: sudo fiber platform enable base\n"),
        "{err}"
    );
    assert_eq!(root.read("etc/fiber/platform.yaml"), "schema: 1\n");
}
