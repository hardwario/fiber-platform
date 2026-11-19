//! The agent: the one process that owns the board (`docs/architecture.md`, "The agent").
//!
//! The logic is here, without the hardware and without MQTT, so it can be tested on its own:
//! [`state::State`] holds what the agent knows and wants, and turns it into the next `Desired`
//! for the south bridge, the payloads of the topics ([`topics`]), and the answers to requests
//! ([`requests`]). The I/O goes around it (T17, on the drafts of T13).

pub mod backlight;
pub mod buttons;
pub mod buzzer;
pub mod config;
mod daemon;
pub mod display;
pub mod firmware;
mod hardware;
pub mod icons;
pub mod images;
pub mod lcd;
pub mod leds;
pub mod link;
pub mod mqtt;
pub mod notify;
pub mod power;
pub mod radio;
pub mod requests;
pub mod run;
pub mod screen;
pub mod simulator;
pub mod state;
pub mod thermometers;
pub mod topics;
pub mod zone;

pub use daemon::agent;
pub use hardware::firmware_chip;
pub use simulator::south_simulator;
