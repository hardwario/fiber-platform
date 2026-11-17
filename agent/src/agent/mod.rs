//! The agent: the one process that owns the board (`docs/architecture.md`, "The agent").
//!
//! The logic is here, without the hardware and without MQTT, so it can be tested on its own:
//! [`state::State`] holds what the agent knows and wants, and turns it into the next `Desired`
//! for the south bridge, the payloads of the topics ([`topics`]), and the answers to requests
//! ([`requests`]). The I/O goes around it (T17, on the drafts of T13).

pub mod firmware;
pub mod leds;
pub mod power;
pub mod requests;
pub mod state;
pub mod topics;
