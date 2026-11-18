//! The nine LEDs: the power LED and one for each port, each two lines
//! (`docs/firmware/south-link.md`, `Desired`), set by the agent or by a client (`leds.set`).

use fiber_protocol::south::Light;

use super::power::Source;

/// A tick of the south bridge, in milliseconds.
const TICK_MS: u16 = 20;

/// An LED.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Led {
    Power,
    /// 1 to 8.
    Port(u8),
}

impl Led {
    /// From its name in a request: `power`, `port1` to `port8`.
    pub fn parse(name: &str) -> Option<Led> {
        if name == "power" {
            return Some(Led::Power);
        }
        let n: u8 = name.strip_prefix("port")?.parse().ok()?;
        (1..=8).contains(&n).then_some(Led::Port(n))
    }

    /// Its place among the nine: 0 the power LED, then the ports.
    pub fn index(self) -> usize {
        match self {
            Led::Power => 0,
            Led::Port(n) => usize::from(n),
        }
    }
}

/// A colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colour {
    Off,
    Green,
    Red,
    /// A port LED's green and red together.
    Amber,
    /// The power LED's second colour.
    Yellow,
}

impl Colour {
    pub fn parse(name: &str) -> Option<Colour> {
        Some(match name {
            "off" => Colour::Off,
            "green" => Colour::Green,
            "red" => Colour::Red,
            "amber" => Colour::Amber,
            "yellow" => Colour::Yellow,
            _ => return None,
        })
    }

    /// Its two lines (green or the first colour, then the second), or `None` when the LED does
    /// not have it.
    fn lines(self, led: Led) -> Option<(bool, bool)> {
        Some(match (self, led) {
            (Colour::Off, _) => (false, false),
            (Colour::Green, _) => (true, false),
            (Colour::Yellow, Led::Power) => (false, true),
            (Colour::Red, Led::Port(_)) => (false, true),
            (Colour::Amber, Led::Port(_)) => (true, true),
            _ => return None,
        })
    }
}

/// How an LED is set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Setting {
    /// The agent shows the state on it.
    #[default]
    Auto,
    /// A client set it, steady or blinking (`on`, `off` in ticks).
    Set { colour: Colour, blink: Option<(u8, u8)> },
}

/// A blink in milliseconds as ticks: each a multiple of 20 from 20 to 5100.
pub fn blink_ticks(on_ms: u64, off_ms: u64) -> Option<(u8, u8)> {
    let ticks = |ms: u64| {
        (ms.is_multiple_of(u64::from(TICK_MS)) && (20..=5100).contains(&ms))
            .then(|| (ms / u64::from(TICK_MS)) as u8)
    };
    Some((ticks(on_ms)?, ticks(off_ms)?))
}

/// A setting checked against its LED: `None` when the LED has no such colour.
pub fn set(led: Led, colour: Colour, blink: Option<(u8, u8)>) -> Option<Setting> {
    colour.lines(led).map(|_| Setting::Set { colour, blink })
}

/// The two lines of an LED as lights. `failing`: a probe on the port gives no readings.
pub fn lights(led: Led, setting: Setting, source: Source, port_on: bool, failing: bool) -> (Light, Light) {
    let (colour, blink) = match setting {
        Setting::Set { colour, blink } => (colour, blink),
        // The power LED: green on PoE (or when not known), yellow on the battery. A port's
        // (`docs/mqtt-api.md`, `ports/n`): off without power, red while a probe on it gives no
        // readings, green otherwise.
        Setting::Auto => match led {
            Led::Power if source == Source::Battery => (Colour::Yellow, None),
            Led::Power => (Colour::Green, None),
            Led::Port(_) if !port_on => (Colour::Off, None),
            Led::Port(_) if failing => (Colour::Red, None),
            Led::Port(_) => (Colour::Green, None),
        },
    };
    let (first, second) = colour.lines(led).unwrap_or((false, false));
    let light = |lit: bool| match (lit, blink) {
        (false, _) => Light::Off,
        (true, None) => Light::On,
        (true, Some((on, off))) => Light::Blink { on, off, phase: 0 },
    };
    (light(first), light(second))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(Led::parse("power"), Some(Led::Power));
        assert_eq!(Led::parse("port8"), Some(Led::Port(8)));
        assert_eq!(Led::parse("port9"), None);
        assert_eq!(Led::parse("port0"), None);
        assert_eq!(Colour::parse("amber"), Some(Colour::Amber));
        assert_eq!(Colour::parse("blue"), None);
    }

    #[test]
    fn colours_each_led_has() {
        assert!(set(Led::Power, Colour::Yellow, None).is_some());
        assert!(set(Led::Power, Colour::Red, None).is_none());
        assert!(set(Led::Port(3), Colour::Amber, None).is_some());
        assert!(set(Led::Port(3), Colour::Yellow, None).is_none());
    }

    #[test]
    fn blinks_in_ticks() {
        assert_eq!(blink_ticks(500, 500), Some((25, 25)));
        assert_eq!(blink_ticks(20, 5100), Some((1, 255)));
        assert_eq!(blink_ticks(10, 500), None);
        assert_eq!(blink_ticks(5120, 500), None);
        assert_eq!(blink_ticks(510, 500), None);
    }

    #[test]
    fn what_the_lines_show() {
        assert_eq!(
            lights(Led::Power, Setting::Auto, Source::Poe, false, false),
            (Light::On, Light::Off)
        );
        assert_eq!(
            lights(Led::Power, Setting::Auto, Source::Battery, false, false),
            (Light::Off, Light::On)
        );
        assert_eq!(
            lights(Led::Port(1), Setting::Auto, Source::Poe, true, false),
            (Light::On, Light::Off)
        );
        assert_eq!(
            lights(Led::Port(1), Setting::Auto, Source::Poe, false, false),
            (Light::Off, Light::Off)
        );
        // A probe that gives no readings: red, but not without power.
        assert_eq!(
            lights(Led::Port(1), Setting::Auto, Source::Poe, true, true),
            (Light::Off, Light::On)
        );
        assert_eq!(
            lights(Led::Port(1), Setting::Auto, Source::Poe, false, true),
            (Light::Off, Light::Off)
        );
        let amber = Setting::Set {
            colour: Colour::Amber,
            blink: Some((25, 25)),
        };
        let blink = Light::Blink {
            on: 25,
            off: 25,
            phase: 0,
        };
        assert_eq!(
            lights(Led::Port(2), amber, Source::Poe, false, true),
            (blink, blink)
        );
    }
}
