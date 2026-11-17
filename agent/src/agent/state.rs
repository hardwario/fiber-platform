//! What the agent knows and wants, and the `Desired` it sends from it.

use fiber_protocol::south::{Desired, Hello, LED_LINES, PORTS, Status};

use super::leds::{self, Led, Setting};
use super::power::{self, Power, Source};

/// How a chip's firmware stands (`firmware/{chip}` on MQTT).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirmwareState {
    /// It answers, with the image the agent carries.
    Running,
    Flashing,
    /// It does not answer.
    Silent,
    /// Writing it failed three times in an hour; the agent stops trying.
    Failed,
}

impl FirmwareState {
    pub fn name(self) -> &'static str {
        match self {
            FirmwareState::Running => "running",
            FirmwareState::Flashing => "flashing",
            FirmwareState::Silent => "silent",
            FirmwareState::Failed => "failed",
        }
    }
}

/// One chip, as the agent sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chip {
    pub state: FirmwareState,
    pub hello: Option<Hello>,
    /// The CRC-32 of the image the agent carries for it.
    pub carried: u32,
}

/// The agent's settings that the state needs (`agent.toml`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The board has the two voltage dividers (the `NB` variant of the schematic has not).
    pub dividers: bool,
}

/// What the agent knows and wants.
#[derive(Clone, Debug)]
pub struct State {
    pub settings: Settings,
    /// The ports' power as wanted: bit 0 is port 1. All on, as at the south bridge's start.
    pub ports: u8,
    /// The nine LEDs: the power LED, then ports 1 to 8.
    pub leds: [Setting; 9],
    pub power: Power,
    pub south: Chip,
    pub last_status: Option<Status>,
}

impl State {
    pub fn new(settings: Settings, south_carried: u32) -> Self {
        State {
            settings,
            ports: 0xFF,
            leds: [Setting::Auto; 9],
            power: Power::default(),
            south: Chip {
                state: FirmwareState::Silent,
                hello: None,
                carried: south_carried,
            },
            last_status: None,
        }
    }

    pub fn port_on(&self, port: u8) -> bool {
        (1..=PORTS as u8).contains(&port) && self.ports & (1 << (port - 1)) != 0
    }

    /// The `Desired` for the south bridge: every LED line and the ports.
    pub fn desired(&self) -> Desired {
        let mut lines = [fiber_protocol::south::Light::Off; LED_LINES];
        for i in 0..9u8 {
            let led = if i == 0 { Led::Power } else { Led::Port(i) };
            let port_on = i > 0 && self.port_on(i);
            let (first, second) = leds::lights(led, self.leds[usize::from(i)], self.power.source, port_on);
            lines[usize::from(i) * 2] = first;
            lines[usize::from(i) * 2 + 1] = second;
        }
        Desired {
            leds: lines,
            ports: self.ports,
        }
    }

    /// A `Hello` from the south bridge.
    pub fn south_hello(&mut self, hello: Hello) {
        self.south.hello = Some(hello);
        if self.south.state != FirmwareState::Flashing {
            self.south.state = FirmwareState::Running;
        }
    }

    /// A `Status` from the south bridge. Returns the new source when it switched between PoE and
    /// battery.
    pub fn south_status(&mut self, status: Status) -> Option<Source> {
        self.last_status = Some(status);
        let volts = match (self.settings.dividers, self.south.hello) {
            (true, Some(hello)) => power::volts(&status, hello.vrefint_cal),
            _ => None,
        };
        self.power.update(volts)
    }

    /// The south bridge stopped answering.
    pub fn south_silent(&mut self) {
        if self.south.state == FirmwareState::Running {
            self.south.state = FirmwareState::Silent;
        }
        self.last_status = None;
        self.power.update(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fiber_protocol::south::{Light, Reset};

    fn hello() -> Hello {
        Hello {
            version: [0, 1, 0],
            image_crc: 1,
            uid: [0; 12],
            reset: Reset::PowerOn,
            fault: None,
            option_bytes: 0,
            vrefint_cal: 1655,
        }
    }

    #[test]
    fn the_desired_follows_the_state() {
        let mut state = State::new(Settings { dividers: true }, 1);
        state.ports = 0b0000_0101;
        let desired = state.desired();
        assert_eq!(desired.ports, 0b0000_0101);
        assert_eq!(desired.leds[0], Light::On, "power green");
        assert_eq!(desired.leds[2], Light::On, "port 1 green, its power is on");
        assert_eq!(desired.leds[4], Light::Off, "port 2 dark, its power is off");
        assert_eq!(desired.leds[6], Light::On, "port 3 green");
    }

    #[test]
    fn the_power_needs_the_hello_and_the_dividers() {
        let status = Status {
            vin: 2557,
            vbat: 2544,
            vref: 1505,
            ..Status::default()
        };
        let mut state = State::new(Settings { dividers: true }, 1);
        state.south_status(status);
        assert_eq!(
            state.power.source,
            Source::Unknown,
            "no calibration before the Hello"
        );
        state.south_hello(hello());
        assert_eq!(state.south.state, FirmwareState::Running);
        state.south_status(status);
        assert_eq!(state.power.source, Source::Poe);
        let mut without = State::new(Settings { dividers: false }, 1);
        without.south_hello(hello());
        without.south_status(status);
        assert_eq!(
            without.power.source,
            Source::Unknown,
            "a board without dividers says nothing"
        );
        state.south_silent();
        assert_eq!(state.south.state, FirmwareState::Silent);
        assert_eq!(state.power.source, Source::Unknown);
    }
}
