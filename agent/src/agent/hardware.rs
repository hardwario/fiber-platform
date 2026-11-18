//! The flasher's adapters to the real board: the serial port, blocking, and the BOOT0 and NRST
//! lines through the kernel's GPIO character device (Linux only). And `fiber firmware chip`,
//! which uses them without the agent.

use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::anyhow;
#[cfg(target_os = "linux")]
use fiber_flasher::Lines;
use fiber_flasher::{Chip, Flasher, Identity, Port, PortLock};
use tokio_serial::{ClearBuffer, Parity, SerialPort};

use crate::board;
use crate::paths::Paths;

/// A serial port for the flasher.
pub struct SerialAdapter(pub Box<dyn SerialPort>);

impl SerialAdapter {
    /// Opens `path` at 115200 baud, as the bootloader wants it (the flasher sets the parity).
    pub fn open(path: &str) -> io::Result<SerialAdapter> {
        let port = tokio_serial::new(path, 115_200)
            .timeout(Duration::from_millis(100))
            .open()?;
        Ok(SerialAdapter(port))
    }
}

impl Port for SerialAdapter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes)?;
        self.0.flush()
    }

    fn read_exact(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<()> {
        self.0.set_timeout(timeout)?;
        let mut done = 0;
        while done < buf.len() {
            match self.0.read(&mut buf[done..]) {
                Ok(0) => return Err(io::ErrorKind::TimedOut.into()),
                Ok(n) => done += n,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn set_even_parity(&mut self, even: bool) -> io::Result<()> {
        self.0
            .set_parity(if even { Parity::Even } else { Parity::None })
            .map_err(io::Error::other)
    }

    fn discard_input(&mut self) -> io::Result<()> {
        self.0.clear(ClearBuffer::Input).map_err(io::Error::other)
    }
}

/// A chip's BOOT0 and NRST lines: BOOT0 driven, NRST open-drain (pulled low, or let go to its
/// pull-up, never driven high).
#[cfg(target_os = "linux")]
pub struct GpioLines {
    boot0: gpiocdev::Request,
    nrst: gpiocdev::Request,
    boot0_line: u32,
    nrst_line: u32,
}

#[cfg(target_os = "linux")]
impl GpioLines {
    pub fn new(chip: &str, boot0: u32, nrst: u32) -> io::Result<GpioLines> {
        use gpiocdev::line::{Drive, Value};
        let err = |e: gpiocdev::Error| io::Error::other(format!("GPIO: {e}"));
        let boot0_request = gpiocdev::Request::builder()
            .on_chip(chip)
            .with_consumer("fiber-flasher")
            .with_line(boot0)
            .as_output(Value::Inactive)
            .request()
            .map_err(err)?;
        let nrst_request = gpiocdev::Request::builder()
            .on_chip(chip)
            .with_consumer("fiber-flasher")
            .with_line(nrst)
            .as_output(Value::Active)
            .with_drive(Drive::OpenDrain)
            .request()
            .map_err(err)?;
        Ok(GpioLines {
            boot0: boot0_request,
            nrst: nrst_request,
            boot0_line: boot0,
            nrst_line: nrst,
        })
    }
}

#[cfg(target_os = "linux")]
impl Lines for GpioLines {
    fn set_boot0(&mut self, high: bool) -> io::Result<()> {
        use gpiocdev::line::Value;
        let value = if high { Value::Active } else { Value::Inactive };
        self.boot0
            .set_value(self.boot0_line, value)
            .map(|_| ())
            .map_err(|e| io::Error::other(format!("GPIO: {e}")))
    }

    fn hold_reset(&mut self) -> io::Result<()> {
        use gpiocdev::line::Value;
        self.nrst
            .set_value(self.nrst_line, Value::Inactive)
            .map(|_| ())
            .map_err(|e| io::Error::other(format!("GPIO: {e}")))
    }

    fn release_reset(&mut self) -> io::Result<()> {
        use gpiocdev::line::Value;
        // Open-drain: "high" lets the line go to its pull-up.
        self.nrst
            .set_value(self.nrst_line, Value::Active)
            .map(|_| ())
            .map_err(|e| io::Error::other(format!("GPIO: {e}")))
    }

    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// The three buttons: inputs with the pull-ups, pressed = low, with the kernel's debounce.
#[cfg(target_os = "linux")]
pub struct GpioButtons(gpiocdev::Request);

#[cfg(target_os = "linux")]
impl GpioButtons {
    const LINES: [(u32, u8); 3] = [
        (board::gpio::BUTTON_UP, super::buttons::UP),
        (board::gpio::BUTTON_ENTER, super::buttons::ENTER),
        (board::gpio::BUTTON_DOWN, super::buttons::DOWN),
    ];

    pub fn new() -> io::Result<GpioButtons> {
        use gpiocdev::line::{Bias, EdgeDetection};
        let offsets: Vec<u32> = Self::LINES.iter().map(|(line, _)| *line).collect();
        gpiocdev::Request::builder()
            .on_chip(board::GPIO_CHIP)
            .with_consumer("fiber-buttons")
            .with_lines(&offsets)
            .as_input()
            .as_active_low()
            .with_bias(Bias::PullUp)
            .with_edge_detection(EdgeDetection::BothEdges)
            .with_debounce_period(Duration::from_millis(5))
            .request()
            .map(GpioButtons)
            .map_err(|e| io::Error::other(format!("GPIO: {e}")))
    }
}

#[cfg(target_os = "linux")]
impl super::buttons::Lines for GpioButtons {
    fn wait(&mut self, timeout: Duration) -> io::Result<()> {
        let err = |e: gpiocdev::Error| io::Error::other(format!("GPIO: {e}"));
        if self.0.wait_edge_event(timeout).map_err(err)? {
            // The edges only wake the thread; the values are read afterwards.
            while self.0.has_edge_event().map_err(err)? {
                self.0.read_edge_event().map_err(err)?;
            }
        }
        Ok(())
    }

    fn read(&mut self) -> io::Result<u8> {
        use gpiocdev::line::{Value, Values};
        let mut values = Values::default();
        self.0
            .values(&mut values)
            .map_err(|e| io::Error::other(format!("GPIO: {e}")))?;
        Ok(Self::LINES
            .iter()
            .filter(|(line, _)| values.get(*line) == Some(Value::Active))
            .fold(0, |mask, (_, bit)| mask | bit))
    }
}

/// The buzzer's line: it sounds while GPIO17 is low.
#[cfg(target_os = "linux")]
pub struct GpioBuzzer(gpiocdev::Request);

#[cfg(target_os = "linux")]
impl GpioBuzzer {
    pub fn new() -> io::Result<GpioBuzzer> {
        use gpiocdev::line::Value;
        gpiocdev::Request::builder()
            .on_chip(board::GPIO_CHIP)
            .with_consumer("fiber-buzzer")
            .with_line(board::gpio::BUZZER)
            .as_active_low()
            .as_output(Value::Inactive)
            .request()
            .map(GpioBuzzer)
            .map_err(|e| io::Error::other(format!("GPIO: {e}")))
    }
}

#[cfg(target_os = "linux")]
impl super::buzzer::Line for GpioBuzzer {
    fn sound(&mut self, on: bool) {
        use gpiocdev::line::Value;
        let value = if on { Value::Active } else { Value::Inactive };
        // A failed write leaves the buzzer as it was; the next one tries again.
        let _ = self.0.set_value(board::gpio::BUZZER, value);
    }
}

/// A chip as the command line names it, with its serial port and its two lines.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub chip: Chip,
    pub port: &'static str,
    pub boot0: u32,
    pub reset: u32,
}

/// `south` or `north`.
pub fn target(name: &str) -> Option<Target> {
    match name {
        "south" => Some(Target {
            chip: fiber_flasher::STM32C031,
            port: board::SOUTH_BRIDGE_UART,
            boot0: board::gpio::SOUTH_BOOT0,
            reset: board::gpio::SOUTH_RESET,
        }),
        "north" => Some(Target {
            chip: fiber_flasher::STM32WL55,
            port: board::NORTH_BRIDGE_UART,
            boot0: board::gpio::NORTH_BOOT0,
            reset: board::gpio::NORTH_RESET,
        }),
        _ => None,
    }
}

/// The flasher on the real port and lines of `target`.
#[cfg(target_os = "linux")]
fn open(target: &Target) -> Result<Flasher<SerialAdapter, GpioLines>, String> {
    let port = SerialAdapter::open(target.port).map_err(|e| format!("{}: {e}", target.port))?;
    let lines = GpioLines::new(board::GPIO_CHIP, target.boot0, target.reset).map_err(|e| e.to_string())?;
    Ok(Flasher::new(port, lines, target.chip))
}

#[cfg(not(target_os = "linux"))]
fn open(_target: &Target) -> Result<Flasher<SerialAdapter, NoLines>, String> {
    Err("the GPIO lines are only there on Linux".into())
}

/// No lines: only for a build on another system than Linux, where [`open`] always fails.
#[cfg(not(target_os = "linux"))]
pub struct NoLines;

#[cfg(not(target_os = "linux"))]
impl fiber_flasher::Lines for NoLines {
    fn set_boot0(&mut self, _high: bool) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn hold_reset(&mut self) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn release_reset(&mut self) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn sleep(&mut self, _duration: Duration) {}
}

/// Writes `image` to the south bridge through its bootloader. Blocking: run it off the agent's
/// loop.
pub fn flash_south(image: &[u8]) -> Result<(), String> {
    let target = target("south").ok_or("no south bridge")?;
    let mut flasher = open(&target)?;
    flasher
        .flash(image, &mut |progress| {
            eprintln!("flashing the south bridge: {progress:?}")
        })
        .map_err(|e| e.to_string())
}

/// Starts the chip's bootloader, asks it what it is, and starts the firmware again, also when
/// the asking failed.
pub fn read_chip<P: Port, L: fiber_flasher::Lines>(
    flasher: &mut Flasher<P, L>,
) -> fiber_flasher::Result<Identity> {
    let identity = flasher.enter(&mut |_| {}).and_then(|()| flasher.identify());
    let reset = flasher.reset_to_firmware();
    let identity = identity?;
    reset?;
    Ok(identity)
}

/// What `fiber firmware chip` prints.
pub fn chip_text(chip: &Chip, identity: &Identity) -> String {
    let expected = if identity.product_id == chip.product_id {
        format!("the {} expected", chip.name)
    } else {
        format!("not the {} expected, 0x{:03x}", chip.name, chip.product_id)
    };
    let commands: Vec<String> = identity.commands.iter().map(|c| format!("{c:02x}")).collect();
    let options = identity.option_bytes;
    let (options, protection) = if options.readout_protected() {
        (
            "not readable".to_owned(),
            "on: the bootloader can neither read nor write the flash".to_owned(),
        )
    } else {
        (format!("0x{:08x}", options.0), "off".to_owned())
    };
    format!(
        "Chip        product id 0x{:03x} ({expected})\n\
         Bootloader  {}.{}, commands {}\n\
         Options     {options}\n\
         Protection  {protection}\n",
        identity.product_id,
        identity.version >> 4,
        identity.version & 0x0f,
        commands.join(" "),
    )
}

/// `fiber firmware chip south|north`: what the chip's bootloader says about it. It needs the
/// serial port to itself, so it refuses while the agent runs.
pub fn firmware_chip(paths: &Paths, name: &str) -> anyhow::Result<ExitCode> {
    let target = target(name).ok_or_else(|| anyhow!("no chip called {name}: south or north"))?;
    let _lock = PortLock::take(&paths.lock_dir(), target.port)
        .map_err(|e| anyhow!("{e}; stop the agent first: sudo systemctl stop fiber-agent"))?;
    let mut flasher = open(&target).map_err(|e| anyhow!(e))?;
    let identity = read_chip(&mut flasher).map_err(|e| anyhow!("the {name} bridge's bootloader: {e}"))?;
    print!("{}", chip_text(&target.chip, &identity));
    Ok(if identity.product_id == target.chip.product_id {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fiber_flasher::sim::{Running, Sim};

    /// A chip with a program on it: a stack in its RAM and a reset vector into its flash.
    fn programmed() -> Sim {
        let sim = Sim::new(fiber_flasher::STM32C031);
        {
            let mut chip = sim.0.borrow_mut();
            chip.flash[..8].copy_from_slice(&[0x00, 0x30, 0x00, 0x20, 0xc1, 0x00, 0x00, 0x08]);
            chip.power_cycle();
        }
        sim
    }

    #[test]
    fn a_chip_is_read_and_left_running_its_firmware() {
        let sim = programmed();
        assert_eq!(sim.0.borrow().running, Running::Firmware);
        let mut flasher = Flasher::new(sim.clone(), sim.clone(), fiber_flasher::STM32C031);
        let identity = read_chip(&mut flasher).unwrap();
        assert_eq!(identity.product_id, 0x453);
        assert_eq!(sim.0.borrow().running, Running::Firmware);
        assert!(!sim.0.borrow().boot0);
        let text = chip_text(&fiber_flasher::STM32C031, &identity);
        assert!(
            text.starts_with("Chip        product id 0x453 (the STM32C031 expected)\n"),
            "{text}"
        );
        assert!(
            text.contains("\nOptions     0xfffffeaa\nProtection  off\n"),
            "{text}"
        );
    }

    #[test]
    fn another_chip_and_readout_protection_are_named() {
        let sim = Sim::new(fiber_flasher::STM32C031);
        sim.0.borrow_mut().option_bytes = 0xFFFF_FEBB;
        let mut flasher = Flasher::new(sim.clone(), sim.clone(), fiber_flasher::STM32C031);
        let identity = read_chip(&mut flasher).unwrap();
        let text = chip_text(&fiber_flasher::STM32WL55, &identity);
        assert!(text.contains("(not the STM32WL55 expected, 0x497)"), "{text}");
        assert!(text.contains("Protection  on: "), "{text}");
    }

    #[test]
    fn a_silent_bootloader_still_gets_its_firmware_started() {
        let sim = Sim::new(fiber_flasher::STM32C031);
        sim.0.borrow_mut().silent_attempts = 5;
        let mut flasher = Flasher::new(sim.clone(), sim.clone(), fiber_flasher::STM32C031);
        assert!(read_chip(&mut flasher).is_err());
        assert!(!sim.0.borrow().boot0);
        assert!(!sim.0.borrow().reset_held);
    }

    #[test]
    fn the_names_are_south_and_north() {
        assert_eq!(target("south").map(|t| t.chip.name), Some("STM32C031"));
        assert_eq!(target("north").map(|t| t.chip.name), Some("STM32WL55"));
        assert!(target("east").is_none());
    }
}
