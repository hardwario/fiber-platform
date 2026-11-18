//! `fiber platform seed <file>`: the choice made in FIBER Setup, handed over once.
//!
//! FIBER Setup writes `/boot/firmware/fiber-seed.yaml`: the components, the settings, and the user
//! name and password that also open the services (D16). The installer runs this command on it,
//! then overwrites the seed and deletes it.
//!
//! ```yaml
//! schema: 1
//! components: {base: true, mosquitto: true, agent: true}
//! settings: {mqtt: {lan: true}}
//! login: {user: pi, password: "the password"}
//! ```
//!
//! Both things it writes are written once. A `platform.yaml` that is there already is the
//! person's, and a password set once is never set again: one changed later stays changed.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::catalog::Catalog;
use super::file::{self, Platform, Settings};
use super::{new_file, write_error, write_file};
use crate::paths::Paths;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    #[allow(dead_code)]
    schema: u32,
    #[serde(default)]
    components: Option<BTreeMap<String, bool>>,
    #[serde(default)]
    settings: Option<Settings>,
    #[serde(default)]
    login: Option<Login>,
}

/// The user name and password for the services, as `/etc/fiber/secrets/login.json` keeps them.
#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Login {
    pub user: String,
    pub password: String,
}

/// What a seed says.
#[derive(Debug, PartialEq)]
pub struct Seed {
    pub platform: Platform,
    pub login: Option<Login>,
}

impl Seed {
    /// Reads and checks a seed, as strictly as `platform.yaml`; `path` is only for the messages.
    pub fn parse(text: &str, path: &Path, catalog: &Catalog) -> Result<Seed> {
        let name = path.display().to_string();
        if file::is_empty(text) {
            bail!("{name} is empty");
        }
        let document: Document = file::read_yaml(text, &name)?;
        let platform = Platform {
            components: document.components.unwrap_or_default(),
            settings: document.settings.unwrap_or_default(),
        };
        platform.check(text, &name, catalog)?;
        if let Some(login) = &document.login {
            if login.user.trim().is_empty() {
                bail!("{name}: the login has no user name");
            }
            if login.password.is_empty() {
                bail!("{name}: the login has no password");
            }
        }
        Ok(Seed {
            platform,
            login: document.login,
        })
    }
}

/// `fiber platform seed <file>`.
pub fn seed(paths: &Paths, seed_file: &Path) -> Result<ExitCode> {
    let catalog = Catalog::built_in()?;
    let text = std::fs::read_to_string(seed_file)
        .map_err(|e| anyhow!("cannot read {}: {e}", seed_file.display()))?;
    let seed = Seed::parse(&text, seed_file, &catalog)?;
    let command = format!("seed {}", seed_file.display());

    let platform_file = paths.platform_file();
    if platform_file.exists() {
        println!("Kept {}: it is there already.", platform_file.display());
    } else {
        // In the order of the catalog, as `enable` would write them.
        let components: Vec<(String, bool)> = catalog
            .components()
            .iter()
            .filter_map(|c| seed.platform.components.get(&c.id).map(|on| (c.id.clone(), *on)))
            .collect();
        let text = new_file(&components, &seed.platform.settings);
        write_file(&platform_file, &text, 0o644).map_err(|e| write_error(&platform_file, &e, &command))?;
        println!("Wrote {}.", platform_file.display());
    }

    if let Some(login) = &seed.login {
        let login_file = paths.login_file();
        if login_file.exists() {
            println!("Kept {}: the login is set once.", login_file.display());
        } else {
            let dir = paths.secrets_dir();
            make_private_dir(&dir).map_err(|e| write_error(&dir, &e, &command))?;
            let json = serde_json::to_string_pretty(login)? + "\n";
            write_file(&login_file, &json, 0o600).map_err(|e| write_error(&login_file, &e, &command))?;
            println!("Wrote {}.", login_file.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// A directory only its owner may enter.
fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::file::MiniPcie;

    fn parse(text: &str) -> Result<Seed> {
        Seed::parse(
            text,
            Path::new("/boot/firmware/fiber-seed.yaml"),
            &Catalog::built_in().unwrap(),
        )
    }

    #[test]
    fn a_whole_seed() {
        let seed = parse(
            "schema: 1\ncomponents: {base: true, mosquitto: true, agent: true}\n\
             settings: {hardware: {minipcie: lte}}\nlogin: {user: pi, password: \"a b: c\"}\n",
        )
        .unwrap();
        assert_eq!(seed.platform.components.len(), 3);
        assert_eq!(seed.platform.settings.hardware.minipcie, MiniPcie::Lte);
        assert_eq!(
            seed.login,
            Some(Login {
                user: "pi".into(),
                password: "a b: c".into()
            })
        );
    }

    /// A seed as FIBER Setup writes it (`firstboot.rs`, `seed()`): flow maps, and the user name
    /// and password as JSON strings, with what YAML would refuse or take for a line end written
    /// as an escape. The same bytes are in FIBER Setup's own test, so that the two cannot drift.
    #[test]
    fn a_seed_as_fiber_setup_writes_it() {
        let text = r#"# Written by FIBER Setup 1.3.0 for the FIBER platform 0.1.0. The installer reads it on the
# first start, then overwrites it with zeros and deletes it.
schema: 1
components: {base: true, mosquitto: true, agent: true, node-red: true, chirpstack: true}
settings: {mqtt: {lan: true}, hardware: {minipcie: rak5146}}
login: {user: "fiber", password: "a\"b\\c: #d\te\u0007\u007f\u0085\u2028\ufeff\ufffe Dílna 📶"}
"#;
        let seed = parse(text).unwrap();
        assert_eq!(seed.platform.components.len(), 5);
        assert!(seed.platform.settings.mqtt.lan);
        assert_eq!(seed.platform.settings.hardware.minipcie, MiniPcie::Rak5146);
        assert_eq!(
            seed.login,
            Some(Login {
                user: "fiber".into(),
                password: "a\"b\\c: #d\te\u{7}\u{7f}\u{85}\u{2028}\u{feff}\u{fffe} Dílna 📶".into()
            })
        );
    }

    #[test]
    fn checked_like_the_platform_file() {
        let e = format!(
            "{:#}",
            parse("schema: 1\ncomponents:\n  nodered: true\n").unwrap_err()
        );
        assert!(
            e.starts_with("/boot/firmware/fiber-seed.yaml, line 3: there is no component `nodered`"),
            "{e}"
        );
        let e = format!(
            "{:#}",
            parse("schema: 1\ncomponents:\n  mosquitto: false\n  agent: true\n").unwrap_err()
        );
        assert!(
            e.contains("`agent` is on, but `mosquitto`, which it requires, is off"),
            "{e}"
        );
        let e = format!("{:#}", parse("schema: 1\nlogin: {user: pi}\n").unwrap_err());
        assert!(e.contains("password"), "{e}");
        let e = format!(
            "{:#}",
            parse("schema: 1\nlogin: {user: '', password: x}\n").unwrap_err()
        );
        assert!(e.contains("no user name"), "{e}");
        assert!(format!("{:#}", parse("").unwrap_err()).contains("is empty"));
    }
}
