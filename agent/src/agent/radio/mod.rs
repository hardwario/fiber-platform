//! The radio service (`docs/mqtt-api.md`, "The radio"; T26): the FIBER as a gateway for TOWER
//! nodes, through the north bridge (`docs/firmware/north-link.md`).
//!
//! As the rest of the agent, the logic is here without the I/O: [`service::Radio`] takes what the
//! north bridge sends, the requests and the clients' `set`s, and answers with [`service::Effect`]s
//! (what to publish, what to send to the north bridge, when to save). The nodes are kept in
//! [`store::Store`] (D8); TOWER's application messages and property tables are
//! `tower-protocol`'s, read with [`values`].

pub mod service;
pub mod store;
pub mod values;
