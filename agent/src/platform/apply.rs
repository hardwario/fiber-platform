//! `fiber platform apply`: brings the system to what `/etc/fiber/platform.yaml` says.
//!
//! The work is done by `fiber platform apply --foreground`, which `fiber-platform-apply.service`
//! runs. Without `--foreground` the command asks the service for an apply and follows it, so an
//! apply survives a dropped SSH session and a power cut, and two never run at once.
//!
//! Each step is one `ansible-playbook` run on the playbook of this program's own release: first
//! `common`, then every component that is `false` is turned off (in the reverse order of
//! `components.yaml`), then every component that is on is brought up (in its order). The result
//! of a step is the exit status of its run. A component whose requirement did not come up in
//! this run is skipped.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::catalog::Catalog;
use super::file::{Platform, Settings};
use super::results::{ComponentResult, Outcome, Results, Wanted};
use super::{State, states, write_file};
use crate::paths::Paths;
use crate::release::Release;

/// Done, and the FIBER needs a restart (a changed `config.txt`).
pub const EXIT_RESTART: u8 = 10;
/// Partly done; the service tries again by itself.
pub const EXIT_PARTLY: u8 = 20;

/// The step before every component: the platform's own directories.
const COMMON: &str = "common";

/// One step: a component (or `common`) brought up or turned off.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub component: String,
    pub roles: Vec<String>,
    pub wanted: Wanted,
}

/// How one step ended.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StepResult {
    pub component: String,
    pub wanted: Wanted,
    pub result: Outcome,
    /// How many things the run changed, when it says.
    pub changes: Option<u32>,
    pub message: Option<String>,
}

impl StepResult {
    /// The line that `apply` prints for it.
    pub fn line(&self) -> String {
        let id = &self.component;
        match self.result {
            Outcome::Ok => {
                let what = if self.wanted == Wanted::On { "up" } else { "off" };
                match self.changes {
                    Some(0) => format!("{id}: {what}, nothing changed"),
                    Some(1) => format!("{id}: {what}, 1 change"),
                    Some(n) => format!("{id}: {what}, {n} changes"),
                    None => format!("{id}: {what}"),
                }
            }
            Outcome::Failed => format!(
                "{id}: failed: {}",
                self.message.as_deref().unwrap_or("see the log")
            ),
            Outcome::Skipped => format!("{id}: skipped: {}", self.message.as_deref().unwrap_or("")),
        }
    }
}

/// Where an apply stands.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ApplyState {
    Running,
    /// Every step ended `ok`.
    Done,
    /// Some step did not; the service tries again.
    Partly,
    /// The apply could not run at all; `message` says why.
    Failed,
}

/// `/run/fiber/platform/status.json`: the progress of the apply that runs, or ran last.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Status {
    pub schema: u32,
    /// The id of the request it answers (the content of `pending`).
    pub run: String,
    pub release: String,
    pub state: ApplyState,
    /// The step that runs, from 1; the number of steps when it is over.
    pub step: usize,
    pub steps: usize,
    pub component: Option<String>,
    pub restart_needed: bool,
    pub message: Option<String>,
    pub results: Vec<StepResult>,
}

/// The steps of an apply: `common`, what is turned off, what is brought up.
pub fn plan(catalog: &Catalog, platform: &Platform) -> Vec<Step> {
    let mut steps = vec![Step {
        component: COMMON.to_owned(),
        roles: vec![COMMON.to_owned()],
        wanted: Wanted::On,
    }];
    let step = |id: &str, wanted| Step {
        component: id.to_owned(),
        roles: catalog.get(id).map(|c| c.roles.clone()).unwrap_or_default(),
        wanted,
    };
    let all = states(catalog, platform);
    for (id, state) in all.iter().rev() {
        if *state == State::Off {
            steps.push(step(id, Wanted::Off));
        }
    }
    for (id, state) in &all {
        if matches!(state, State::On | State::Needed(_)) {
            steps.push(step(id, Wanted::On));
        }
    }
    steps
}

/// How a runner's run of one step went.
pub struct Ran {
    pub ok: bool,
    pub changes: Option<u32>,
    pub message: Option<String>,
}

/// What runs a step: Ansible on a FIBER, a stand-in in the tests.
pub trait Runner {
    fn run(&mut self, step: &Step) -> Ran;
}

/// Runs the steps in order and hands each result to `report` as it ends. A step is skipped when
/// `common` did not come up, or when it brings up a component whose requirement did not.
pub fn run_steps(
    catalog: &Catalog,
    steps: &[Step],
    runner: &mut dyn Runner,
    mut report: impl FnMut(usize, &StepResult),
) -> Vec<StepResult> {
    let mut results: Vec<StepResult> = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let not_up = |id: &str| {
            results
                .iter()
                .any(|r| r.component == id && r.result != Outcome::Ok)
        };
        let blocker = if step.component != COMMON && not_up(COMMON) {
            Some(COMMON.to_owned())
        } else if step.wanted == Wanted::On {
            catalog
                .requirements(&step.component)
                .into_iter()
                .find(|r| not_up(r))
        } else {
            None
        };
        let result = match blocker {
            Some(other) => StepResult {
                component: step.component.clone(),
                wanted: step.wanted,
                result: Outcome::Skipped,
                changes: None,
                message: Some(format!("{other} did not come up")),
            },
            None => {
                let ran = runner.run(step);
                StepResult {
                    component: step.component.clone(),
                    wanted: step.wanted,
                    result: if ran.ok { Outcome::Ok } else { Outcome::Failed },
                    changes: ran.changes,
                    message: ran.message,
                }
            }
        };
        report(index, &result);
        results.push(result);
    }
    results
}

/// The number of changes in the recap line of `ansible-playbook`:
/// `localhost : ok=5 changed=2 unreachable=0 failed=0 ...`.
pub fn changes_in(output: &str) -> Option<u32> {
    output
        .lines()
        .rev()
        .find(|l| l.contains(" ok=") && l.contains(" changed="))
        .and_then(|line| {
            let rest = &line[line.find("changed=")? + "changed=".len()..];
            rest.split_whitespace().next()?.parse().ok()
        })
}

/// The reason of the first failed task: the `msg` of `fatal: [localhost]: FAILED! => {...}`.
pub fn failure_in(output: &str) -> Option<String> {
    let line = output
        .lines()
        .find(|l| l.contains("FAILED!") || l.starts_with("fatal:"))?;
    let json = line.find("=> ").map(|i| &line[i + 3..]);
    let message = json
        .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
        .and_then(|v| v.get("msg").and_then(|m| m.as_str().map(str::to_owned)))
        .unwrap_or_else(|| line.trim().to_owned());
    let message: String = message
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect();
    Some(message)
}

/// Seconds since 1970 as RFC 3339 in UTC, `2026-11-17T21:04:05Z`.
pub fn rfc3339(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rest) = ((secs / 86_400) as i64, secs % 86_400);
    // Civil from days (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// Runs a step with `ansible-playbook` on the playbook of a release.
struct Ansible<'a> {
    paths: &'a Paths,
    release: &'a Release,
    settings: Settings,
    log: Option<File>,
    /// For the converge test: the components whose run fails on purpose.
    fail_on_purpose: Vec<String>,
}

impl Ansible<'_> {
    fn vars(&self, step: &Step) -> serde_json::Value {
        let path = |p: PathBuf| p.display().to_string();
        serde_json::json!({ "fiber": {
            "component": step.component,
            "roles": step.roles,
            "wanted": step.wanted,
            "settings": self.settings,
            "release": self.release.version,
            "paths": {
                "boot": path(self.paths.boot_dir()),
                "secrets": path(self.paths.secrets_dir()),
                "state": path(self.paths.platform_state_dir()),
                "run": path(self.paths.platform_run_dir()),
            },
            "fail_on_purpose": self.fail_on_purpose.contains(&step.component),
        }})
    }

    fn log(&mut self, text: &str) {
        if let Some(log) = &mut self.log {
            let _ = log.write_all(text.as_bytes());
        }
    }
}

impl Runner for Ansible<'_> {
    fn run(&mut self, step: &Step) -> Ran {
        let vars_file = self.paths.platform_run_dir().join("vars.json");
        let vars = self.vars(step).to_string();
        if let Err(e) = write_file(&vars_file, &vars, 0o600) {
            return Ran {
                ok: false,
                changes: None,
                message: Some(format!("cannot write {}: {e}", vars_file.display())),
            };
        }
        let ansible = self.release.dir.join("provision/ansible");
        let what = if step.wanted == Wanted::On { "up" } else { "off" };
        self.log(&format!(
            "--- {} {what}, {}\n",
            step.component,
            rfc3339(SystemTime::now())
        ));
        let output = Command::new("ansible-playbook")
            .args(["-i", "localhost,", "-c", "local"])
            .arg(ansible.join("site.yml"))
            .arg("-e")
            .arg(format!("@{}", vars_file.display()))
            .env("ANSIBLE_CONFIG", ansible.join("ansible.cfg"))
            .env("LC_ALL", "C.UTF-8")
            .env("ANSIBLE_NOCOLOR", "1")
            .output();
        match output {
            Err(e) => {
                let message = format!("cannot run ansible-playbook: {e}");
                self.log(&format!("{message}\n"));
                Ran {
                    ok: false,
                    changes: None,
                    message: Some(message),
                }
            }
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                self.log(&stdout);
                self.log(&stderr);
                let ok = output.status.success();
                Ran {
                    ok,
                    changes: changes_in(&stdout),
                    message: (!ok).then(|| {
                        failure_in(&stdout)
                            .or_else(|| stderr.lines().find(|l| !l.trim().is_empty()).map(str::to_owned))
                            .unwrap_or_else(|| format!("ansible-playbook ended with {}", output.status))
                    }),
                }
            }
        }
    }
}

/// Writes the status, and the Avahi record that carries it.
fn publish(paths: &Paths, status: &Status) {
    if let Ok(json) = serde_json::to_string_pretty(status) {
        let _ = write_file(&paths.platform_status(), &(json + "\n"), 0o644);
    }
    let service = paths.avahi_service();
    if service.parent().is_some_and(Path::is_dir) {
        let _ = write_file(&service, &avahi_text(status), 0o644);
    }
}

/// The Avahi service of the FIBER (`_fiber._tcp`): FIBER Setup follows the first start by its
/// TXT records, with no login. The port is SSH's, where the FIBER is reached.
pub fn avahi_text(status: &Status) -> String {
    let state = match status.state {
        ApplyState::Running => "running",
        ApplyState::Done => "done",
        ApplyState::Partly => "partly",
        ApplyState::Failed => "failed",
    };
    let txt = [
        format!("state={state}"),
        format!("step={}/{}", status.step, status.steps),
        format!("component={}", status.component.as_deref().unwrap_or("")),
        format!("release={}", status.release),
        format!("restart={}", if status.restart_needed { "yes" } else { "no" }),
    ];
    let records: String = txt
        .iter()
        .map(|t| format!("    <txt-record>{t}</txt-record>\n"))
        .collect();
    format!(
        "<?xml version=\"1.0\" standalone=\"no\"?>\n\
         <!-- Written by `fiber platform apply`: the progress of the FIBER platform. -->\n\
         <!DOCTYPE service-group SYSTEM \"avahi-service.dtd\">\n\
         <service-group>\n  <name replace-wildcards=\"yes\">FIBER %h</name>\n  <service>\n    \
         <type>_fiber._tcp</type>\n    <port>22</port>\n{records}  </service>\n</service-group>\n"
    )
}

/// Records one component's result in `results.json` (not `common`'s: it is no component).
fn record(paths: &Paths, result: &StepResult) -> Result<()> {
    if result.component == COMMON {
        return Ok(());
    }
    let path = paths.platform_results();
    let mut results = Results::read(&path)?;
    results.components.insert(
        result.component.clone(),
        ComponentResult {
            wanted: result.wanted,
            result: result.result,
            finished: rfc3339(SystemTime::now()),
            message: result.message.clone(),
        },
    );
    let json = serde_json::to_string_pretty(&results)? + "\n";
    write_file(&path, &json, 0o644).map_err(|e| anyhow!("cannot write {}: {e}", path.display()))
}

/// Copies the status and the log to the boot partition, for reading over USB.
fn copy_to_boot(paths: &Paths) {
    let boot = paths.boot_dir();
    if !boot.is_dir() {
        return;
    }
    for (from, name) in [
        (paths.platform_status(), "fiber-platform-status.json"),
        (paths.platform_log(), "fiber-platform.log"),
    ] {
        if let Ok(text) = std::fs::read(&from) {
            // The partition is FAT: no owner, no mode, so a plain write.
            let _ = std::fs::write(boot.join(name), text);
        }
    }
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

/// One apply: the plan, its steps, the status as it goes. Returns the exit status.
fn apply_once(paths: &Paths, request: &str) -> Result<u8> {
    let release = Release::of_this_program()
        .ok_or_else(|| anyhow!("this fiber program is not inside a release, so it has no roles to run"))?;
    let catalog = Catalog::built_in()?;
    let mut status = Status {
        schema: 1,
        run: request.to_owned(),
        release: release.version.clone(),
        state: ApplyState::Running,
        step: 0,
        steps: 0,
        component: None,
        restart_needed: false,
        message: None,
        results: Vec::new(),
    };
    let platform = match Platform::read(&paths.platform_file(), &catalog) {
        Ok(platform) => platform,
        Err(e) => {
            status.state = ApplyState::Failed;
            status.message = Some(format!("{e:#}"));
            publish(paths, &status);
            return Err(e);
        }
    };
    let steps = plan(&catalog, &platform);
    status.steps = steps.len();
    publish(paths, &status);

    let log_path = paths.platform_log();
    let _ = std::fs::create_dir_all(log_path.parent().unwrap_or(Path::new("/")));
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();
    if let Some(log) = &mut log {
        let _ = writeln!(
            log,
            "=== apply, release {}, {}",
            release.version,
            rfc3339(SystemTime::now())
        );
    }
    let fail_on_purpose = std::fs::read_to_string(paths.platform_run_dir().join("fail-on-purpose"))
        .map(|t| t.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default();
    let mut runner = Ansible {
        paths,
        release: &release,
        settings: platform.settings.clone(),
        log,
        fail_on_purpose,
    };

    let mut record_error = None;
    let results = run_steps(&catalog, &steps, &mut runner, |index, result| {
        println!("{}", result.line());
        if let Err(e) = record(paths, result) {
            record_error.get_or_insert(e);
        }
        status.results.push(result.clone());
        status.step = index + 1;
        status.component = steps.get(index + 1).map(|s| s.component.clone());
        publish(paths, &status);
    });
    if let Some(e) = record_error {
        eprintln!("error: {e:#}");
    }

    let all_ok = results.iter().all(|r| r.result == Outcome::Ok);
    status.restart_needed = paths.platform_run_dir().join("restart-needed").exists();
    status.state = if all_ok {
        ApplyState::Done
    } else {
        ApplyState::Partly
    };
    status.component = None;
    publish(paths, &status);
    if let Some(log) = &mut runner.log {
        let _ = writeln!(
            log,
            "=== {:?}{}",
            status.state,
            if status.restart_needed {
                ", restart needed"
            } else {
                ""
            }
        );
    }
    copy_to_boot(paths);
    Ok(match (all_ok, status.restart_needed) {
        (true, false) => 0,
        (true, true) => EXIT_RESTART,
        (false, _) => EXIT_PARTLY,
    })
}

/// Turns every component off, in the reverse order of `components.yaml`, whatever the file says
/// (for `fiber platform uninstall`). Prints each step; returns whether all of them ended `ok`.
pub fn turn_all_off(paths: &Paths, release: &Release) -> Result<bool> {
    let catalog = Catalog::built_in()?;
    let settings = Platform::read(&paths.platform_file(), &catalog)
        .map(|p| p.settings)
        .unwrap_or_default();
    let steps: Vec<Step> = catalog
        .components()
        .iter()
        .rev()
        .map(|c| Step {
            component: c.id.clone(),
            roles: c.roles.clone(),
            wanted: Wanted::Off,
        })
        .collect();
    std::fs::create_dir_all(paths.platform_run_dir())?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.platform_log())
        .ok();
    let mut runner = Ansible {
        paths,
        release,
        settings,
        log,
        fail_on_purpose: Vec::new(),
    };
    let results = run_steps(&catalog, &steps, &mut runner, |_, result| {
        println!("{}", result.line())
    });
    Ok(results.iter().all(|r| r.result == Outcome::Ok))
}

/// `fiber platform apply --foreground`: the apply itself, in this process. One at a time; a
/// request that comes in while it runs makes it run again.
pub fn foreground(paths: &Paths) -> Result<ExitCode> {
    let run_dir = paths.platform_run_dir();
    std::fs::create_dir_all(&run_dir).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => {
            anyhow!(
                "cannot write {}: permission denied. Run it as root",
                run_dir.display()
            )
        }
        _ => anyhow!("cannot write {}: {e}", run_dir.display()),
    })?;
    let lock = File::create(run_dir.join("apply.lock"))?;
    if lock.try_lock().is_err() {
        bail!("another apply is running");
    }
    loop {
        let request = read_trimmed(&paths.platform_pending()).unwrap_or_default();
        let code = apply_once(paths, &request)?;
        let now = read_trimmed(&paths.platform_pending());
        match now {
            // A new request came while this one ran: run again for it.
            Some(now) if now != request => continue,
            Some(_) if code != EXIT_PARTLY => {
                let _ = std::fs::remove_file(paths.platform_pending());
            }
            _ => {}
        }
        return Ok(ExitCode::from(code));
    }
}

fn systemctl(args: &[&str]) -> Result<String> {
    let output = Command::new("systemctl")
        .args(args)
        .output()
        .map_err(|e| anyhow!("cannot run systemctl: {e}"))?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Puts the release's `fiber-platform-apply.service` in place when it differs.
fn install_unit(paths: &Paths, release: &Release) -> Result<()> {
    let source = release.dir.join("provision/systemd/fiber-platform-apply.service");
    let text =
        std::fs::read_to_string(&source).map_err(|e| anyhow!("cannot read {}: {e}", source.display()))?;
    let unit = paths.apply_unit();
    if std::fs::read_to_string(&unit).ok().as_deref() != Some(text.as_str()) {
        write_file(&unit, &text, 0o644).map_err(|e| anyhow!("cannot write {}: {e}", unit.display()))?;
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", "--quiet", "fiber-platform-apply.service"])?;
    }
    Ok(())
}

/// `fiber platform apply`: asks the service for an apply and follows it.
pub fn apply(paths: &Paths) -> Result<ExitCode> {
    if !Path::new("/run/systemd/system").is_dir() {
        bail!(
            "systemd is not running here, so the apply cannot run as its service. \
             Run it in this process: sudo fiber platform apply --foreground"
        );
    }
    let release = Release::of_this_program()
        .ok_or_else(|| anyhow!("this fiber program is not inside a release, so it has no roles to run"))?;
    install_unit(paths, &release).map_err(|e| match e.to_string().contains("permission denied") {
        true => anyhow!("{e:#}. Run it as root: sudo fiber platform apply"),
        false => e,
    })?;
    let request = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .to_string();
    let pending = paths.platform_pending();
    write_file(&pending, &format!("{request}\n"), 0o644)
        .map_err(|e| anyhow!("cannot write {}: {e}", pending.display()))?;
    systemctl(&["start", "--no-block", "fiber-platform-apply.service"])?;
    println!(
        "The apply runs as fiber-platform-apply.service; its log: {}",
        paths.platform_log().display()
    );
    follow(paths, &request)
}

/// Prints each step of the apply that answers `request` as it ends, until it is over.
fn follow(paths: &Paths, request: &str) -> Result<ExitCode> {
    let started = Instant::now();
    let mut shown = 0;
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let status: Option<Status> = std::fs::read_to_string(paths.platform_status())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .filter(|s: &Status| s.run == request);
        let Some(status) = status else {
            // Not started yet. If the service is not even running, it is not going to.
            if started.elapsed() > Duration::from_secs(60) {
                let state = systemctl(&["is-active", "fiber-platform-apply.service"])?;
                if state != "active" && state != "activating" {
                    bail!(
                        "the apply service stopped without a result. See: journalctl -u fiber-platform-apply"
                    );
                }
            }
            continue;
        };
        for result in status.results.iter().skip(shown) {
            println!("{}", result.line());
        }
        shown = status.results.len();
        match status.state {
            ApplyState::Running => {}
            ApplyState::Failed => {
                bail!(
                    "the apply could not run: {}",
                    status.message.as_deref().unwrap_or("see the log")
                )
            }
            ApplyState::Done if status.restart_needed => {
                println!("Done. The FIBER needs a restart: sudo systemctl reboot");
                return Ok(ExitCode::from(EXIT_RESTART));
            }
            ApplyState::Done => {
                println!("Done.");
                return Ok(ExitCode::SUCCESS);
            }
            ApplyState::Partly => {
                println!("Partly done. The platform tries again by itself; see: fiber platform status");
                return Ok(ExitCode::from(EXIT_PARTLY));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn platform(pairs: &[(&str, bool)]) -> Platform {
        Platform {
            components: pairs
                .iter()
                .map(|(id, on)| ((*id).to_owned(), *on))
                .collect::<BTreeMap<_, _>>(),
            ..Platform::default()
        }
    }

    fn names(steps: &[Step]) -> Vec<String> {
        steps
            .iter()
            .map(|s| {
                format!(
                    "{} {}",
                    s.component,
                    if s.wanted == Wanted::On { "on" } else { "off" }
                )
            })
            .collect()
    }

    #[test]
    fn the_plan() {
        let catalog = Catalog::built_in().unwrap();
        let steps = plan(
            &catalog,
            &platform(&[("agent", true), ("node-red", false), ("chirpstack", false)]),
        );
        assert_eq!(
            names(&steps),
            [
                "common on",
                "chirpstack off",
                "node-red off",
                "base on",
                "mosquitto on",
                "agent on"
            ]
        );
        assert_eq!(steps[5].roles, ["agent"]);
        assert_eq!(names(&plan(&catalog, &Platform::default())), ["common on"]);
    }

    /// A stand-in for Ansible: fails the components it is told to, and counts the runs.
    struct Fake {
        fail: Vec<&'static str>,
        ran: Vec<String>,
    }

    impl Runner for Fake {
        fn run(&mut self, step: &Step) -> Ran {
            self.ran.push(step.component.clone());
            let ok = !self.fail.contains(&step.component.as_str());
            Ran {
                ok,
                changes: Some(2),
                message: (!ok).then(|| "no network".to_owned()),
            }
        }
    }

    #[test]
    fn a_failed_requirement_skips_what_needs_it() {
        let catalog = Catalog::built_in().unwrap();
        let steps = plan(
            &catalog,
            &platform(&[("agent", true), ("node-red", true), ("chirpstack", false)]),
        );
        let mut fake = Fake {
            fail: vec!["base"],
            ran: Vec::new(),
        };
        let mut reported = Vec::new();
        let results = run_steps(&catalog, &steps, &mut fake, |i, r| reported.push((i, r.line())));
        assert_eq!(
            fake.ran,
            ["common", "chirpstack", "base", "mosquitto", "node-red"]
        );
        let lines: Vec<&str> = reported.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(
            lines,
            [
                "common: up, 2 changes",
                "chirpstack: off, 2 changes",
                "base: failed: no network",
                "mosquitto: up, 2 changes",
                "agent: skipped: base did not come up",
                "node-red: up, 2 changes",
            ]
        );
        assert_eq!(results.len(), steps.len());
    }

    #[test]
    fn nothing_runs_when_common_fails() {
        let catalog = Catalog::built_in().unwrap();
        let steps = plan(&catalog, &platform(&[("mosquitto", true), ("node-red", false)]));
        let mut fake = Fake {
            fail: vec!["common"],
            ran: Vec::new(),
        };
        let results = run_steps(&catalog, &steps, &mut fake, |_, _| {});
        assert_eq!(fake.ran, ["common"]);
        assert!(results[1..].iter().all(|r| r.result == Outcome::Skipped));
    }

    #[test]
    fn what_ansible_prints() {
        let out = "PLAY RECAP ****\nlocalhost                  : ok=7    changed=3    unreachable=0    \
                   failed=0    skipped=1    rescued=0    ignored=0\n";
        assert_eq!(changes_in(out), Some(3));
        assert_eq!(changes_in("nothing here"), None);
        let failed = "TASK [x] ***\nfatal: [localhost]: FAILED! => {\"changed\": false, \"msg\": \"No package matching 'x' is available\"}\n";
        assert_eq!(
            failure_in(failed).as_deref(),
            Some("No package matching 'x' is available")
        );
        assert_eq!(
            failure_in("fatal: [localhost]: something odd").as_deref(),
            Some("fatal: [localhost]: something odd")
        );
    }

    #[test]
    fn dates() {
        assert_eq!(rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            rfc3339(UNIX_EPOCH + Duration::from_secs(1_794_952_515)),
            "2026-11-17T21:55:15Z"
        );
        assert_eq!(
            rfc3339(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29T00:00:00Z"
        );
    }

    #[test]
    fn the_avahi_record() {
        let status = Status {
            schema: 1,
            run: "1".into(),
            release: "0.1.0".into(),
            state: ApplyState::Running,
            step: 2,
            steps: 5,
            component: Some("mosquitto".into()),
            restart_needed: false,
            message: None,
            results: Vec::new(),
        };
        let text = avahi_text(&status);
        assert!(text.contains("<type>_fiber._tcp</type>"));
        for record in [
            "state=running",
            "step=2/5",
            "component=mosquitto",
            "release=0.1.0",
            "restart=no",
        ] {
            assert!(
                text.contains(&format!("<txt-record>{record}</txt-record>")),
                "{record}"
            );
        }
    }
}
