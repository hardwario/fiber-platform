//! The DS18B20 probes on the eight ports (`docs/mqtt-api.md`, `thermometers/id`).
//!
//! [`Thermometers`] is what reads them, a round at a time; [`Kernel`] reads them through Linux's
//! 1-Wire drivers (`docs/hardware.md`, "The 1-Wire ports in Linux"). A driver of our own for the
//! DS2482 would be another implementation (D17). [`Probes`] is what the agent knows of them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::board;

/// How often all probes are read.
pub const ROUND_EVERY: Duration = Duration::from_secs(10);

/// Rounds in a row without a reading before a probe's temperature is `null`, and its port red.
pub const MISSES: u32 = 3;

/// What a DS18B20 holds after power-on, before its first conversion, in millidegrees.
const POWER_ON: i32 = 85_000;

/// Above this, a reading of 85 °C may be a real one.
const POWER_ON_BELIEVED_ABOVE: i32 = 75_000;

/// A probe as one round found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// The name Linux gives it: `28-` and its serial number.
    pub id: String,
    /// Millidegrees Celsius, or `None` when the read gave nothing.
    pub millis: Option<i32>,
}

/// One reading of every probe.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Round {
    /// The probes of ports 1 to 8; `None` when the eight 1-Wire buses were not found.
    pub ports: Option<Vec<Vec<Probe>>>,
}

/// What reads the probes. A round blocks (a read takes about 750 ms): run it off the agent's
/// loop.
pub trait Thermometers {
    fn round(&mut self) -> Round;
}

/// The probes through Linux's 1-Wire drivers, under `devices` (`/sys/bus/w1/devices`).
pub struct Kernel {
    devices: PathBuf,
}

impl Kernel {
    pub fn new(devices: PathBuf) -> Kernel {
        Kernel { devices }
    }

    /// The eight bus masters, port 1 first; `None` when there are not exactly eight.
    fn masters(&self) -> Option<Vec<PathBuf>> {
        let masters = bus_masters(&self.devices);
        (masters.len() == board::ONE_WIRE_PORTS as usize)
            .then(|| masters.into_iter().map(|(_, path)| path).collect())
    }
}

/// The 1-Wire bus masters under `devices`, by their number. Nothing in sysfs names a master's
/// chip or channel; the DS2482 registers its eight one after another, channel 0 first, so with
/// no other master the k-th is port k.
pub fn bus_masters(devices: &Path) -> Vec<(u32, PathBuf)> {
    let mut masters: Vec<(u32, PathBuf)> = std::fs::read_dir(devices)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let number = entry
                .file_name()
                .to_str()?
                .strip_prefix("w1_bus_master")?
                .parse()
                .ok()?;
            Some((number, entry.path()))
        })
        .collect();
    masters.sort();
    masters
}

/// The probes on one bus, each read in turn.
fn read_bus(master: &Path) -> Vec<Probe> {
    let slaves = std::fs::read_to_string(master.join("w1_master_slaves")).unwrap_or_default();
    slaves
        .lines()
        .filter(|id| id.starts_with(board::DS18B20_FAMILY))
        .map(|id| Probe {
            id: id.to_owned(),
            // Every error in the driver reads as an empty file.
            millis: std::fs::read_to_string(master.join(id).join("temperature"))
                .ok()
                .and_then(|text| text.trim().parse().ok()),
        })
        .collect()
}

impl Thermometers for Kernel {
    /// The eight buses side by side: each has its own lock in the kernel.
    fn round(&mut self) -> Round {
        let Some(masters) = self.masters() else {
            return Round { ports: None };
        };
        let ports = std::thread::scope(|scope| {
            let reads: Vec<_> = masters
                .iter()
                .map(|master| scope.spawn(move || read_bus(master)))
                .collect();
            reads
                .into_iter()
                .map(|read| read.join().unwrap_or_default())
                .collect()
        });
        Round { ports: Some(ports) }
    }
}

/// Reads a round every [`ROUND_EVERY`] and hands it over, until the receiver is gone. Run it on
/// a thread of its own.
pub fn read_forever<T: Thermometers>(mut thermometers: T, rounds: tokio::sync::mpsc::Sender<Round>) {
    loop {
        let started = Instant::now();
        if rounds.blocking_send(thermometers.round()).is_err() {
            return;
        }
        std::thread::sleep(ROUND_EVERY.saturating_sub(started.elapsed()));
    }
}

/// A probe as the agent knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Known {
    pub port: u8,
    /// The last reading; `None` before the first, and after [`MISSES`] rounds without one.
    pub millis: Option<i32>,
    /// Rounds in a row without a reading.
    pub misses: u32,
}

impl Known {
    /// Whether it is worth a topic: it has a reading, or has given none for long enough to say
    /// so.
    pub fn shown(&self) -> bool {
        self.millis.is_some() || self.misses >= MISSES
    }

    pub fn failing(&self) -> bool {
        self.misses >= MISSES
    }
}

/// What the agent knows of the probes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Probes {
    /// Whether the eight buses were found; `None` before the first round.
    pub buses: Option<bool>,
    pub known: BTreeMap<String, Known>,
}

impl Probes {
    /// Takes a round in. `powered` is the ports' power (bit 0 is port 1): the probes of a port
    /// without power are gone at once. Returns the ids of the probes that are gone.
    pub fn take(&mut self, round: Round, powered: u8) -> Vec<String> {
        self.buses = Some(round.ports.is_some());
        // Each probe with the ports that list it: a probe moved to another port stays listed on
        // its old one for about 100 s, and reads only on the new one.
        let mut listed: BTreeMap<String, Vec<(u8, Option<i32>)>> = BTreeMap::new();
        for (index, probes) in round.ports.into_iter().flatten().enumerate() {
            if powered & (1 << index) == 0 {
                continue;
            }
            for probe in probes {
                listed
                    .entry(probe.id)
                    .or_default()
                    .push((index as u8 + 1, probe.millis));
            }
        }
        for (id, places) in &listed {
            let before = self.known.get(id).map(|known| known.port);
            let (port, millis) = places
                .iter()
                .find(|(_, millis)| millis.is_some())
                .or_else(|| places.iter().find(|(port, _)| Some(*port) == before))
                .or(places.first())
                .copied()
                .unwrap_or((0, None));
            let known = self.known.entry(id.clone()).or_insert(Known {
                port,
                millis: None,
                misses: 0,
            });
            known.port = port;
            let last = known.millis;
            match millis {
                Some(POWER_ON) if !last.is_some_and(|m| m > POWER_ON_BELIEVED_ABOVE) => known.misses += 1,
                Some(millis) => {
                    known.millis = Some(millis);
                    known.misses = 0;
                }
                None => known.misses += 1,
            }
            if known.failing() {
                known.millis = None;
            }
        }
        let gone: Vec<String> = self
            .known
            .keys()
            .filter(|id| !listed.contains_key(*id))
            .cloned()
            .collect();
        for id in &gone {
            self.known.remove(id);
        }
        gone
    }

    /// The ids of the probes on `port`; `None` when the buses were not found.
    pub fn on_port(&self, port: u8) -> Option<Vec<&str>> {
        if self.buses == Some(false) {
            return None;
        }
        Some(
            self.known
                .iter()
                .filter(|(_, known)| known.port == port)
                .map(|(id, _)| id.as_str())
                .collect(),
        )
    }

    /// Whether a probe on `port` gives no readings.
    pub fn failing(&self, port: u8) -> bool {
        self.known
            .values()
            .any(|known| known.port == port && known.failing())
    }
}

/// Millidegrees as degrees with two decimals.
pub fn degrees(millis: i32) -> f64 {
    (f64::from(millis) / 10.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;

    /// A made-up `/sys/bus/w1/devices`: masters with probes and what their `temperature` reads.
    fn tree(dir: &Path, masters: &[(u32, &[(&str, &str)])]) {
        for (number, probes) in masters {
            let master = dir.join(format!("w1_bus_master{number}"));
            std::fs::create_dir_all(&master).unwrap();
            let mut slaves = String::new();
            for (id, temperature) in *probes {
                std::fs::create_dir_all(master.join(id)).unwrap();
                std::fs::write(master.join(id).join("temperature"), temperature).unwrap();
                slaves.push_str(id);
                slaves.push('\n');
            }
            if probes.is_empty() {
                slaves.push_str("not found.\n");
            }
            std::fs::write(master.join("w1_master_slaves"), slaves).unwrap();
        }
    }

    #[test]
    fn the_eight_buses_in_the_order_of_their_numbers() {
        let dir = TestDir::new("w1");
        // Numbers with a gap, and above 9, so that sorting by text would be wrong.
        tree(
            dir.path(),
            &[
                (3, &[("28-0000000000a1", "21437\n")]),
                (4, &[]),
                (5, &[]),
                (6, &[]),
                (7, &[]),
                (8, &[]),
                (9, &[]),
                (
                    10,
                    &[
                        ("28-0000000000b1", "-62\n"),
                        ("28-0000000000b2", ""),
                        ("10-0000000000c1", "1000\n"),
                    ],
                ),
            ],
        );
        let round = Kernel::new(dir.path().to_owned()).round();
        let ports = round.ports.unwrap();
        assert_eq!(ports.len(), 8);
        assert_eq!(
            ports[0],
            [Probe {
                id: "28-0000000000a1".into(),
                millis: Some(21437)
            }]
        );
        assert!(ports[1].is_empty());
        // Port 8: a reading below zero, an empty read, and no other family.
        assert_eq!(
            ports[7],
            [
                Probe {
                    id: "28-0000000000b1".into(),
                    millis: Some(-62)
                },
                Probe {
                    id: "28-0000000000b2".into(),
                    millis: None
                }
            ]
        );
    }

    #[test]
    fn more_or_fewer_buses_are_none() {
        let dir = TestDir::new("w1-seven");
        let seven: Vec<(u32, &[(&str, &str)])> = (1..=7).map(|n| (n, &[][..])).collect();
        tree(dir.path(), &seven);
        assert_eq!(Kernel::new(dir.path().to_owned()).round(), Round { ports: None });
        tree(dir.path(), &[(8, &[]), (9, &[])]);
        assert_eq!(Kernel::new(dir.path().to_owned()).round(), Round { ports: None });
        assert_eq!(
            Kernel::new(dir.path().join("none")).round(),
            Round { ports: None }
        );
    }

    fn round(port: usize, probes: &[(&str, Option<i32>)]) -> Round {
        let mut ports = vec![Vec::new(); 8];
        ports[port - 1] = probes
            .iter()
            .map(|(id, millis)| Probe {
                id: (*id).into(),
                millis: *millis,
            })
            .collect();
        Round { ports: Some(ports) }
    }

    #[test]
    fn readings_misses_and_gone() {
        let mut probes = Probes::default();
        assert!(
            probes
                .take(round(3, &[("28-a", Some(21437)), ("28-b", None)]), 0xFF)
                .is_empty()
        );
        assert_eq!(probes.known["28-a"].millis, Some(21437));
        assert!(probes.known["28-a"].shown());
        assert!(!probes.known["28-b"].shown());
        assert_eq!(probes.on_port(3), Some(vec!["28-a", "28-b"]));
        assert!(!probes.failing(3));

        // Three rounds without a reading: null, and the port is failing.
        probes.take(round(3, &[("28-a", None), ("28-b", None)]), 0xFF);
        assert_eq!(probes.known["28-a"].millis, Some(21437));
        probes.take(round(3, &[("28-a", None), ("28-b", None)]), 0xFF);
        assert!(probes.failing(3));
        assert!(probes.known["28-b"].shown());
        assert_eq!(probes.known["28-b"].millis, None);
        probes.take(round(3, &[("28-a", None), ("28-b", Some(20000))]), 0xFF);
        assert_eq!(probes.known["28-a"].millis, None);
        assert_eq!(probes.known["28-b"].millis, Some(20000));

        // Unplugged: gone once Linux no longer lists it.
        assert_eq!(probes.take(round(3, &[("28-b", Some(20000))]), 0xFF), ["28-a"]);
        assert!(!probes.failing(3));
        // The port's power off: its probes are gone at once.
        assert_eq!(
            probes.take(round(3, &[("28-b", Some(20000))]), 0b1111_1011),
            ["28-b"]
        );
        assert_eq!(probes.on_port(3), Some(vec![]));
        // The buses gone.
        probes.take(round(3, &[("28-b", Some(20000))]), 0xFF);
        assert_eq!(probes.take(Round { ports: None }, 0xFF), ["28-b"]);
        assert_eq!(probes.on_port(3), None);
    }

    #[test]
    fn a_probe_moved_to_another_port() {
        let mut probes = Probes::default();
        probes.take(round(5, &[("28-a", Some(21000))]), 0xFF);
        // Moved to port 3: Linux lists it on both for a while, and it reads on port 3 only.
        let mut both = round(3, &[("28-a", Some(21100))]);
        if let Some(ports) = &mut both.ports {
            ports[4] = vec![Probe {
                id: "28-a".into(),
                millis: None,
            }];
        }
        probes.take(both, 0xFF);
        assert_eq!(probes.known["28-a"].port, 3);
        assert_eq!(probes.known["28-a"].misses, 0);
        assert_eq!(probes.on_port(5), Some(vec![]));
    }

    #[test]
    fn eighty_five_degrees_after_power_on_is_not_a_reading() {
        let mut probes = Probes::default();
        probes.take(round(1, &[("28-a", Some(85_000))]), 0xFF);
        assert_eq!(probes.known["28-a"].millis, None);
        assert_eq!(probes.known["28-a"].misses, 1);
        probes.take(round(1, &[("28-a", Some(21_000))]), 0xFF);
        probes.take(round(1, &[("28-a", Some(85_000))]), 0xFF);
        assert_eq!(probes.known["28-a"].millis, Some(21_000));
        // From a hot one, 85 °C is believed.
        probes.take(round(1, &[("28-a", Some(80_000))]), 0xFF);
        probes.take(round(1, &[("28-a", Some(85_000))]), 0xFF);
        assert_eq!(probes.known["28-a"].millis, Some(85_000));
    }

    #[test]
    fn two_decimals() {
        assert_eq!(degrees(21437), 21.44);
        assert_eq!(degrees(-62), -0.06);
        assert_eq!(degrees(85_000), 85.0);
    }
}
