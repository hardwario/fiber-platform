//! The local time zone, for the front panel's clock: `/etc/localtime`, the TZif file the system
//! keeps (FIBER Setup sets it), read with `tz-rs`, which also follows the rule at its end for the
//! years after its last transition. UTC when there is none.

use std::path::PathBuf;
use std::time::SystemTime;

use tz::TimeZone;

/// The zone, read again when `/etc/localtime` points elsewhere or its file changes.
pub struct Zone {
    path: PathBuf,
    /// The file the link resolved to and its time, when it was last read.
    read: Option<(PathBuf, Option<SystemTime>)>,
    zone: Option<TimeZone>,
}

impl Zone {
    pub fn new(path: PathBuf) -> Self {
        Zone {
            path,
            read: None,
            zone: None,
        }
    }

    /// The offset from UTC in seconds and the zone's abbreviation (`CET`) at the Unix time `ts`.
    pub fn at(&mut self, ts: u64) -> (i32, String) {
        let target = std::fs::canonicalize(&self.path).ok();
        let key = target.map(|t| {
            let modified = std::fs::metadata(&t).and_then(|m| m.modified()).ok();
            (t, modified)
        });
        if key != self.read {
            self.zone = std::fs::read(&self.path)
                .ok()
                .and_then(|bytes| TimeZone::from_tz_data(&bytes).ok());
            self.read = key;
        }
        self.zone
            .as_ref()
            .and_then(|zone| zone.find_local_time_type(ts as i64).ok())
            .map_or((0, "UTC".to_owned()), |local| {
                (local.ut_offset(), local.time_zone_designation().to_owned())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;

    #[test]
    fn summer_and_winter_in_prague() {
        let dir = TestDir::new("zone");
        let file = dir.path().join("Europe-Prague");
        std::fs::write(&file, include_bytes!("zone-europe-prague.tzif")).unwrap();
        let localtime = dir.path().join("localtime");
        std::os::unix::fs::symlink(&file, &localtime).unwrap();
        let mut zone = Zone::new(localtime.clone());
        // 2026-11-19 12:00 UTC, winter; 2026-07-01 12:00 UTC, summer; and 2040, past the file's
        // last transition, from its rule.
        assert_eq!(zone.at(1_795_089_600), (3600, "CET".into()));
        assert_eq!(zone.at(1_782_907_200), (7200, "CEST".into()));
        assert_eq!(zone.at(2_224_670_400), (7200, "CEST".into()));
        // The link pointed elsewhere: no zone there, so UTC.
        std::fs::remove_file(&localtime).unwrap();
        std::os::unix::fs::symlink(dir.path().join("none"), &localtime).unwrap();
        assert_eq!(zone.at(1_795_089_600), (0, "UTC".into()));
    }
}
