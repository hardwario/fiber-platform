//! `/etc/fiber/platform.yaml`: which components are wanted, and the settings.
//!
//! ```yaml
//! schema: 1
//! components:        # true: bring it up; false: stop and disable it, keep its data;
//!   base: true       # not named: leave it alone
//!   mosquitto: true
//! settings:
//!   mqtt: { lan: false }
//! ```
//!
//! No file, or a file with nothing but comments in it, means that nothing is chosen. A person may
//! edit the file; `fiber platform enable` and `disable` change only the lines under `components:`
//! ([`super::edit`]).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Result, anyhow, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::catalog::Catalog;
use super::edit;

/// The schema of `platform.yaml` this program reads and writes.
pub const SCHEMA: u32 = 1;

/// What the file says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Platform {
    /// `true` or `false` for each component the file names.
    pub components: BTreeMap<String, bool>,
    pub settings: Settings,
}

/// The settings, each with its default.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Settings {
    pub mqtt: Mqtt,
    pub lorawan: Lorawan,
    pub hardware: Hardware,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Mqtt {
    /// Also listen on port 1884 for the network, with a login.
    pub lan: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Lorawan {
    pub region: Region,
}

/// The LoRaWAN regions the `chirpstack` role can set up.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    #[default]
    Eu868,
}

impl Region {
    /// As the file writes it.
    pub fn name(self) -> &'static str {
        match self {
            Region::Eu868 => "eu868",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Hardware {
    pub minipcie: MiniPcie,
}

/// The card in the mini-PCIe slot.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MiniPcie {
    #[default]
    None,
    Lte,
    Rak5146,
}

impl MiniPcie {
    /// As the file writes it.
    pub fn name(self) -> &'static str {
        match self {
            MiniPcie::None => "none",
            MiniPcie::Lte => "lte",
            MiniPcie::Rak5146 => "rak5146",
        }
    }
}

#[derive(Deserialize)]
struct SchemaOnly {
    schema: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    #[allow(dead_code)]
    schema: u32,
    #[serde(default)]
    components: Option<BTreeMap<String, bool>>,
    #[serde(default)]
    settings: Option<Settings>,
}

impl Platform {
    /// Reads the file at `path`. No file means nothing is chosen.
    pub fn read(path: &Path, catalog: &Catalog) -> Result<Platform> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, path, catalog),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Platform::default()),
            Err(e) => Err(anyhow!("cannot read {}: {e}", path.display())),
        }
    }

    /// Reads and checks the text of the file; `path` is only for the messages.
    pub fn parse(text: &str, path: &Path, catalog: &Catalog) -> Result<Platform> {
        if is_empty(text) {
            return Ok(Platform::default());
        }
        let name = path.display().to_string();
        let document: Document = read_yaml(text, &name)?;
        let platform = Platform {
            components: document.components.unwrap_or_default(),
            settings: document.settings.unwrap_or_default(),
        };
        platform.check(text, &name, catalog)?;
        Ok(platform)
    }

    /// Every component the file names exists, and no component that is on requires one that is
    /// off. `text` is where the components were read from, for the lines in the messages.
    pub(super) fn check(&self, text: &str, name: &str, catalog: &Catalog) -> Result<()> {
        let at = |id: &str| match edit::line_of(text, id) {
            Some(line) => format!("{name}, line {line}"),
            None => name.to_owned(),
        };
        for id in self.components.keys() {
            if catalog.get(id).is_none() {
                bail!(
                    "{}: there is no component `{id}`. The components are {}",
                    at(id),
                    catalog.ids_in_words()
                );
            }
        }
        for (id, _) in self.components.iter().filter(|(_, on)| **on) {
            for required in catalog.requirements(id) {
                if self.components.get(&required) == Some(&false) {
                    bail!(
                        "{}: `{id}` is on, but `{required}`, which it requires, is off. \
                         Turn `{required}` on, or `{id}` off",
                        at(id)
                    );
                }
            }
        }
        Ok(())
    }
}

/// Reads a YAML file of this schema (`platform.yaml`, the seed) into `T`, with the messages a
/// person can act on: the file, the line, and the line itself.
pub(super) fn read_yaml<T: DeserializeOwned>(text: &str, name: &str) -> Result<T> {
    let yaml_error = |e: serde_saphyr::Error| {
        let rendered = e.render_with_options(serde_saphyr::render_options! {
            formatter: &serde_saphyr::UserMessageFormatter,
            source_name: Some(name),
        });
        anyhow!("{name}: {}", rendered.trim_start_matches("error: "))
    };
    let options = || serde_saphyr::options! { strict_booleans: true };

    // The schema first, alone: a file for another schema may have fields this program does not
    // know, and the message should be about the schema, not about the fields.
    let SchemaOnly { schema } = serde_saphyr::from_str_with_options(text, options()).map_err(yaml_error)?;
    match schema {
        Some(SCHEMA) => {}
        Some(other) => bail!(
            "{name} is written for schema {other}; this fiber program reads schema {SCHEMA}. \
             It belongs to another release of the platform"
        ),
        None => bail!("{name} has no `schema: {SCHEMA}` line"),
    }
    serde_saphyr::from_str_with_options(text, options()).map_err(yaml_error)
}

/// Nothing but blank lines and comments.
pub(super) fn is_empty(text: &str) -> bool {
    text.lines().all(|line| {
        let line = line.trim();
        line.is_empty() || line.starts_with('#') || line == "---"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Platform> {
        Platform::parse(
            text,
            Path::new("/etc/fiber/platform.yaml"),
            &Catalog::built_in().unwrap(),
        )
    }

    fn error(text: &str) -> String {
        format!("{:#}", parse(text).unwrap_err())
    }

    #[test]
    fn a_whole_file() {
        let platform = parse(
            "schema: 1\ncomponents:\n  base: true\n  node-red: false\nsettings:\n  mqtt:\n    lan: true\n  \
             lorawan:\n    region: eu868\n  hardware:\n    minipcie: rak5146\n",
        )
        .unwrap();
        assert_eq!(platform.components.get("base"), Some(&true));
        assert_eq!(platform.components.get("node-red"), Some(&false));
        assert_eq!(platform.components.get("agent"), None);
        assert!(platform.settings.mqtt.lan);
        assert_eq!(platform.settings.hardware.minipcie, MiniPcie::Rak5146);
    }

    #[test]
    fn nothing_chosen() {
        assert_eq!(parse("").unwrap(), Platform::default());
        assert_eq!(parse("# a comment\n\n").unwrap(), Platform::default());
        assert_eq!(parse("schema: 1\n").unwrap(), Platform::default());
        assert_eq!(
            parse("schema: 1\ncomponents:\nsettings:\n").unwrap(),
            Platform::default()
        );
        assert_eq!(parse("schema: 1\ncomponents: {}\n").unwrap(), Platform::default());
    }

    #[test]
    fn settings_have_defaults() {
        let platform = parse("schema: 1\nsettings:\n  mqtt:\n    lan: true\n").unwrap();
        assert!(platform.settings.mqtt.lan);
        assert_eq!(platform.settings.lorawan.region, Region::Eu868);
        assert_eq!(platform.settings.hardware.minipcie, MiniPcie::None);
    }

    #[test]
    fn an_unknown_component_is_named_with_its_line() {
        let e = error("schema: 1\ncomponents:\n  base: true\n  nodered: true\n");
        assert!(
            e.starts_with("/etc/fiber/platform.yaml, line 4: there is no component `nodered`."),
            "{e}"
        );
        assert!(
            e.ends_with("The components are base, mosquitto, agent, node-red and chirpstack"),
            "{e}"
        );
    }

    #[test]
    fn a_wrong_value_says_what_is_allowed() {
        let e = error("schema: 1\ncomponents:\n  base: yes\n");
        assert!(
            e.contains("line 3 column 9") && e.contains("true or false"),
            "{e}"
        );
        let e = error("schema: 1\nsettings:\n  lorawan:\n    region: us915\n");
        assert!(
            e.contains("line 4") && e.contains("`us915`") && e.contains("eu868"),
            "{e}"
        );
        let e = error("schema: 1\nsettings:\n  hardware:\n    minipcie: wifi\n");
        assert!(e.contains("none, lte, rak5146"), "{e}");
        let e = error("schema: 1\nsettings:\n  mqtt:\n    lann: true\n");
        assert!(e.contains("line 4") && e.contains("`lann`"), "{e}");
        let e = error("schema: 1\ncomponent:\n  base: true\n");
        assert!(e.contains("`component`") && e.contains("components"), "{e}");
    }

    #[test]
    fn the_message_shows_the_file() {
        let e = error("schema: 1\ncomponents:\n  base: maybe\n");
        assert!(e.contains("--> /etc/fiber/platform.yaml:3:9"), "{e}");
        assert!(e.contains("base: maybe"), "{e}");
    }

    #[test]
    fn the_schema() {
        assert!(error("components:\n  base: true\n").contains("no `schema: 1` line"));
        let e = error("schema: 2\ncomponents:\n  base: true\nnew_things: {}\n");
        assert!(
            e.contains("written for schema 2; this fiber program reads schema 1"),
            "{e}"
        );
    }

    #[test]
    fn a_component_that_is_on_needs_what_it_requires() {
        let e = error("schema: 1\ncomponents:\n  mosquitto: false\n  agent: true\n");
        assert_eq!(
            e,
            "/etc/fiber/platform.yaml, line 4: `agent` is on, but `mosquitto`, which it requires, is off. \
             Turn `mosquitto` on, or `agent` off"
        );
        // Not named is not off: `agent` brings up `base` and `mosquitto` (`resolve`).
        assert!(parse("schema: 1\ncomponents:\n  agent: true\n").is_ok());
    }

    #[test]
    fn a_name_twice_is_refused() {
        assert!(error("schema: 1\ncomponents:\n  base: true\n  base: false\n").contains("`base`"));
    }
}
