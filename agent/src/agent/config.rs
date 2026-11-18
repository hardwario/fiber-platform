//! `/etc/fiber/agent.toml`: the agent's settings. It needs none; each has a default.
//!
//! ```toml
//! [mqtt]
//! host = "127.0.0.1"   # the broker on the FIBER itself
//! port = 1883
//!
//! [board]
//! dividers = true      # false on the board variant without the two voltage dividers
//! ```

use std::path::Path;

use anyhow::{Result, anyhow};
use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    pub mqtt: Mqtt,
    pub board: Board,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct Mqtt {
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct Board {
    pub dividers: bool,
}

impl Default for Mqtt {
    fn default() -> Self {
        Mqtt {
            host: "127.0.0.1".into(),
            port: 1883,
        }
    }
}

impl Default for Board {
    fn default() -> Self {
        Board { dividers: true }
    }
}

impl Config {
    /// The settings in `path`; the defaults when there is no such file.
    pub fn read(path: &Path) -> Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| anyhow!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(anyhow!("cannot read {}: {e}", path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_a_file() {
        let tmp = crate::testdir::TestDir::new("agent-config");
        assert_eq!(
            Config::read(&tmp.path().join("none.toml")).unwrap(),
            Config::default()
        );
        let path = tmp.path().join("agent.toml");
        std::fs::write(&path, "[board]\ndividers = false\n").unwrap();
        let config = Config::read(&path).unwrap();
        assert!(!config.board.dividers);
        assert_eq!(config.mqtt.port, 1883);
        std::fs::write(&path, "[board]\ndivider = false\n").unwrap();
        let error = Config::read(&path).unwrap_err().to_string();
        assert!(error.contains("divider"), "{error}");
    }

    /// The file the agent role writes: as it is, it changes nothing; with every setting taken out
    /// of its comment, it names every setting, each at its default.
    #[test]
    fn the_roles_file_holds_the_defaults() {
        let text = include_str!("../../../provision/ansible/roles/agent/files/agent.toml");
        assert_eq!(toml::from_str::<Config>(text).unwrap(), Config::default());
        let uncommented: String = text
            .lines()
            .filter_map(|line| line.strip_prefix("# "))
            .filter(|line| {
                line.starts_with('[')
                    || line
                        .split_once(" = ")
                        .is_some_and(|(key, _)| key.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            })
            .map(|line| format!("{line}\n"))
            .collect();
        let table: toml::Table = toml::from_str(&uncommented).unwrap();
        assert_eq!(table.len(), 2, "{uncommented}");
        assert_eq!(toml::from_str::<Config>(&uncommented).unwrap(), Config::default());
        assert_eq!(uncommented.matches(" = ").count(), 3, "{uncommented}");
    }
}
