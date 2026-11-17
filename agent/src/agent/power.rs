//! The power source and the two voltages, from the south bridge's raw counts
//! (`docs/firmware/south-link.md`, `Status`).

use fiber_protocol::south::Status;

/// The full scale of the 12-bit ADC.
const FULL_SCALE: f64 = 4095.0;
/// The supply at which `VREFINT_CAL` was measured.
const CAL_SUPPLY: f64 = 3.0;

/// The input above which the FIBER is on PoE; and below which it is on its battery. Between the
/// two the source stays what it was, so it does not flap.
pub const POE_ABOVE: f64 = 10.5;
pub const BATTERY_BELOW: f64 = 9.0;

/// The two voltages, in volts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volts {
    pub input: f64,
    pub battery: f64,
}

/// The counts in volts: the supply from the reference and its calibration, each pin from its
/// count, the battery through 68 k / 68 k, the input through 330 k / 68 k. `None` without a
/// reference count or a calibration.
pub fn volts(status: &Status, vrefint_cal: u16) -> Option<Volts> {
    if status.vref == 0 || vrefint_cal == 0 {
        return None;
    }
    let supply = CAL_SUPPLY * f64::from(vrefint_cal) / f64::from(status.vref);
    let pin = |count: u16| f64::from(count) * supply / FULL_SCALE;
    Some(Volts {
        input: pin(status.vin) * 398.0 / 68.0,
        battery: pin(status.vbat) * 2.0,
    })
}

/// Where the power comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Poe,
    Battery,
    /// The south bridge does not answer, or the board has no dividers.
    Unknown,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Poe => "poe",
            Source::Battery => "battery",
            Source::Unknown => "unknown",
        }
    }
}

/// The source and the voltages as the agent shows them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Power {
    pub source: Source,
    pub volts: Option<Volts>,
}

impl Default for Power {
    fn default() -> Self {
        Power {
            source: Source::Unknown,
            volts: None,
        }
    }
}

impl Power {
    /// Takes new voltages (`None`: none to be had). Returns the new source when it changed
    /// between PoE and battery, for `power/event`.
    pub fn update(&mut self, volts: Option<Volts>) -> Option<Source> {
        self.volts = volts;
        let before = self.source;
        self.source = match (volts, before) {
            (None, _) => Source::Unknown,
            (Some(v), _) if v.input >= POE_ABOVE => Source::Poe,
            (Some(v), _) if v.input < BATTERY_BELOW => Source::Battery,
            // In between: as it was; from unknown, by the middle.
            (Some(_), Source::Poe | Source::Battery) => before,
            (Some(v), Source::Unknown) if v.input >= (POE_ABOVE + BATTERY_BELOW) / 2.0 => Source::Poe,
            (Some(_), Source::Unknown) => Source::Battery,
        };
        let changed = matches!(
            (before, self.source),
            (Source::Poe, Source::Battery) | (Source::Battery, Source::Poe)
        );
        changed.then_some(self.source)
    }

    /// Whether `volts` differ from the ones shown by more than 0.05 V.
    pub fn moved(&self, volts: Option<Volts>) -> bool {
        match (self.volts, volts) {
            (Some(a), Some(b)) => (a.input - b.input).abs() > 0.05 || (a.battery - b.battery).abs() > 0.05,
            (a, b) => a.is_some() != b.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(vbat: u16, vin: u16, vref: u16) -> Status {
        Status {
            vbat,
            vin,
            vref,
            ..Status::default()
        }
    }

    #[test]
    fn counts_to_volts() {
        // A supply of 3.3 V: the reference reads 3.0 / 3.3 of its calibration.
        let cal = 1655;
        let vref = (f64::from(cal) * 3.0 / 3.3).round() as u16;
        let v = volts(&status(2544, 862, vref), cal).unwrap();
        assert!((v.battery - 4.10).abs() < 0.01, "{v:?}");
        assert!((v.input - 4.06).abs() < 0.01, "{v:?}");
        let v = volts(&status(0, 2557, vref), cal).unwrap();
        assert!((v.input - 12.05).abs() < 0.02, "{v:?}");
        assert_eq!(volts(&status(1, 1, 0), cal), None);
        assert_eq!(volts(&status(1, 1, 1500), 0), None);
    }

    fn input(v: f64) -> Option<Volts> {
        Some(Volts {
            input: v,
            battery: 4.0,
        })
    }

    #[test]
    fn the_source_does_not_flap() {
        let mut power = Power::default();
        assert_eq!(power.update(input(12.0)), None, "unknown to PoE is no event");
        assert_eq!(power.source, Source::Poe);
        assert_eq!(power.update(input(9.5)), None, "in between: as it was");
        assert_eq!(power.update(input(8.0)), Some(Source::Battery));
        assert_eq!(power.update(input(10.0)), None);
        assert_eq!(power.source, Source::Battery);
        assert_eq!(power.update(input(11.0)), Some(Source::Poe));
        assert_eq!(power.update(None), None);
        assert_eq!(power.source, Source::Unknown);
    }

    #[test]
    fn small_moves_are_not_news() {
        let mut power = Power::default();
        power.update(input(12.0));
        assert!(!power.moved(input(12.04)));
        assert!(power.moved(input(12.1)));
        assert!(power.moved(None));
    }
}
