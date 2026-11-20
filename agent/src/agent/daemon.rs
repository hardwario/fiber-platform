//! `fiber agent`: the agent as `fiber-agent.service` runs it.

use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use tokio_serial::SerialPortBuilderExt;

use super::config::Config;
use super::mqtt::Mqtt;
use super::notify::Notify;
use super::run::{Agent, Stop, run, run_without_link};
use super::state::{Settings, State};
use super::thermometers::{Kernel, read_forever};
use super::topics::Identity;
use crate::board;
use crate::paths::Paths;

/// How long to wait before opening the south bridge's port again.
const REOPEN: Duration = Duration::from_secs(5);

/// How often the watchdog is fed while the agent flashes, and its loop does not run. Well inside
/// the unit's `WatchdogSec=` (30 s).
const BEAT: Duration = Duration::from_secs(1);

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
        agent.notify = Notify::from_env();
        // The thermometers, read on a thread of their own: a read blocks for about 750 ms. For
        // the tests: another 1-Wire tree, made up.
        let devices = std::env::var_os("FIBER_W1_DEVICES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| paths.under(board::W1_DEVICES));
        let (rounds, received) = tokio::sync::mpsc::channel(1);
        agent.rounds = Some(received);
        std::thread::Builder::new()
            .name("thermometers".into())
            .spawn(move || read_forever(Kernel::new(devices), rounds))?;
        front_panel(&mut agent, paths)?;
        agent.sources = Some(super::run::Sources {
            results: paths.platform_results(),
            status: paths.platform_status(),
            fib_trie: paths.under("/proc/net/fib_trie"),
            restart: paths.restart_request(),
            zone: super::zone::Zone::new(paths.under("/etc/localtime")),
        });
        agent.radio = radio(paths);
        // Held while the agent runs, so that a command run by hand keeps off the port.
        let _north_lock = match fiber_flasher::PortLock::take(&paths.lock_dir(), board::NORTH_BRIDGE_UART) {
            Ok(lock) => Some(lock),
            Err(e) => {
                eprintln!("cannot lock {}: {e}", board::NORTH_BRIDGE_UART);
                None
            }
        };
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
        agent.notify.send("READY=1");
        // For the tests: another serial port, a pseudo-terminal to the simulated south bridge.
        let port = std::env::var("FIBER_SOUTH_PORT").unwrap_or_else(|_| board::SOUTH_BRIDGE_UART.to_owned());
        // The last error with the port, said once and not every 5 s.
        let mut said: Option<String> = None;
        loop {
            let stop = match tokio_serial::new(&port, 115_200).open_native_async() {
                Ok(stream) => {
                    if said.take().is_some() {
                        eprintln!("opened {port}");
                    }
                    let (reader, writer) = tokio::io::split(stream);
                    match run(&mut agent, reader, writer).await {
                        Ok(stop) => stop,
                        Err(error) => {
                            say(&mut said, format!("the south link: {error}"));
                            run_without_link(&mut agent, REOPEN).await
                        }
                    }
                }
                Err(error) => {
                    say(&mut said, format!("cannot open {port}: {error}"));
                    run_without_link(&mut agent, REOPEN).await
                }
            };
            // The link's port is closed again before the flasher opens it.
            if let Stop::Flash { request } = stop {
                flash(&mut agent, request).await;
            }
        }
    })
}

/// The radio (T26): its nodes from their file, or a new gateway address at random the first time,
/// and the north link on its port. A file that cannot be read is left as it is (it holds the
/// nodes' keys), and the agent goes on without the radio.
fn radio(paths: &Paths) -> Option<super::run::RadioSide> {
    use super::radio::{service::Radio, store};
    let path = paths.radio_nodes();
    let kept = match store::Store::load(&path) {
        Ok(Some(kept)) => kept,
        Ok(None) => {
            let gateway = match store::random_address(&paths.under("/dev/urandom")) {
                Ok(gateway) => gateway,
                Err(e) => {
                    eprintln!("the radio: no random address: {e}");
                    return None;
                }
            };
            let new = store::Store::new(gateway);
            if let Err(e) = new.save(&path) {
                eprintln!("the radio: cannot keep {}: {e}", path.display());
                return None;
            }
            new
        }
        Err(e) => {
            eprintln!(
                "the radio: {} cannot be read, and is left as it is: {e}",
                path.display()
            );
            return None;
        }
    };
    // For the tests: another serial port, a pseudo-terminal to a simulated north bridge.
    let port = std::env::var("FIBER_NORTH_PORT").unwrap_or_else(|_| board::NORTH_BRIDGE_UART.to_owned());
    let tower: String = env!("FIBER_TOWER_REV").chars().take(7).collect();
    Some(super::run::RadioSide {
        service: Radio::new(kept, tower),
        north: super::north::spawn(port),
        store_path: path,
    })
}

/// The front panel's LCD, buttons, buzzer and backlight. Each one that is not there is said once, and
/// the rest goes on without it.
fn front_panel<B: super::run::Bus>(agent: &mut Agent<B>, paths: &Paths) -> Result<()> {
    let (backlight, duties) = tokio::sync::mpsc::unbounded_channel();
    agent.backlight = Some(backlight);
    tokio::spawn(super::backlight::drive(
        super::backlight::Pwm::new(
            paths.under(board::BACKLIGHT_PWM_CHIP),
            board::BACKLIGHT_PWM_CHANNEL,
        ),
        duties,
    ));
    #[cfg(target_os = "linux")]
    {
        match super::hardware::GpioBuzzer::new() {
            Ok(line) => {
                let (buzzer, patterns) = tokio::sync::mpsc::unbounded_channel();
                agent.buzzer = Some(buzzer);
                tokio::spawn(super::buzzer::play(line, patterns));
            }
            Err(e) => eprintln!("the buzzer: {e}"),
        }
        match super::hardware::SpiLcd::new() {
            Ok(bus) => {
                let (lcd, pictures) = std::sync::mpsc::channel();
                agent.lcd = Some(lcd);
                std::thread::Builder::new()
                    .name("lcd".into())
                    .spawn(move || super::lcd::drive(super::lcd::Lcd::new(bus), pictures))?;
            }
            Err(e) => eprintln!("the LCD: {e}"),
        }
        match super::hardware::GpioButtons::new() {
            Ok(lines) => {
                let (gestures, received) = tokio::sync::mpsc::channel(16);
                agent.gestures = Some(received);
                std::thread::Builder::new()
                    .name("buttons".into())
                    .spawn(move || {
                        if let Err(e) = super::buttons::watch(lines, gestures) {
                            eprintln!("the buttons: {e}");
                        }
                    })?;
            }
            Err(e) => eprintln!("the buttons: {e}"),
        }
    }
    Ok(())
}

/// Prints `message` when it is not the one printed last.
fn say(said: &mut Option<String>, message: String) {
    if said.as_deref() != Some(message.as_str()) {
        eprintln!("{message}");
        *said = Some(message);
    }
}

/// Flashes the south bridge with the image the program carries, off the agent's loop.
async fn flash<B: super::run::Bus>(agent: &mut Agent<B>, request: Option<String>) {
    let Some(image) = super::images::south() else {
        return;
    };
    eprintln!("flashing the south bridge ({} bytes)", image.len());
    agent.begin_flash().await;
    let mut flashing = tokio::task::spawn_blocking(move || super::hardware::flash_south(image));
    // The flasher gives up on a chip that does not answer, so this ends.
    let result = loop {
        tokio::select! {
            done = &mut flashing => {
                break done.unwrap_or_else(|e| Err(format!("the flasher stopped: {e}")));
            }
            _ = tokio::time::sleep(BEAT) => agent.notify.send("WATCHDOG=1"),
        }
    };
    match &result {
        Ok(()) => eprintln!("flashed the south bridge"),
        Err(e) => eprintln!("flashing the south bridge failed: {e}"),
    }
    agent.end_flash(result, request).await;
}
