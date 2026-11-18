//! `fiber`: the HARDWARIO FIBER on-device software in one binary.
//!
//! It will be three things (the plan and its milestones are in `TASKS.md`):
//!
//! * the platform tool, `fiber platform ...`, which applies the components chosen in FIBER Setup;
//! * the agent, `fiber agent`, the one process that owns the board and presents it over MQTT;
//! * the command line that queries and commands the board through the agent.
//!
//! `fiber version` is a contract with FIBER Setup, whose first-start script records its output:
//! one line, the bare version. The commands are written down in `docs/cli.md`.

#[allow(dead_code)] // The agent's I/O around it comes next (T17).
mod agent;
mod board;
mod client;
mod doctor;
mod paths;
mod platform;
mod release;
#[cfg(test)]
mod testdir;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tower_protocol::radio::RADIO_TAG;

use paths::Paths;

/// The commit of github.com/hardwario/tower this binary was built against (`build.rs`).
const TOWER_REV: &str = env!("FIBER_TOWER_REV");

#[derive(Parser)]
#[command(name = "fiber", version, about = "The HARDWARIO FIBER on-device software")]
struct Cli {
    /// Read and write every file below this directory instead of `/` (for the tests).
    #[arg(long, global = true, hide = true, value_name = "DIR")]
    root: Option<PathBuf>,

    /// The broker of another FIBER, as HOST or HOST:PORT (the board's commands; this FIBER without it).
    #[arg(long, global = true, value_name = "HOST[:PORT]")]
    broker: Option<String>,

    /// The user for a FIBER's listener for the network; the password in FIBER_MQTT_PASSWORD.
    #[arg(long, global = true, value_name = "USER")]
    user: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the version of the FIBER software.
    Version {
        /// Also print what this build is made of: the TOWER revision its radio is compatible
        /// with, and the schema tag of the TOWER radio application messages.
        #[arg(long)]
        all: bool,
    },
    /// The agent: the one process that owns the board and shows it on MQTT. Run by
    /// fiber-agent.service.
    Agent,
    /// The south bridge, simulated, on a serial port (for the tests).
    #[command(hide = true)]
    SouthSimulator { port: PathBuf },
    /// The FIBER, the agent, the power, the ports and the firmware, in short.
    Status,
    /// The power source and the two voltages.
    Power {
        /// Then a line on every change and every power event, until Ctrl+C.
        #[arg(long)]
        follow: bool,
    },
    /// The power of the 1-Wire ports: show it, or switch ports on, off, or off and on again.
    Ports {
        /// on, off or cycle; without it, the ports are shown.
        #[arg(value_parser = ["on", "off", "cycle"])]
        action: Option<String>,
        /// The ports, 1 to 8; all of them when none is named.
        #[arg(value_parser = clap::value_parser!(u8).range(1..=8))]
        ports: Vec<u8>,
        /// For cycle: how long the ports stay off, 1 to 60 s (5 without it).
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=60))]
        seconds: Option<u64>,
    },
    /// The LEDs: show how they are set, set them, or give them back to the agent.
    Leds {
        /// auto, or a colour: off, green, red, amber, yellow. Without it, the LEDs are shown.
        #[arg(value_parser = ["auto", "off", "green", "red", "amber", "yellow"])]
        action: Option<String>,
        /// power, port1 to port8; all port LEDs when none is named.
        leds: Vec<String>,
        /// Blink: milliseconds on, then off, each a multiple of 20 from 20 to 5100.
        #[arg(long, num_args = 2, value_names = ["ON", "OFF"])]
        blink: Option<Vec<u64>>,
    },
    /// The DS18B20 probes on the ports.
    Thermometers {
        #[command(subcommand)]
        command: Option<ThermometersCommand>,
    },
    /// The firmware of the two microcontrollers: show it, or write it again.
    Firmware {
        #[command(subcommand)]
        command: Option<FirmwareCommand>,
    },
    /// Check that the board's hardware is there as Linux should see it, and say what to do
    /// where it is not.
    Doctor,
    /// The components of the FIBER platform, and the file that chooses them.
    Platform {
        #[command(subcommand)]
        command: PlatformCommand,
    },
}

#[derive(Subcommand)]
enum ThermometersCommand {
    /// A line for every change, until Ctrl+C.
    Watch,
}

#[derive(Subcommand)]
enum FirmwareCommand {
    /// Both chips: what runs on them, and whether it is the image the agent carries.
    Status,
    /// Writes the image the agent carries to a chip, through the agent.
    Flash {
        #[arg(value_parser = ["south", "north"])]
        chip: String,
    },
    /// Asks a chip's bootloader what the chip is, without the agent: stop the agent first.
    Chip {
        #[arg(value_parser = ["south", "north"])]
        chip: String,
    },
}

#[derive(Subcommand)]
enum PlatformCommand {
    /// Show every component: on, off or left alone, and what it requires.
    List,
    /// Show the release, and the last result of each component.
    Status,
    /// Turn components on in /etc/fiber/platform.yaml, with what they require.
    Enable {
        #[arg(required = true, value_name = "COMPONENT")]
        components: Vec<String>,
    },
    /// Turn components off in /etc/fiber/platform.yaml. Their data stays.
    Disable {
        #[arg(required = true, value_name = "COMPONENT")]
        components: Vec<String>,
    },
    /// Bring the FIBER to /etc/fiber/platform.yaml: install, configure and start what is on,
    /// stop and turn off what is off. Runs as a service and follows it.
    Apply {
        /// Do the work in this process instead of the service (the service runs this).
        #[arg(long)]
        foreground: bool,
    },
    /// Install a newer release (the newest, or the one named) with its own installer.
    Update {
        /// The release to install, as in 1.2.0.
        #[arg(long, value_name = "VERSION")]
        to: Option<String>,
    },
    /// Go back to the release before, and apply it.
    Rollback,
    /// Take the FIBER platform off this system. The services' data stays.
    Uninstall {
        /// Yes, really.
        #[arg(long)]
        yes: bool,
    },
    /// Take the choice and the login from a seed file written by FIBER Setup. Writes
    /// /etc/fiber/platform.yaml and the login only where they are not there yet.
    Seed {
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let paths = Paths::new(cli.root);
    let broker = || client::Broker::new(cli.broker.as_deref(), cli.user.clone());
    let result = match cli.command {
        Command::Version { all } => {
            print!("{}", version_text(all));
            Ok(ExitCode::SUCCESS)
        }
        Command::Agent => agent::agent(&paths),
        Command::SouthSimulator { port } => agent::south_simulator(&port),
        Command::Status => broker().and_then(|b| client::status(&b)),
        Command::Power { follow } => broker().and_then(|b| client::power(&b, follow)),
        Command::Thermometers { command } => {
            broker().and_then(|b| client::thermometers(&b, command.is_some()))
        }
        Command::Ports {
            action,
            ports,
            seconds,
        } => broker().and_then(|b| client::ports(&b, action.as_deref(), &ports, seconds)),
        Command::Leds { action, leds, blink } => broker().and_then(|b| {
            let blink = blink.and_then(|v| Some((*v.first()?, *v.get(1)?)));
            client::leds(&b, action.as_deref(), &leds, blink)
        }),
        Command::Firmware {
            command: Some(FirmwareCommand::Chip { chip }),
        } => agent::firmware_chip(&paths, &chip),
        Command::Firmware { command } => broker().and_then(|b| match command {
            Some(FirmwareCommand::Flash { chip }) => client::firmware(&b, Some(&chip)),
            _ => client::firmware(&b, None),
        }),
        Command::Doctor => doctor::doctor(&paths),
        Command::Platform { command } => match command {
            PlatformCommand::List => platform::list(&paths),
            PlatformCommand::Status => platform::status(&paths),
            PlatformCommand::Enable { components } => platform::change(&paths, &components, true),
            PlatformCommand::Disable { components } => platform::change(&paths, &components, false),
            PlatformCommand::Seed { file } => platform::seed(&paths, &file),
            PlatformCommand::Apply { foreground: true } => platform::apply_foreground(&paths),
            PlatformCommand::Apply { foreground: false } => platform::apply(&paths),
            PlatformCommand::Update { to } => platform::update(&paths, to.as_deref()),
            PlatformCommand::Rollback => platform::rollback(&paths),
            PlatformCommand::Uninstall { yes } => platform::uninstall(&paths, yes),
        },
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// What `fiber version` prints. Without `all`, the bare version on one line (FIBER Setup's
/// first-start script stores it as it is).
fn version_text(all: bool) -> String {
    let version = env!("CARGO_PKG_VERSION");
    if all {
        format!("fiber  {version}\ntower  {TOWER_REV}\nradio  0x{RADIO_TAG:04x}\n")
    } else {
        format!("{version}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn version_is_one_bare_line() {
        assert_eq!(version_text(false), concat!(env!("CARGO_PKG_VERSION"), "\n"));
    }

    /// The pin is a full commit id, and it is the revision the root `Cargo.toml` names.
    #[test]
    fn version_all_names_the_tower_pin() {
        assert_eq!(TOWER_REV.len(), 40);
        assert!(TOWER_REV.bytes().all(|b| b.is_ascii_hexdigit()));
        let manifest = include_str!("../../Cargo.toml");
        assert!(
            manifest.contains(&format!("rev = \"{TOWER_REV}\"")),
            "Cargo.toml and Cargo.lock disagree"
        );
        assert!(version_text(true).contains(TOWER_REV));
    }

    /// The front panel's LCD is 128 x 64 pixels: `tower_hmi::Frame<16, 64>` (16 bytes a row).
    #[test]
    fn lcd_frame_type() {
        assert_eq!(tower_hmi::Frame::<16, 64>::WIDTH, 128);
        assert_eq!(tower_hmi::Frame::<16, 64>::HEIGHT, 64);
    }
}
