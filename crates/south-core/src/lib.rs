//! The south bridge's logic, without the hardware (`docs/firmware/south-link.md`, "The timing").
//!
//! The firmware calls [`Engine::tick`] 50 times a second and sets its 18 LED lines and 8 port
//! lines as it says, hands every `Desired` to [`Engine::desired`], and answers it. The engine
//! keeps nothing the agent did not send, but for the time since start: after a reset the agent's
//! next `Desired` puts everything back.
//!
//! - **At start** the power LED blinks green, and the ports come on one after another, 100 ms
//!   apart, so that 1-Wire works without the agent and the inrush stays small.
//! - **A `Desired`** sets every LED line and the ports, from the next tick on.
//! - **No `Desired` for 5 s:** the power LED blinks yellow and the port LEDs are dark; the ports
//!   stay as they were. The next `Desired` ends it.
//!
//! Every blinking line counts from the same tick, so lines with the same `on`, `off` and `phase`
//! blink together.
#![no_std]

use fiber_protocol::south::{Desired, LED_LINES, Light, PORTS};

/// Ticks a second.
pub const TICKS_PER_SECOND: u32 = 50;
/// Ticks without a `Desired` before the fallback pattern.
pub const FALLBACK_AFTER: u32 = 5 * TICKS_PER_SECOND;
/// Ticks between two ports switched on at start (100 ms).
pub const PORT_STEP: u32 = 5;

/// The power LED's lines.
pub const POWER_GREEN: usize = 0;
pub const POWER_YELLOW: usize = 1;

/// The start pattern: the power LED blinks green, once a second.
const START: Light = Light::Blink {
    on: 25,
    off: 25,
    phase: 0,
};
/// The fallback: the power LED blinks yellow, briefly, once a second.
const FALLBACK: Light = Light::Blink {
    on: 5,
    off: 45,
    phase: 0,
};

/// What the lines should be at a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outputs {
    /// Lit or dark, for each LED line (the firmware drives a lit line low).
    pub leds: [bool; LED_LINES],
    /// Bit 0 is port 1; 1 = on (driven high).
    pub ports: u8,
}

/// What the engine shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// No `Desired` yet since start.
    Start,
    /// The last `Desired`.
    Desired,
    /// No `Desired` for 5 s.
    Fallback,
}

/// The LED engine and the ports.
#[derive(Clone, Debug)]
pub struct Engine {
    tick: u32,
    since_desired: u32,
    desired: Option<Desired>,
    ports: u8,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a light is lit at a tick.
pub fn lit(light: Light, tick: u32) -> bool {
    match light {
        Light::Off => false,
        Light::On => true,
        Light::Blink { on, off, phase } => {
            let cycle = u32::from(on) + u32::from(off);
            cycle != 0 && (tick + u32::from(phase)) % cycle < u32::from(on)
        }
    }
}

impl Engine {
    pub const fn new() -> Self {
        Self {
            tick: 0,
            since_desired: 0,
            desired: None,
            ports: 0,
        }
    }

    /// A `Desired` from the agent, applied from the next tick on.
    pub fn desired(&mut self, desired: Desired) {
        self.desired = Some(desired);
        self.since_desired = 0;
        self.ports = desired.ports;
    }

    pub fn mode(&self) -> Mode {
        match self.desired {
            None => Mode::Start,
            Some(_) if self.since_desired >= FALLBACK_AFTER => Mode::Fallback,
            Some(_) => Mode::Desired,
        }
    }

    /// The ports as they are now.
    pub fn ports(&self) -> u8 {
        self.ports
    }

    /// Seconds since start.
    pub fn uptime(&self) -> u32 {
        self.tick / TICKS_PER_SECOND
    }

    /// One tick (20 ms): what the lines should be now.
    pub fn tick(&mut self) -> Outputs {
        let tick = self.tick;
        self.tick = self.tick.wrapping_add(1);
        self.since_desired = self.since_desired.saturating_add(1);

        let mut leds = [false; LED_LINES];
        match (self.mode(), self.desired) {
            (Mode::Desired, Some(desired)) => {
                for (led, light) in leds.iter_mut().zip(desired.leds) {
                    *led = lit(light, tick);
                }
            }
            (Mode::Fallback, _) => leds[POWER_YELLOW] = lit(FALLBACK, tick),
            _ => {
                leds[POWER_GREEN] = lit(START, tick);
                // The ports, one after another: port n at tick n x 100 ms.
                let on = (tick / PORT_STEP).min(PORTS as u32);
                self.ports = ((1u16 << on) - 1) as u8;
            }
        }
        Outputs {
            leds,
            ports: self.ports,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(engine: &mut Engine, ticks: u32) -> Outputs {
        let mut last = engine.tick();
        for _ in 1..ticks {
            last = engine.tick();
        }
        last
    }

    #[test]
    fn blinking() {
        let light = Light::Blink {
            on: 2,
            off: 3,
            phase: 0,
        };
        let pattern: [bool; 10] = core::array::from_fn(|t| lit(light, t as u32));
        assert_eq!(
            pattern,
            [true, true, false, false, false, true, true, false, false, false]
        );
        let shifted = Light::Blink {
            on: 2,
            off: 3,
            phase: 2,
        };
        assert!(!lit(shifted, 0) && lit(shifted, 3) && lit(shifted, 4));
        assert!(!lit(
            Light::Blink {
                on: 0,
                off: 0,
                phase: 0
            },
            7
        ));
        assert!(lit(
            Light::Blink {
                on: 1,
                off: 0,
                phase: 0
            },
            7
        ));
        assert!(lit(Light::On, 0) && !lit(Light::Off, 0));
    }

    #[test]
    fn at_start_the_ports_come_on_one_after_another() {
        let mut engine = Engine::new();
        let ports: [u8; 10] = core::array::from_fn(|_| run(&mut engine, PORT_STEP).ports);
        assert_eq!(
            ports,
            [0x00, 0x01, 0x03, 0x07, 0x0f, 0x1f, 0x3f, 0x7f, 0xff, 0xff]
        );
        assert_eq!(engine.mode(), Mode::Start);
    }

    #[test]
    fn at_start_the_power_led_blinks_green() {
        let mut engine = Engine::new();
        let first = engine.tick();
        assert!(first.leds[POWER_GREEN] && !first.leds[POWER_YELLOW]);
        assert!(first.leds[2..].iter().all(|l| !l));
        assert!(!run(&mut engine, 25).leds[POWER_GREEN]);
    }

    #[test]
    fn a_desired_sets_everything() {
        let mut engine = Engine::new();
        run(&mut engine, 3);
        let mut leds = [Light::Off; LED_LINES];
        leds[3] = Light::On;
        leds[5] = Light::Blink {
            on: 1,
            off: 1,
            phase: 0,
        };
        engine.desired(Desired {
            leds,
            ports: 0b1010_0101,
        });
        let out = engine.tick();
        assert_eq!(engine.mode(), Mode::Desired);
        assert_eq!(out.ports, 0b1010_0101);
        assert!(out.leds[3] && !out.leds[POWER_GREEN]);
        let blink = [out.leds[5], engine.tick().leds[5]];
        assert_ne!(blink[0], blink[1]);
    }

    #[test]
    fn without_a_desired_for_5_s_the_fallback() {
        let mut engine = Engine::new();
        let leds = [Light::On; LED_LINES];
        engine.desired(Desired { leds, ports: 0x0f });
        run(&mut engine, FALLBACK_AFTER - 1);
        assert_eq!(engine.mode(), Mode::Desired);
        let out = engine.tick();
        assert_eq!(engine.mode(), Mode::Fallback);
        assert_eq!(out.ports, 0x0f, "the ports stay as they were");
        assert!(out.leds[2..].iter().all(|l| !l), "the port LEDs are dark");
        let yellow: u32 = (0..50).map(|_| u32::from(engine.tick().leds[POWER_YELLOW])).sum();
        assert_eq!(yellow, 5);
        // The next Desired ends it.
        engine.desired(Desired { leds, ports: 0x0f });
        engine.tick();
        assert_eq!(engine.mode(), Mode::Desired);
    }

    #[test]
    fn uptime() {
        let mut engine = Engine::new();
        run(&mut engine, 3 * TICKS_PER_SECOND);
        assert_eq!(engine.uptime(), 3);
    }
}
