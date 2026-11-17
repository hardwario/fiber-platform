//! `fiber platform`: the components of the FIBER platform, and the file that chooses them.
//!
//! The commands are written down in `docs/cli.md`.

mod apply;
pub mod catalog;
mod edit;
pub mod file;
pub mod results;
mod seed;

pub use apply::{apply, foreground as apply_foreground};
pub use seed::seed;

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Result, anyhow, bail};

use crate::paths::Paths;
use crate::release::Release;
use catalog::{Catalog, in_words};
use file::Platform;
use results::{Outcome, Results, Wanted};

/// `fiber platform status` when a component that is on or off has not been brought to it.
pub const EXIT_NOT_APPLIED: u8 = 3;

/// Where a component stands in `platform.yaml`.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// `true`.
    On,
    /// Not named, but brought up because these components, which are on, require it.
    Needed(Vec<String>),
    /// `false`.
    Off,
    /// Not named, and nothing that is on requires it.
    LeftAlone,
}

impl State {
    fn wanted(&self) -> Option<Wanted> {
        match self {
            State::On | State::Needed(_) => Some(Wanted::On),
            State::Off => Some(Wanted::Off),
            State::LeftAlone => None,
        }
    }

    fn words(&self) -> String {
        match self {
            State::On => "on".to_owned(),
            State::Needed(by) => format!("on, for {}", in_words(by.iter().map(String::as_str))),
            State::Off => "off".to_owned(),
            State::LeftAlone => "left alone".to_owned(),
        }
    }
}

/// The state of every component, in the order of the catalog.
pub fn states<'a>(catalog: &'a Catalog, platform: &Platform) -> Vec<(&'a str, State)> {
    catalog
        .components()
        .iter()
        .map(|component| {
            let id = component.id.as_str();
            let state = match platform.components.get(id) {
                Some(true) => State::On,
                Some(false) => State::Off,
                None => {
                    let by = needed_by(catalog, platform, id, &[]);
                    if by.is_empty() {
                        State::LeftAlone
                    } else {
                        State::Needed(by)
                    }
                }
            };
            (id, state)
        })
        .collect()
}

/// The components that are on, are not among `leaving`, and require `id`.
fn needed_by(catalog: &Catalog, platform: &Platform, id: &str, leaving: &[String]) -> Vec<String> {
    catalog
        .components()
        .iter()
        .map(|c| &c.id)
        .filter(|c| platform.components.get(*c) == Some(&true) && !leaving.contains(c))
        .filter(|c| catalog.requirements(c).iter().any(|r| r == id))
        .cloned()
        .collect()
}

/// Rows of text in columns, two spaces apart.
fn table(rows: &[Vec<String>]) -> String {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|i| {
            rows.iter()
                .filter_map(|r| r.get(i))
                .map(|c| c.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for row in rows {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            line.push_str(cell);
            if i + 1 < row.len() {
                line.push_str(&" ".repeat(widths[i] - cell.chars().count() + 2));
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// What `fiber platform list` prints.
pub fn list_text(catalog: &Catalog, platform: &Platform) -> String {
    let mut rows = vec![vec![
        "COMPONENT".into(),
        "STATE".into(),
        "REQUIRES".into(),
        "WHAT".into(),
    ]];
    for (id, state) in states(catalog, platform) {
        let requires = catalog.get(id).map(|c| c.requires.join(", ")).unwrap_or_default();
        let title = catalog.get(id).map(|c| c.title.clone()).unwrap_or_default();
        rows.push(vec![id.to_owned(), state.words(), requires, title]);
    }
    table(&rows)
}

/// What `fiber platform status` prints, and whether everything that is on or off has been
/// brought there.
pub fn status_text(
    catalog: &Catalog,
    platform: &Platform,
    results: &Results,
    release: Option<&Release>,
    pending: bool,
) -> (String, bool) {
    let mut done = !pending;
    let mut out = String::new();
    let release = match release {
        Some(r) => format!("{}, in {}", r.version, r.dir.display()),
        None => "none: this fiber program is not inside a release".to_owned(),
    };
    let apply = if pending {
        "waiting: an apply was asked for and has not finished"
    } else {
        "nothing waiting"
    };
    out.push_str(&table(&[
        vec!["Release".into(), release],
        vec!["Apply".into(), apply.into()],
    ]));
    out.push('\n');

    let mut rows = vec![vec!["COMPONENT".into(), "STATE".into(), "LAST RESULT".into()]];
    for (id, state) in states(catalog, platform) {
        let last = match (state.wanted(), results.components.get(id)) {
            (None, _) => "-".to_owned(),
            (Some(wanted), Some(r)) if r.wanted == wanted => {
                done &= r.result == Outcome::Ok;
                let outcome = match r.result {
                    Outcome::Ok => "ok",
                    Outcome::Failed => "failed",
                    Outcome::Skipped => "skipped",
                };
                match &r.message {
                    Some(m) => format!("{outcome}, {}: {m}", r.finished),
                    None => format!("{outcome}, {}", r.finished),
                }
            }
            (Some(_), _) => {
                done = false;
                "not applied yet".to_owned()
            }
        };
        rows.push(vec![id.to_owned(), state.words(), last]);
    }
    out.push_str(&table(&rows));
    (out, done)
}

/// The first lines of a `platform.yaml` that `fiber platform` makes.
const NEW_FILE_HEAD: &str = "\
# `fiber platform enable` and `disable` change only the lines under `components:`; the rest of this file, comments too, stays as it is.
#
# What the FIBER platform brings up. A component that is true is brought up; one that is false
# is stopped and turned off, and its data stays; one that is not named is left alone.
# `fiber platform list` shows them all.
schema: 1
components:
";

/// The `settings:` block of a new `platform.yaml`, with a comment on each line.
fn settings_text(settings: &file::Settings) -> String {
    let line = |text: String, comment: &str| format!("{text:<22}# {comment}\n");
    [
        "settings:\n".to_owned(),
        "  mqtt:\n".to_owned(),
        line(
            format!("    lan: {}", settings.mqtt.lan),
            "true: also listen on port 1884 for the network, with a login",
        ),
        "  lorawan:\n".to_owned(),
        line(
            format!("    region: {}", settings.lorawan.region.name()),
            "the only region so far",
        ),
        "  hardware:\n".to_owned(),
        line(
            format!("    minipcie: {}", settings.hardware.minipcie.name()),
            "the card in the mini-PCIe slot: none, lte or rak5146",
        ),
    ]
    .concat()
}

/// A new `platform.yaml` with these components and settings.
pub fn new_file(components: &[(String, bool)], settings: &file::Settings) -> String {
    let mut text = NEW_FILE_HEAD.to_owned();
    for (id, on) in components {
        text.push_str(&format!("  {id}: {on}\n"));
    }
    text.push_str(&settings_text(settings));
    text
}

/// Checks that every id is a component, and drops repeats.
fn known(catalog: &Catalog, ids: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        if catalog.get(id).is_none() {
            bail!(
                "there is no component `{id}`. The components are {}",
                catalog.ids_in_words()
            );
        }
        if !out.contains(id) {
            out.push(id.clone());
        }
    }
    Ok(out)
}

/// What `enable` changes: the components and what they require, in the order of the catalog,
/// each with the line it prints.
fn enable_changes(catalog: &Catalog, platform: &Platform, ids: &[String]) -> Vec<(String, String)> {
    let mut changes = Vec::new();
    for component in catalog.components() {
        let id = &component.id;
        if platform.components.get(id) == Some(&true) {
            continue;
        }
        if ids.contains(id) {
            changes.push((id.clone(), format!("{id}: on")));
        } else {
            let by: Vec<&str> = ids
                .iter()
                .filter(|asked| catalog.requirements(asked).contains(id))
                .map(String::as_str)
                .collect();
            if !by.is_empty() {
                let verb = if by.len() == 1 { "requires" } else { "require" };
                changes.push((id.clone(), format!("{id}: on, {} {verb} it", in_words(by))));
            }
        }
    }
    changes
}

/// What `disable` changes, or why it is refused.
fn disable_changes(catalog: &Catalog, platform: &Platform, ids: &[String]) -> Result<Vec<(String, String)>> {
    for id in ids {
        let by = needed_by(catalog, platform, id, ids);
        if !by.is_empty() {
            let verb = if by.len() == 1 { "is" } else { "are" };
            let mut all: Vec<&str> = by.iter().map(String::as_str).collect();
            all.extend(ids.iter().map(String::as_str));
            bail!(
                "{id} is needed by {}, which {verb} on. Turn {} off with it: fiber platform disable {}",
                in_words(by.iter().map(String::as_str)),
                if by.len() == 1 { "it" } else { "them" },
                all.join(" ")
            );
        }
    }
    Ok(catalog
        .components()
        .iter()
        .map(|c| &c.id)
        .filter(|id| ids.contains(id) && platform.components.get(*id) != Some(&false))
        .map(|id| (id.clone(), format!("{id}: off")))
        .collect())
}

/// `fiber platform list`.
pub fn list(paths: &Paths) -> Result<ExitCode> {
    let catalog = Catalog::built_in()?;
    let platform = Platform::read(&paths.platform_file(), &catalog)?;
    print!("{}", list_text(&catalog, &platform));
    if !paths.platform_file().exists() {
        println!(
            "\nThere is no {} yet: nothing is chosen.",
            paths.platform_file().display()
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// `fiber platform status`.
pub fn status(paths: &Paths) -> Result<ExitCode> {
    let catalog = Catalog::built_in()?;
    let platform = Platform::read(&paths.platform_file(), &catalog)?;
    let results = Results::read(&paths.platform_results())?;
    let release = Release::of_this_program();
    let pending = paths.platform_pending().exists();
    let (text, done) = status_text(&catalog, &platform, &results, release.as_ref(), pending);
    print!("{text}");
    Ok(if done {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_NOT_APPLIED)
    })
}

/// `fiber platform enable <component>...` and `disable <component>...`.
pub fn change(paths: &Paths, ids: &[String], on: bool) -> Result<ExitCode> {
    let catalog = Catalog::built_in()?;
    let ids = known(&catalog, ids)?;
    let path = paths.platform_file();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(anyhow!("cannot read {}: {e}", path.display())),
    };
    let platform = match &text {
        Some(text) => Platform::parse(text, &path, &catalog)?,
        None => Platform::default(),
    };

    let changes = if on {
        enable_changes(&catalog, &platform, &ids)
    } else {
        disable_changes(&catalog, &platform, &ids)?
    };
    if changes.is_empty() {
        let state = if on { "on" } else { "off" };
        let verb = if ids.len() == 1 { "is" } else { "are" };
        println!(
            "{} {verb} {state} already. Nothing changed.",
            in_words(ids.iter().map(String::as_str))
        );
        return Ok(ExitCode::SUCCESS);
    }

    let set: Vec<(String, bool)> = changes.iter().map(|(id, _)| (id.clone(), on)).collect();
    let new_text = match &text {
        Some(text) => edit::set(text, &set).map_err(|e| {
            anyhow!(
                "{}, {e}. Change the file by hand, or write the components one per line",
                path.display()
            )
        })?,
        None => new_file(&set, &file::Settings::default()),
    };

    // Read the new text back: it must say exactly what was asked, and nothing else may change.
    let mut expected = platform.clone();
    expected.components.extend(set.iter().cloned());
    let check = Platform::parse(&new_text, &path, &catalog);
    if check.as_ref().ok() != Some(&expected) {
        bail!(
            "could not change {} safely, so it was not written. Change it by hand",
            path.display()
        );
    }

    let command = format!("{} {}", if on { "enable" } else { "disable" }, ids.join(" "));
    write_file(&path, &new_text, 0o644).map_err(|e| write_error(&path, &e, &command))?;
    for (_, line) in &changes {
        println!("{line}");
    }
    println!(
        "Changed {}. Bring the FIBER to it with: sudo fiber platform apply",
        path.display()
    );
    Ok(ExitCode::SUCCESS)
}

/// The message for a file that could not be written; `command` is what to run again as root.
pub(super) fn write_error(path: &Path, e: &std::io::Error, command: &str) -> anyhow::Error {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => anyhow!(
            "cannot write {}: permission denied. Run it as root: sudo fiber platform {command}",
            path.display()
        ),
        _ => anyhow!("cannot write {}: {e}", path.display()),
    }
}

/// Writes `text` to `path` in one step: into a new file beside it, which then takes its place.
/// A reader sees the old file or the new one, never half of it. The new file keeps the old one's
/// owner and permissions; a file that was not there gets `mode`.
pub(super) fn write_file(path: &Path, text: &str, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let new = dir.join(format!(".{name}.new"));
    let _ = std::fs::remove_file(&new);
    let result = (|| {
        // Created with its final mode, so that a secret is never readable by others, not even
        // for a moment.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&new)?;
        match std::fs::metadata(path) {
            Ok(old) => {
                file.set_permissions(old.permissions())?;
                std::os::unix::fs::fchown(&file, Some(old.uid()), Some(old.gid()))?;
            }
            // The mode given to `open` is cut by the umask; set it as it was asked.
            Err(_) => file.set_permissions(std::fs::Permissions::from_mode(mode))?,
        }
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&new, path)?;
        std::fs::File::open(dir)?.sync_all()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&new);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use results::ComponentResult;
    use std::collections::BTreeMap;

    fn map(pairs: &[(&str, bool)]) -> BTreeMap<String, bool> {
        pairs.iter().map(|(id, on)| ((*id).to_owned(), *on)).collect()
    }

    fn platform(pairs: &[(&str, bool)]) -> Platform {
        Platform {
            components: map(pairs),
            ..Platform::default()
        }
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn states_of_the_components() {
        let catalog = Catalog::built_in().unwrap();
        let states = states(
            &catalog,
            &platform(&[("agent", true), ("node-red", true), ("chirpstack", false)]),
        );
        assert_eq!(
            states,
            [
                ("base", State::Needed(ids(&["agent"]))),
                ("mosquitto", State::Needed(ids(&["agent", "node-red"]))),
                ("agent", State::On),
                ("node-red", State::On),
                ("chirpstack", State::Off),
            ]
        );
        assert_eq!(states[1].1.words(), "on, for agent and node-red");
    }

    #[test]
    fn the_list() {
        let catalog = Catalog::built_in().unwrap();
        let text = list_text(&catalog, &platform(&[("agent", true), ("chirpstack", false)]));
        let expected = "\
COMPONENT   STATE          REQUIRES         WHAT
base        on, for agent                   Basic system configuration
mosquitto   on, for agent                   Mosquitto
agent       on             base, mosquitto  FIBER agent
node-red    left alone     mosquitto        Node-RED
chirpstack  off            mosquitto        ChirpStack
";
        assert_eq!(text, expected);
    }

    #[test]
    fn enabling_brings_what_is_required() {
        let catalog = Catalog::built_in().unwrap();
        let changes = enable_changes(
            &catalog,
            &platform(&[("base", true)]),
            &ids(&["agent", "node-red"]),
        );
        let lines: Vec<&str> = changes.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(
            lines,
            [
                "mosquitto: on, agent and node-red require it",
                "agent: on",
                "node-red: on"
            ]
        );
        assert!(enable_changes(&catalog, &platform(&[("base", true)]), &ids(&["base"])).is_empty());
    }

    #[test]
    fn disabling_what_another_needs_is_refused() {
        let catalog = Catalog::built_in().unwrap();
        let p = platform(&[("agent", true), ("node-red", true)]);
        let e = disable_changes(&catalog, &p, &ids(&["mosquitto"]))
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "mosquitto is needed by agent and node-red, which are on. \
             Turn them off with it: fiber platform disable agent node-red mosquitto"
        );
        // Together it is allowed; `base` stays as it is.
        let changes = disable_changes(&catalog, &p, &ids(&["mosquitto", "node-red", "agent"])).unwrap();
        let lines: Vec<&str> = changes.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(lines, ["mosquitto: off", "agent: off", "node-red: off"]);
    }

    #[test]
    fn a_new_file_reads_back() {
        let catalog = Catalog::built_in().unwrap();
        let text = new_file(
            &[("base".to_owned(), true), ("node-red".to_owned(), false)],
            &file::Settings::default(),
        );
        let platform = Platform::parse(&text, Path::new("platform.yaml"), &catalog).unwrap();
        assert_eq!(platform.components, map(&[("base", true), ("node-red", false)]));
        assert_eq!(platform.settings, file::Settings::default());
        assert!(
            text.lines()
                .next()
                .unwrap()
                .contains("change only the lines under `components:`")
        );
    }

    fn result(wanted: Wanted, result: Outcome, message: Option<&str>) -> ComponentResult {
        ComponentResult {
            wanted,
            result,
            finished: "2026-11-17T21:04:05Z".to_owned(),
            message: message.map(str::to_owned),
        }
    }

    #[test]
    fn the_status() {
        let catalog = Catalog::built_in().unwrap();
        let p = platform(&[("agent", true), ("node-red", false)]);
        let mut results = Results {
            schema: results::SCHEMA,
            components: BTreeMap::new(),
        };
        results
            .components
            .insert("base".into(), result(Wanted::On, Outcome::Ok, None));
        results.components.insert(
            "mosquitto".into(),
            result(Wanted::On, Outcome::Failed, Some("no network")),
        );
        results
            .components
            .insert("node-red".into(), result(Wanted::On, Outcome::Ok, None));
        let release = Release {
            dir: "/opt/fiber/releases/1.2.0".into(),
            version: "1.2.0".into(),
        };
        let (text, done) = status_text(&catalog, &p, &results, Some(&release), false);
        let expected = "\
Release  1.2.0, in /opt/fiber/releases/1.2.0
Apply    nothing waiting

COMPONENT   STATE          LAST RESULT
base        on, for agent  ok, 2026-11-17T21:04:05Z
mosquitto   on, for agent  failed, 2026-11-17T21:04:05Z: no network
agent       on             not applied yet
node-red    off            not applied yet
chirpstack  left alone     -
";
        assert_eq!(text, expected);
        assert!(!done);
    }

    #[test]
    fn the_status_when_everything_is_applied() {
        let catalog = Catalog::built_in().unwrap();
        let p = platform(&[("mosquitto", true), ("chirpstack", false)]);
        let mut results = Results {
            schema: results::SCHEMA,
            components: BTreeMap::new(),
        };
        results
            .components
            .insert("mosquitto".into(), result(Wanted::On, Outcome::Ok, None));
        results
            .components
            .insert("chirpstack".into(), result(Wanted::Off, Outcome::Ok, None));
        assert!(status_text(&catalog, &p, &results, None, false).1);
        // Not while an apply is waiting.
        assert!(!status_text(&catalog, &p, &results, None, true).1);
    }

    #[test]
    fn columns() {
        let rows = vec![
            vec!["a".to_owned(), "bb".to_owned(), "".to_owned()],
            vec!["ccc".to_owned(), "d".to_owned()],
        ];
        assert_eq!(table(&rows), "a    bb\nccc  d\n");
    }
}
