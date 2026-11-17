//! When the agent writes a chip's firmware (`docs/architecture.md`, "The agent", Firmware): the
//! program carries both images, and a chip whose `Hello` names another image, that speaks another
//! schema, or that says nothing, is flashed. At most three tries for one image in an hour,
//! remembered across restarts, so that a chip that cannot be written does not wear out its flash.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Tries for one image within [`WINDOW`].
pub const TRIES: usize = 3;
/// The window, in seconds.
pub const WINDOW: u64 = 3600;

/// What the agent knows of a chip's firmware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    /// Its `Hello`, with the CRC-32 of its image.
    Hello { image_crc: u32 },
    /// Frames with another schema tag: a firmware from another build.
    OtherSchema,
    /// Nothing at all, for longer than it takes to start.
    Silent,
}

/// Whether to flash the chip that was `seen`, carrying an image with `carried_crc`.
pub fn needs_flash(seen: Seen, carried_crc: u32) -> bool {
    match seen {
        Seen::Hello { image_crc } => image_crc != carried_crc,
        Seen::OtherSchema | Seen::Silent => true,
    }
}

/// The tries of each image, as times in seconds; kept in a file across restarts.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Tries {
    /// By the image's CRC-32 as hex, and the chip's name.
    pub tries: BTreeMap<String, Vec<u64>>,
}

impl Tries {
    fn key(chip: &str, image_crc: u32) -> String {
        format!("{chip}:{image_crc:08x}")
    }

    /// Whether another try of this image is allowed at `now`.
    pub fn allowed(&self, chip: &str, image_crc: u32, now: u64) -> bool {
        let recent = self.tries.get(&Self::key(chip, image_crc)).map_or(0, |times| {
            times.iter().filter(|t| now.saturating_sub(**t) < WINDOW).count()
        });
        recent < TRIES
    }

    /// A try at `now`; tries older than the window are forgotten.
    pub fn record(&mut self, chip: &str, image_crc: u32, now: u64) {
        let times = self.tries.entry(Self::key(chip, image_crc)).or_default();
        times.retain(|t| now.saturating_sub(*t) < WINDOW);
        times.push(now);
    }

    /// The tries kept in `path`; none when it is not there or not readable.
    pub fn load(path: &Path) -> Tries {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string(self).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_to_flash() {
        assert!(!needs_flash(Seen::Hello { image_crc: 7 }, 7));
        assert!(needs_flash(Seen::Hello { image_crc: 6 }, 7));
        assert!(needs_flash(Seen::OtherSchema, 7));
        assert!(needs_flash(Seen::Silent, 7));
    }

    #[test]
    fn three_tries_an_hour() {
        let mut tries = Tries::default();
        for t in [0, 10, 20] {
            assert!(tries.allowed("south", 7, t));
            tries.record("south", 7, t);
        }
        assert!(!tries.allowed("south", 7, 30));
        assert!(tries.allowed("south", 8, 30), "another image counts on its own");
        assert!(tries.allowed("north", 7, 30), "and another chip");
        assert!(tries.allowed("south", 7, WINDOW + 1));
    }

    #[test]
    fn kept_across_restarts() {
        let tmp = crate::testdir::TestDir::new("tries");
        let path = tmp.path().join("agent/flash-tries.json");
        let mut tries = Tries::default();
        tries.record("south", 7, 100);
        tries.save(&path).unwrap();
        assert_eq!(Tries::load(&path), tries);
        assert_eq!(Tries::load(&tmp.path().join("none.json")), Tries::default());
    }
}
