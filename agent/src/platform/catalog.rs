//! The components of the platform, from `provision/components.yaml`.
//!
//! The file is compiled into the program, so the program and its list of components can never
//! disagree. The archive carries the same file for FIBER Setup's picker.

use std::fmt;
use std::marker::PhantomData;

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use serde::de::{MapAccess, Visitor};

/// `provision/components.yaml`, as it was when this program was built.
const COMPONENTS_YAML: &str = include_str!("../../../provision/components.yaml");

/// The schema of `components.yaml` this program reads.
const SCHEMA: u32 = 1;

/// Every component, in the order of the file.
#[derive(Debug)]
pub struct Catalog {
    components: Vec<Component>,
}

/// One component.
#[derive(Debug)]
pub struct Component {
    pub id: String,
    pub title: String,
    pub requires: Vec<String>,
    /// The Ansible roles that bring it up, in order.
    pub roles: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema: u32,
    components: Ordered<Fields>,
}

// Every field of the file is read, so that a mistake in it fails the build's tests, even where
// the program does not use the field yet: the roles (T6), the services, ports and sizes (FIBER
// Setup, T12).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct Fields {
    title: String,
    about: String,
    roles: Vec<String>,
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    suggests: Vec<String>,
    #[serde(default)]
    services: Vec<String>,
    #[serde(default)]
    ports: Vec<Port>,
    #[serde(default)]
    size: Option<Size>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct Port {
    port: u16,
    listen: Listen,
    #[serde(default)]
    when: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
enum Listen {
    Loopback,
    Network,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct Size {
    download: Option<u32>,
    installed: Option<u32>,
}

/// A YAML map, read with its entries in the order of the file.
struct Ordered<V>(Vec<(String, V)>);

impl<'de, V: Deserialize<'de>> Deserialize<'de> for Ordered<V> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct OrderedVisitor<V>(PhantomData<V>);
        impl<'de, V: Deserialize<'de>> Visitor<'de> for OrderedVisitor<V> {
            type Value = Ordered<V>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a map")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(Ordered(entries))
            }
        }
        deserializer.deserialize_map(OrderedVisitor(PhantomData))
    }
}

impl Catalog {
    /// The components this program was built with.
    pub fn built_in() -> Result<Catalog> {
        Self::parse(COMPONENTS_YAML)
    }

    /// Reads and checks a `components.yaml`: the schema, that every component it names exists,
    /// and that no component requires itself, directly or through others.
    fn parse(text: &str) -> Result<Catalog> {
        let file: File = serde_saphyr::from_str_with_options(
            text,
            serde_saphyr::options! {
                strict_booleans: true,
            },
        )
        .map_err(|e| anyhow!("components.yaml: {}", e.without_snippet()))?;
        if file.schema != SCHEMA {
            bail!(
                "components.yaml has schema {}; this program reads schema {SCHEMA}",
                file.schema
            );
        }
        let catalog = Catalog {
            components: file
                .components
                .0
                .into_iter()
                .map(|(id, fields)| Component {
                    id,
                    title: fields.title,
                    requires: fields.requires,
                    roles: fields.roles,
                })
                .collect(),
        };
        let mut seen = Vec::new();
        for component in &catalog.components {
            if seen.contains(&component.id.as_str()) {
                bail!("components.yaml names `{}` twice", component.id);
            }
            seen.push(&component.id);
            for other in &component.requires {
                if catalog.get(other).is_none() {
                    bail!(
                        "components.yaml: `{}` requires `{other}`, which is not a component",
                        component.id
                    );
                }
            }
            if catalog
                .requirements(&component.id)
                .iter()
                .any(|r| r == &component.id)
            {
                bail!("components.yaml: `{}` requires itself", component.id);
            }
        }
        Ok(catalog)
    }

    /// Every component, in the order of the file.
    pub fn components(&self) -> &[Component] {
        &self.components
    }

    pub fn get(&self, id: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.id == id)
    }

    /// The ids, for a message: "base, mosquitto, agent, node-red and chirpstack".
    pub fn ids_in_words(&self) -> String {
        in_words(self.components.iter().map(|c| c.id.as_str()))
    }

    /// Everything `id` requires, directly or through others, in the order of the file.
    pub fn requirements(&self, id: &str) -> Vec<String> {
        let mut found: Vec<&str> = Vec::new();
        let mut todo = vec![id];
        while let Some(next) = todo.pop() {
            for required in self.get(next).map(|c| c.requires.as_slice()).unwrap_or_default() {
                if !found.contains(&required.as_str()) {
                    found.push(required);
                    todo.push(required);
                }
            }
        }
        self.components
            .iter()
            .map(|c| c.id.clone())
            .filter(|c| found.contains(&c.as_str()))
            .collect()
    }
}

/// "a", "a and b", "a, b and c".
pub fn in_words<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let items: Vec<&str> = items.into_iter().collect();
    match items.split_last() {
        None => String::new(),
        Some((last, [])) => (*last).to_owned(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_file_is_good() {
        let catalog = Catalog::built_in().unwrap();
        let ids: Vec<&str> = catalog.components().iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["base", "mosquitto", "agent", "node-red", "chirpstack"]);
        assert_eq!(
            catalog.ids_in_words(),
            "base, mosquitto, agent, node-red and chirpstack"
        );
    }

    #[test]
    fn requirements_as_the_plan_has_them() {
        let catalog = Catalog::built_in().unwrap();
        assert_eq!(catalog.requirements("base"), Vec::<String>::new());
        assert_eq!(catalog.requirements("agent"), ["base", "mosquitto"]);
        assert_eq!(catalog.requirements("node-red"), ["mosquitto"]);
        assert_eq!(catalog.requirements("chirpstack"), ["mosquitto"]);
    }

    const HEAD: &str = "schema: 1\ncomponents:\n";

    fn component(id: &str, requires: &str) -> String {
        format!("  {id}:\n    title: T\n    about: A.\n    roles: [r]\n    requires: [{requires}]\n")
    }

    #[test]
    fn requirements_through_others() {
        let text = format!(
            "{HEAD}{}{}{}",
            component("c", "b"),
            component("a", ""),
            component("b", "a")
        );
        let catalog = Catalog::parse(&text).unwrap();
        // In the order of the file, not in the order they were found.
        assert_eq!(catalog.requirements("c"), ["a", "b"]);
    }

    #[test]
    fn mistakes_in_the_file() {
        let unknown = format!("{HEAD}{}", component("a", "x"));
        let circle = format!("{HEAD}{}{}", component("a", "b"), component("b", "a"));
        let twice = format!("{HEAD}{}{}", component("a", ""), component("a", ""));
        let schema = format!("schema: 2\ncomponents:\n{}", component("a", ""));
        let field = format!("{HEAD}{}    colour: red\n", component("a", ""));
        assert!(
            Catalog::parse(&unknown)
                .unwrap_err()
                .to_string()
                .contains("`x`, which is not a component")
        );
        assert!(
            Catalog::parse(&circle)
                .unwrap_err()
                .to_string()
                .contains("`a` requires itself")
        );
        assert!(Catalog::parse(&twice).is_err());
        assert!(
            Catalog::parse(&schema)
                .unwrap_err()
                .to_string()
                .contains("schema 2")
        );
        assert!(Catalog::parse(&field).unwrap_err().to_string().contains("colour"));
    }

    #[test]
    fn lists_in_words() {
        assert_eq!(in_words([]), "");
        assert_eq!(in_words(["a"]), "a");
        assert_eq!(in_words(["a", "b"]), "a and b");
        assert_eq!(in_words(["a", "b", "c"]), "a, b and c");
    }
}
