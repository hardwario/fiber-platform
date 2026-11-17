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
    /// The components of the FIBER platform, and the file that chooses them.
    Platform {
        #[command(subcommand)]
        command: PlatformCommand,
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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let paths = Paths::new(cli.root);
    let result = match cli.command {
        Command::Version { all } => {
            print!("{}", version_text(all));
            Ok(ExitCode::SUCCESS)
        }
        Command::Platform { command } => match command {
            PlatformCommand::List => platform::list(&paths),
            PlatformCommand::Status => platform::status(&paths),
            PlatformCommand::Enable { components } => platform::change(&paths, &components, true),
            PlatformCommand::Disable { components } => platform::change(&paths, &components, false),
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
