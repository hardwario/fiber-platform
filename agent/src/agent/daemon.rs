//! `fiber agent`: the agent as `fiber-agent.service` runs it.

use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use tokio_serial::SerialPortBuilderExt;

use super::config::Config;
use super::mqtt::Mqtt;
use super::run::{Agent, Stop, run};
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
        let mut agent = Agent::new(
            State::new(settings, super::images::south_crc()),
            bus,
            identity(paths),
        );
        agent.keep_tries(paths.flash_tries());
        // Held while the agent runs, so that a command run by hand keeps off the port.
        let _lock = match fiber_flasher::PortLock::take(&paths.lock_dir(), board::SOUTH_BRIDGE_UART) {
            Ok(lock) => Some(lock),
            Err(e) => {
                eprintln!("cannot lock {}: {e}", board::SOUTH_BRIDGE_UART);
                None
            }
        };
        eprintln!(
            "fiber agent {}: the south bridge on {}",
            env!("CARGO_PKG_VERSION"),
            board::SOUTH_BRIDGE_UART
        );
        // For the tests: another serial port, a pseudo-terminal to the simulated south bridge.
        let port = std::env::var("FIBER_SOUTH_PORT").unwrap_or_else(|_| board::SOUTH_BRIDGE_UART.to_owned());
        loop {
            match tokio_serial::new(&port, 115_200).open_native_async() {
                Ok(stream) => {
                    let (reader, writer) = tokio::io::split(stream);
                    match run(&mut agent, reader, writer).await {
                        Ok(Stop::Flash { request }) => {
                            // The link's port is closed again before the flasher opens it.
                            flash(&mut agent, request).await;
                            continue;
                        }
                        Err(error) => eprintln!("the south link: {error}"),
                    }
                }
                Err(error) => eprintln!("cannot open {port}: {error}"),
            }
            agent.state.south_silent();
            tokio::time::sleep(REOPEN).await;
        }
    })
}

/// Flashes the south bridge with the image the program carries, off the agent's loop.
async fn flash<B: super::run::Bus>(agent: &mut Agent<B>, request: Option<String>) {
    let Some(image) = super::images::south() else {
        return;
    };
    eprintln!("flashing the south bridge ({} bytes)", image.len());
    agent.begin_flash().await;
    let result = tokio::task::spawn_blocking(move || super::hardware::flash_south(image))
        .await
        .unwrap_or_else(|e| Err(format!("the flasher stopped: {e}")));
    match &result {
        Ok(()) => eprintln!("flashed the south bridge"),
        Err(e) => eprintln!("flashing the south bridge failed: {e}"),
    }
    agent.end_flash(result, request).await;
}
