//! `/var/lib/fiber/platform/results.json`: the last result of each component.
//!
//! `fiber platform apply` (T6) writes it as each component finishes; `fiber platform status`
//! reads it. It survives a restart, unlike the progress of a running apply in `/run`.
//!
//! ```json
//! {"schema": 1, "components": {
//!   "base": {"wanted": "on", "result": "ok", "finished": "2026-11-17T21:04:05Z"},
//!   "node-red": {"wanted": "on", "result": "failed", "finished": "...", "message": "..."}}}
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

/// The schema of `results.json`.
pub const SCHEMA: u32 = 1;

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Results {
    pub schema: u32,
    pub components: BTreeMap<String, ComponentResult>,
}

/// How the last apply of one component ended.
#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct ComponentResult {
    /// What the apply brought the component to.
    pub wanted: Wanted,
    pub result: Outcome,
    /// When it finished, as RFC 3339 in UTC.
    pub finished: String,
    /// What went wrong, for `failed` and `skipped`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Wanted {
    On,
    Off,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Ok,
    Failed,
    /// Not tried, because a component it requires failed.
    Skipped,
}

impl Results {
    /// Reads the file at `path`. No file means no component has been applied yet.
    pub fn read(path: &Path) -> Result<Results> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Results {
                    schema: SCHEMA,
                    components: BTreeMap::new(),
                });
            }
            Err(e) => return Err(anyhow!("cannot read {}: {e}", path.display())),
        };
        let results: Results =
            serde_json::from_str(&text).map_err(|e| anyhow!("cannot read {}: {e}", path.display()))?;
        if results.schema != SCHEMA {
            bail!(
                "{} is written for schema {}; this fiber program reads schema {SCHEMA}",
                path.display(),
                results.schema
            );
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_in_the_documentation() {
        let text = r#"{"schema": 1, "components": {
            "base": {"wanted": "on", "result": "ok", "finished": "2026-11-17T21:04:05Z"},
            "node-red": {"wanted": "off", "result": "failed", "finished": "2026-11-17T21:05:00Z",
                         "message": "npm could not reach the registry"}}}"#;
        let results: Results = serde_json::from_str(text).unwrap();
        assert_eq!(results.components["base"].result, Outcome::Ok);
        assert_eq!(results.components["node-red"].wanted, Wanted::Off);
        assert_eq!(
            results.components["node-red"].message.as_deref(),
            Some("npm could not reach the registry")
        );
        let again: Results = serde_json::from_str(&serde_json::to_string(&results).unwrap()).unwrap();
        assert_eq!(again, results);
    }
}
