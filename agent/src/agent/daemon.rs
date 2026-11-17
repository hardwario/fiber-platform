//! `fiber agent`: the agent as `fiber-agent.service` runs it.

use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use tokio_serial::SerialPortBuilderExt;

use super::config::Config;
use super::mqtt::Mqtt;
use super::run::{Agent, run};
use super::state::{Settings, State};
use super::topics::Identity;
use crate::board;
use crate::paths::Paths;

/// How long to wait before opening the south bridge's port again.
const REOPEN: Duration = Duration::from_secs(5);

/// The FIBER's identity, as `fiber/status` shows it.
fn identity(paths: &Paths) -> Identity {
    let read = |path: &str| {
        std::fs::read_to_string(paths.under(path))
            .map(|s| s.trim().trim_end_matches('\0').to_owned())
            .ok()
    };
    Identity {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        hardware: board::HARDWARE.to_owned(),
        serial: read("/sys/firmware/devicetree/base/serial-number").unwrap_or_default(),
        name: read("/proc/sys/kernel/hostname").unwrap_or_default(),
    }
}

/// `fiber agent`.
pub fn agent(paths: &Paths) -> Result<ExitCode> {
    let config = Config::read(&paths.agent_config())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let bus = Mqtt::connect(&config.mqtt.host, config.mqtt.port);
        let settings = Settings {
            dividers: config.board.dividers,
        };
        // The firmware images the program carries come with the flasher's adapters (T17).
        let mut agent = Agent::new(State::new(settings, None), bus, identity(paths));
        eprintln!(
            "fiber agent {}: the south bridge on {}",
            env!("CARGO_PKG_VERSION"),
            board::SOUTH_BRIDGE_UART
        );
        loop {
            match tokio_serial::new(board::SOUTH_BRIDGE_UART, 115_200).open_native_async() {
                Ok(port) => {
                    let (reader, writer) = tokio::io::split(port);
                    if let Err(error) = run(&mut agent, reader, writer).await {
                        eprintln!("the south link: {error}");
                    }
                }
                Err(error) => eprintln!("cannot open {}: {error}", board::SOUTH_BRIDGE_UART),
            }
            agent.state.south_silent();
            tokio::time::sleep(REOPEN).await;
        }
    })
}
