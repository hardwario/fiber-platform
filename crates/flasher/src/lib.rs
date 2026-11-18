//! Writing a microcontroller's firmware from the CM4, through its ROM UART bootloader (ST AN3155),
//! in a way that cannot leave it dead (`docs/hardware.md`, "Flashing and recovery").
//!
//! - **The chips** it knows: the south bridge (STM32C031, product id `0x453`) and the north
//!   bridge (STM32WL55, `0x497`). Any other id is refused before anything is erased.
//! - **The lines** are behind [`Lines`]: BOOT0 is driven; NRST is pulled low or let go, never
//!   driven high, so the chip can still pull its own reset. **The serial port** is behind
//!   [`Port`]: even parity for the bootloader, none for the firmware.
//! - **The image is checked** before anything is erased: its size, where its stack starts, where
//!   its reset vector points.
//! - **The block with the vector table is written last,** after everything else was written,
//!   read back and compared. A write cut short leaves the first block empty, so the chip waits
//!   in its bootloader at the next start, never runs half a program.
//! - **It leaves through `Go`,** not through a reset: after a first write the chip still believes
//!   its memory is empty, and a reset would land in the bootloader again.
//! - **The option bytes are read and reported,** and a chip with readout protection is refused.
//!   The flasher never writes them.
//!
//! The adapters for a real serial port and real GPIO lines are the agent's (T17); [`sim`] is a
//! simulated chip for the tests.

pub mod sim;

use std::fmt;
use std::io;
use std::time::Duration;

const ACK: u8 = 0x79;
const NACK: u8 = 0x1F;
const START: u8 = 0x7F;

const GET: u8 = 0x00;
const GET_ID: u8 = 0x02;
const READ_MEMORY: u8 = 0x11;
const GO: u8 = 0x21;
const WRITE_MEMORY: u8 = 0x31;
const EXTENDED_ERASE: u8 = 0x44;

/// `FLASH_OPTR` as the system memory holds it, on both chips.
pub const OPTION_BYTES: u32 = 0x1FFF_7800;

/// The most one Read Memory or Write Memory moves.
const BLOCK: usize = 256;

/// How long to wait for a byte the bootloader sends at once.
const ANSWER: Duration = Duration::from_millis(500);

/// A chip the flasher knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chip {
    pub name: &'static str,
    pub product_id: u16,
    pub flash_start: u32,
    pub flash_size: u32,
    pub page_size: u32,
    pub ram_start: u32,
    pub ram_end: u32,
    /// Writes must be a multiple of this (the flash's word: 8 bytes on both chips).
    pub write_unit: usize,
}

/// The south bridge: 32 KB of flash in 16 pages of 2 KB, 12 KB of RAM (RM0490).
pub const STM32C031: Chip = Chip {
    name: "STM32C031",
    product_id: 0x453,
    flash_start: 0x0800_0000,
    flash_size: 32 * 1024,
    page_size: 2048,
    ram_start: 0x2000_0000,
    ram_end: 0x2000_3000,
    write_unit: 8,
};

/// The north bridge: 256 KB of flash in 128 pages of 2 KB, 64 KB of RAM (RM0453).
pub const STM32WL55: Chip = Chip {
    name: "STM32WL55",
    product_id: 0x497,
    flash_start: 0x0800_0000,
    flash_size: 256 * 1024,
    page_size: 2048,
    ram_start: 0x2000_0000,
    ram_end: 0x2001_0000,
    write_unit: 8,
};

/// The serial port to the chip.
pub trait Port {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Fills `buf`, or fails with `TimedOut` when the bytes do not come in `timeout`.
    fn read_exact(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<()>;
    /// Even parity for the bootloader; none for the firmware.
    fn set_even_parity(&mut self, even: bool) -> io::Result<()>;
    /// Throws away what has come in and not been read.
    fn discard_input(&mut self) -> io::Result<()>;
}

/// The chip's BOOT0 and NRST lines, and the passing of time.
pub trait Lines {
    fn set_boot0(&mut self, high: bool) -> io::Result<()>;
    /// Pulls NRST low.
    fn hold_reset(&mut self) -> io::Result<()>;
    /// Lets NRST go, to its pull-up. Never drives it high.
    fn release_reset(&mut self) -> io::Result<()>;
    fn sleep(&mut self, duration: Duration);
}

/// What happens, for the caller to show. The flasher prints nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Entering { attempt: u32 },
    Identified { product_id: u16 },
    Erasing { pages: usize },
    Writing { done: usize, total: usize },
    Verifying { done: usize, total: usize },
    WritingVectorTable,
    Starting,
}

/// Why it stopped.
#[derive(Debug)]
pub enum Error {
    /// The bootloader did not answer the start byte, after every attempt.
    NoAnswer { attempts: u32 },
    /// The bootloader refused something (NACK).
    Refused(&'static str),
    /// A byte that is neither ACK nor NACK.
    Unexpected { what: &'static str, byte: u8 },
    /// The serial port or the lines failed.
    Io(io::Error),
    /// Another chip answered than the one expected.
    WrongChip { found: u16, expected: u16 },
    /// Readout protection is on: the bootloader would erase everything to take it off, and only
    /// SWD can do that safely.
    ReadoutProtected { level: u8 },
    /// The image does not look like a firmware for this chip.
    BadImage(String),
    /// What was read back differs from what was written.
    Mismatch { address: u32 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::NoAnswer { attempts } => {
                write!(
                    f,
                    "the bootloader does not answer ({attempts} attempts with BOOT0 high and a reset)"
                )
            }
            Error::Refused(what) => write!(f, "the bootloader refused {what}"),
            Error::Unexpected { what, byte } => {
                write!(f, "unexpected byte 0x{byte:02x} in place of the answer to {what}")
            }
            Error::Io(e) => write!(f, "{e}"),
            Error::WrongChip { found, expected } => {
                write!(
                    f,
                    "the chip is 0x{found:03x}, not 0x{expected:03x}; nothing was erased"
                )
            }
            Error::ReadoutProtected { level } => write!(
                f,
                "the chip has readout protection (0x{level:02x}); taking it off erases the chip and is for SWD, not for the CM4"
            ),
            Error::BadImage(why) => write!(
                f,
                "the image is not a firmware for this chip: {why}; nothing was erased"
            ),
            Error::Mismatch { address } => {
                write!(
                    f,
                    "the flash differs from the image at 0x{address:08x} after writing"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// The option bytes (`FLASH_OPTR`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptionBytes(pub u32);

impl OptionBytes {
    /// Readout protection: `0xAA` is level 0, off.
    pub fn rdp(self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    pub fn readout_protected(self) -> bool {
        self.rdp() != 0xAA
    }

    /// Bit `bit` of the register.
    pub fn bit(self, bit: u32) -> bool {
        self.0 >> bit & 1 == 1
    }
}

/// What the bootloader says about the chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The bootloader's version: `0x31` is 3.1.
    pub version: u8,
    pub commands: Vec<u8>,
    pub product_id: u16,
    pub option_bytes: OptionBytes,
}

/// Checks that an image is a firmware for `chip`: not empty, it fits, its first word is a stack
/// in the chip's RAM, and its second points into the image, at Thumb code.
pub fn check_image(chip: &Chip, image: &[u8]) -> Result<()> {
    if image.len() < 8 {
        return Err(Error::BadImage("it is shorter than a vector table".into()));
    }
    if image.len() > chip.flash_size as usize {
        return Err(Error::BadImage(format!(
            "it is {} bytes and the flash {}",
            image.len(),
            chip.flash_size
        )));
    }
    let word = |at: usize| u32::from_le_bytes([image[at], image[at + 1], image[at + 2], image[at + 3]]);
    let stack = word(0);
    if !(chip.ram_start..=chip.ram_end).contains(&stack) {
        return Err(Error::BadImage(format!(
            "its stack starts at 0x{stack:08x}, outside the RAM"
        )));
    }
    let reset = word(4);
    let end = chip.flash_start + image.len() as u32;
    if reset & 1 == 0 || !(chip.flash_start..end).contains(&(reset & !1)) {
        return Err(Error::BadImage(format!(
            "its reset vector 0x{reset:08x} does not point into it"
        )));
    }
    Ok(())
}

fn xor(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |a, b| a ^ b)
}

/// The bootloader of one chip, behind a serial port and two lines.
pub struct Flasher<P: Port, L: Lines> {
    port: P,
    lines: L,
    chip: Chip,
}

impl<P: Port, L: Lines> Flasher<P, L> {
    pub fn new(port: P, lines: L, chip: Chip) -> Self {
        Self { port, lines, chip }
    }

    /// The port and the lines back.
    pub fn into_parts(self) -> (P, L) {
        (self.port, self.lines)
    }

    fn read(&mut self, count: usize, timeout: Duration) -> Result<Vec<u8>> {
        let mut buf = vec![0; count];
        self.port.read_exact(&mut buf, timeout)?;
        Ok(buf)
    }

    fn expect_ack(&mut self, what: &'static str, timeout: Duration) -> Result<()> {
        match self.read(1, timeout)?[0] {
            ACK => Ok(()),
            NACK => Err(Error::Refused(what)),
            byte => Err(Error::Unexpected { what, byte }),
        }
    }

    fn command(&mut self, code: u8, what: &'static str) -> Result<()> {
        self.port.write(&[code, code ^ 0xFF])?;
        self.expect_ack(what, ANSWER)
    }

    fn address(&mut self, address: u32, what: &'static str) -> Result<()> {
        let bytes = address.to_be_bytes();
        self.port
            .write(&[bytes[0], bytes[1], bytes[2], bytes[3], xor(&bytes)])?;
        self.expect_ack(what, ANSWER)
    }

    /// Resets the chip with BOOT0 high and waits for its bootloader: three attempts, with BOOT0
    /// held longer each time (50, 200 and 500 ms after the reset).
    pub fn enter(&mut self, progress: &mut dyn FnMut(Progress)) -> Result<()> {
        self.port.set_even_parity(true)?;
        for (attempt, wait) in [(1, 50), (2, 200), (3, 500)] {
            progress(Progress::Entering { attempt });
            self.lines.set_boot0(true)?;
            self.lines.hold_reset()?;
            self.lines.sleep(Duration::from_millis(20));
            self.lines.release_reset()?;
            self.lines.sleep(Duration::from_millis(wait));
            self.port.discard_input()?;
            self.port.write(&[START])?;
            match self.expect_ack("the start byte", ANSWER) {
                Ok(()) => return Ok(()),
                Err(Error::Io(e)) if e.kind() == io::ErrorKind::TimedOut => continue,
                // A bootloader that is already listening (an earlier start byte got through)
                // refuses a second start byte: it is there.
                Err(Error::Refused(_)) => return Ok(()),
                Err(e) => return Err(e),
            }
        }
        self.lines.set_boot0(false)?;
        Err(Error::NoAnswer { attempts: 3 })
    }

    /// Asks the bootloader what it is, and reads the option bytes.
    pub fn identify(&mut self) -> Result<Identity> {
        self.command(GET, "Get")?;
        let count = self.read(1, ANSWER)?[0] as usize;
        let body = self.read(count + 1, ANSWER)?;
        self.expect_ack("the end of Get", ANSWER)?;
        self.command(GET_ID, "Get ID")?;
        let count = self.read(1, ANSWER)?[0] as usize;
        let id = self.read(count + 1, ANSWER)?;
        self.expect_ack("the end of Get ID", ANSWER)?;
        let product_id = id.iter().fold(0u32, |a, b| a << 8 | u32::from(*b)) as u16;
        let option_bytes = match self.read_memory(OPTION_BYTES, 4) {
            Ok(bytes) => OptionBytes(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
            // Read Memory is refused only while readout protection is on.
            Err(Error::Refused(_)) => OptionBytes(0),
            Err(e) => return Err(e),
        };
        Ok(Identity {
            version: body[0],
            commands: body[1..].to_vec(),
            product_id,
            option_bytes,
        })
    }

    fn read_memory(&mut self, address: u32, count: usize) -> Result<Vec<u8>> {
        self.command(READ_MEMORY, "Read Memory")?;
        self.address(address, "the address to read")?;
        let n = (count - 1) as u8;
        self.port.write(&[n, n ^ 0xFF])?;
        self.expect_ack("the length to read", ANSWER)?;
        self.read(count, ANSWER)
    }

    fn write_memory(&mut self, address: u32, data: &[u8]) -> Result<()> {
        self.command(WRITE_MEMORY, "Write Memory")?;
        self.address(address, "the address to write")?;
        let n = (data.len() - 1) as u8;
        let mut frame = Vec::with_capacity(data.len() + 2);
        frame.push(n);
        frame.extend_from_slice(data);
        frame.push(xor(&frame));
        self.port.write(&frame)?;
        self.expect_ack("the data to write", Duration::from_secs(1))
    }

    fn erase(&mut self, pages: &[u16]) -> Result<()> {
        self.command(EXTENDED_ERASE, "Extended Erase")?;
        let mut frame = Vec::with_capacity(pages.len() * 2 + 3);
        frame.extend_from_slice(&((pages.len() - 1) as u16).to_be_bytes());
        for page in pages {
            frame.extend_from_slice(&page.to_be_bytes());
        }
        frame.push(xor(&frame));
        self.port.write(&frame)?;
        // About 40 ms a page at most (datasheets), with room to spare.
        self.expect_ack(
            "the pages to erase",
            Duration::from_millis(500 + 100 * pages.len() as u64),
        )
    }

    fn go(&mut self) -> Result<()> {
        self.command(GO, "Go")?;
        let start = self.chip.flash_start;
        self.address(start, "the address to go to")
    }

    /// Writes `image` and starts it: into the bootloader, check, erase, write all but the first
    /// block, compare, write the first block, compare, `Go`. The serial port is back at no parity
    /// for the firmware when it returns, also after an error.
    pub fn flash(&mut self, image: &[u8], progress: &mut dyn FnMut(Progress)) -> Result<()> {
        check_image(&self.chip, image)?;
        let result = self.flash_checked(image, progress);
        let _ = self.lines.set_boot0(false);
        let _ = self.port.set_even_parity(false);
        result
    }

    fn flash_checked(&mut self, image: &[u8], progress: &mut dyn FnMut(Progress)) -> Result<()> {
        self.enter(progress)?;
        let identity = self.identify()?;
        progress(Progress::Identified {
            product_id: identity.product_id,
        });
        if identity.product_id != self.chip.product_id {
            return Err(Error::WrongChip {
                found: identity.product_id,
                expected: self.chip.product_id,
            });
        }
        if identity.option_bytes.readout_protected() {
            return Err(Error::ReadoutProtected {
                level: identity.option_bytes.rdp(),
            });
        }

        // The image in blocks of 256 bytes, the last one padded to the flash's word with 0xFF.
        let unit = self.chip.write_unit;
        let mut padded = image.to_vec();
        padded.resize(image.len().div_ceil(unit) * unit, 0xFF);
        let blocks: Vec<(u32, &[u8])> = padded
            .chunks(BLOCK)
            .enumerate()
            .map(|(i, data)| (self.chip.flash_start + (i * BLOCK) as u32, data))
            .collect();

        let pages: Vec<u16> = (0..padded.len().div_ceil(self.chip.page_size as usize) as u16).collect();
        progress(Progress::Erasing { pages: pages.len() });
        self.erase(&pages)?;

        let total = blocks.len();
        for (done, (address, data)) in blocks.iter().enumerate().skip(1) {
            progress(Progress::Writing { done, total });
            self.write_memory(*address, data)?;
        }
        for (done, (address, data)) in blocks.iter().enumerate().skip(1) {
            progress(Progress::Verifying { done, total });
            self.compare(*address, data)?;
        }
        progress(Progress::WritingVectorTable);
        let (address, data) = blocks[0];
        self.write_memory(address, data)?;
        self.compare(address, data)?;

        progress(Progress::Starting);
        self.lines.set_boot0(false)?;
        self.go()?;
        Ok(())
    }

    fn compare(&mut self, address: u32, data: &[u8]) -> Result<()> {
        let read = self.read_memory(address, data.len())?;
        match read.iter().zip(data).position(|(a, b)| a != b) {
            None => Ok(()),
            Some(at) => Err(Error::Mismatch {
                address: address + at as u32,
            }),
        }
    }

    /// Resets the chip into its firmware: BOOT0 low, a pulse on NRST, the port at no parity.
    pub fn reset_to_firmware(&mut self) -> Result<()> {
        self.lines.set_boot0(false)?;
        self.lines.hold_reset()?;
        self.lines.sleep(Duration::from_millis(20));
        self.lines.release_reset()?;
        self.port.set_even_parity(false)?;
        Ok(())
    }
}

/// Keeps others off a serial port while it is held: the agent holds it while it runs, so a
/// command run by hand is refused instead of talking over it. A lock file in `/run/lock`.
pub struct PortLock {
    _file: std::fs::File,
}

impl PortLock {
    /// Takes the lock for `port` (as in `/dev/ttyAMA4`), in `dir`; fails at once when another
    /// process has it.
    pub fn take(dir: &std::path::Path, port: &str) -> io::Result<PortLock> {
        let name = port.rsplit('/').next().unwrap_or(port);
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("fiber-{name}.lock"));
        // Read-only and without O_CREAT when it is there: the file may be another user's (the
        // agent runs as `fiber`, a command by hand as root), and `/run/lock` is sticky, where the
        // kernel refuses O_CREAT on another user's file (`fs.protected_regular`).
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)?,
            Err(e) => return Err(e),
        };
        match file.try_lock() {
            Ok(()) => Ok(PortLock { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("{port} is in use by another program (the agent?)"),
            )),
            Err(std::fs::TryLockError::Error(e)) => Err(e),
        }
    }
}
