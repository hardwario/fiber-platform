//! A simulated chip with its ROM bootloader, for the tests of the flasher and of the agent.
//!
//! It models what the flasher relies on: BOOT0 and NRST; the "empty" flag that the chip sets at
//! power-on when its flash is erased and does not look at again on a reset (so after a first
//! write a reset lands in the bootloader again); the bootloader's start byte, its commands and
//! the refusals a real one gives (readout protection, a bad checksum, a write over flash that is
//! not erased); and the faults to test against: a bootloader that does not answer at first or at
//! all, a power cut after some writes, a byte that reads back wrong.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;
use std::time::Duration;

use crate::{Chip, Lines, OPTION_BYTES, Port};

const ACK: u8 = 0x79;
const NACK: u8 = 0x1F;

/// What the chip runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Running {
    /// Held in reset, or without power after a cut.
    Nothing,
    Firmware,
    /// The ROM bootloader; `listening` once it had its start byte.
    Bootloader {
        listening: bool,
    },
}

/// A command the bootloader is in the middle of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Command,
    ReadAddress,
    ReadLength { address: u32 },
    WriteAddress,
    WriteData { address: u32 },
    EraseCount,
    ErasePages { count: usize },
    GoAddress,
}

/// The chip.
#[derive(Debug)]
pub struct SimChip {
    pub chip: Chip,
    pub flash: Vec<u8>,
    pub option_bytes: u32,
    /// Set at power-on when the flash's first word is erased.
    pub empty: bool,
    pub boot0: bool,
    pub reset_held: bool,
    pub running: Running,
    /// The parity the host's port is at.
    pub even_parity: bool,
    /// The bootloader does not answer the first this many attempts to start it.
    pub silent_attempts: u32,
    /// The power goes after this many Write Memory commands.
    pub power_cut_after_writes: Option<usize>,
    /// Read Memory gives this address's byte wrong.
    pub bad_read_at: Option<u32>,
    pub writes: usize,
    pub attempts: u32,
    /// Every sleep the flasher asked for, in milliseconds.
    pub sleeps: Vec<u64>,
    pending: Pending,
    rx: Vec<u8>,
    tx: VecDeque<u8>,
}

impl SimChip {
    /// A chip with its flash erased, just powered on, readout protection off.
    pub fn new(chip: Chip) -> Self {
        let mut sim = SimChip {
            chip,
            flash: vec![0xFF; chip.flash_size as usize],
            option_bytes: 0xFFFF_FEAA,
            empty: true,
            boot0: false,
            reset_held: false,
            running: Running::Nothing,
            even_parity: false,
            silent_attempts: 0,
            power_cut_after_writes: None,
            bad_read_at: None,
            writes: 0,
            attempts: 0,
            sleeps: Vec::new(),
            pending: Pending::Command,
            rx: Vec::new(),
            tx: VecDeque::new(),
        };
        sim.power_cycle();
        sim
    }

    /// Power off and on: the empty flag is looked at again.
    pub fn power_cycle(&mut self) {
        self.empty = self.flash[..4].iter().all(|b| *b == 0xFF);
        self.boot(false);
    }

    fn boot(&mut self, by_reset: bool) {
        self.rx.clear();
        self.tx.clear();
        self.pending = Pending::Command;
        let _ = by_reset;
        self.running = if self.boot0 || self.empty {
            Running::Bootloader { listening: false }
        } else {
            Running::Firmware
        };
    }

    fn answer(&mut self, bytes: &[u8]) {
        self.tx.extend(bytes);
    }

    fn in_flash(&self, address: u32, len: usize) -> bool {
        let start = self.chip.flash_start;
        address >= start && (address - start) as usize + len <= self.flash.len()
    }

    fn take(&mut self, n: usize) -> Option<Vec<u8>> {
        (self.rx.len() >= n).then(|| self.rx.drain(..n).collect())
    }

    fn address(bytes: &[u8]) -> Option<u32> {
        let xor = bytes[..4].iter().fold(0, |a, b| a ^ b);
        (xor == bytes[4]).then(|| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Handles what came in, as far as it goes.
    fn step(&mut self) {
        loop {
            let progressed = match self.pending {
                Pending::Command => match self.take(2) {
                    None => false,
                    Some(c) if c[0] ^ c[1] != 0xFF => {
                        self.answer(&[NACK]);
                        true
                    }
                    Some(c) => {
                        self.command(c[0]);
                        true
                    }
                },
                Pending::ReadAddress => match self.take(5) {
                    None => false,
                    Some(b) => {
                        match Self::address(&b) {
                            Some(address) if self.in_flash(address, 1) || address == OPTION_BYTES => {
                                self.answer(&[ACK]);
                                self.pending = Pending::ReadLength { address };
                                return self.step();
                            }
                            _ => self.answer(&[NACK]),
                        }
                        self.pending = Pending::Command;
                        true
                    }
                },
                Pending::ReadLength { address } => match self.take(2) {
                    None => false,
                    Some(b) => {
                        self.pending = Pending::Command;
                        if b[0] ^ b[1] != 0xFF {
                            self.answer(&[NACK]);
                        } else {
                            let n = b[0] as usize + 1;
                            let mut data = if address == OPTION_BYTES {
                                self.option_bytes.to_le_bytes().to_vec()
                            } else {
                                let at = (address - self.chip.flash_start) as usize;
                                self.flash[at..(at + n).min(self.flash.len())].to_vec()
                            };
                            data.resize(n, 0xFF);
                            if let Some(bad) = self.bad_read_at
                                && bad >= address
                                && ((bad - address) as usize) < n
                            {
                                data[(bad - address) as usize] ^= 0x01;
                            }
                            self.answer(&[ACK]);
                            self.answer(&data);
                        }
                        true
                    }
                },
                Pending::WriteAddress => match self.take(5) {
                    None => false,
                    Some(b) => {
                        match Self::address(&b) {
                            Some(address) if self.in_flash(address, 1) => {
                                self.answer(&[ACK]);
                                self.pending = Pending::WriteData { address };
                                return self.step();
                            }
                            _ => self.answer(&[NACK]),
                        }
                        self.pending = Pending::Command;
                        true
                    }
                },
                Pending::WriteData { address } => {
                    let Some(&n) = self.rx.first() else { return };
                    let total = n as usize + 3;
                    match self.take(total) {
                        None => false,
                        Some(frame) => {
                            self.pending = Pending::Command;
                            self.write(address, &frame);
                            true
                        }
                    }
                }
                Pending::EraseCount => match self.take(2) {
                    None => false,
                    Some(b) => {
                        self.pending = Pending::ErasePages {
                            count: u16::from_be_bytes([b[0], b[1]]) as usize + 1,
                        };
                        self.rx.splice(0..0, b);
                        true
                    }
                },
                Pending::ErasePages { count } => match self.take(2 + count * 2 + 1) {
                    None => false,
                    Some(frame) => {
                        self.pending = Pending::Command;
                        let (body, xor) = frame.split_at(frame.len() - 1);
                        if body.iter().fold(0, |a, b| a ^ b) != xor[0] {
                            self.answer(&[NACK]);
                        } else {
                            let page = self.chip.page_size as usize;
                            for p in body[2..].chunks(2) {
                                let at = u16::from_be_bytes([p[0], p[1]]) as usize * page;
                                if at + page <= self.flash.len() {
                                    self.flash[at..at + page].fill(0xFF);
                                }
                            }
                            self.answer(&[ACK]);
                        }
                        true
                    }
                },
                Pending::GoAddress => match self.take(5) {
                    None => false,
                    Some(b) => {
                        self.pending = Pending::Command;
                        if Self::address(&b) == Some(self.chip.flash_start) {
                            self.answer(&[ACK]);
                            self.running = Running::Firmware;
                            return;
                        }
                        self.answer(&[NACK]);
                        true
                    }
                },
            };
            if !progressed {
                return;
            }
        }
    }

    fn command(&mut self, code: u8) {
        let protected = self.option_bytes & 0xFF != 0xAA;
        match code {
            0x00 => {
                let body = [
                    0x31, 0x00, 0x01, 0x02, 0x11, 0x21, 0x31, 0x44, 0x63, 0x73, 0x82, 0x92,
                ];
                self.answer(&[ACK, (body.len() - 1) as u8]);
                self.answer(&body);
                self.answer(&[ACK]);
            }
            0x02 => {
                let id = self.chip.product_id.to_be_bytes();
                self.answer(&[ACK, 1, id[0], id[1], ACK]);
            }
            0x11 if protected => self.answer(&[NACK]),
            0x11 => {
                self.answer(&[ACK]);
                self.pending = Pending::ReadAddress;
            }
            0x31 if protected => self.answer(&[NACK]),
            0x31 => {
                self.answer(&[ACK]);
                self.pending = Pending::WriteAddress;
            }
            0x44 if protected => self.answer(&[NACK]),
            0x44 => {
                self.answer(&[ACK]);
                self.pending = Pending::EraseCount;
            }
            0x21 => {
                self.answer(&[ACK]);
                self.pending = Pending::GoAddress;
            }
            _ => self.answer(&[NACK]),
        }
    }

    fn write(&mut self, address: u32, frame: &[u8]) {
        let (body, xor) = frame.split_at(frame.len() - 1);
        let data = &body[1..];
        if body.iter().fold(0, |a, b| a ^ b) != xor[0]
            || data.len() % self.chip.write_unit != 0
            || !self.in_flash(address, data.len())
        {
            self.answer(&[NACK]);
            return;
        }
        self.writes += 1;
        if self.power_cut_after_writes.is_some_and(|n| self.writes > n) {
            // The power goes in the middle of this write: nothing of it lands.
            self.running = Running::Nothing;
            return;
        }
        let at = (address - self.chip.flash_start) as usize;
        let target = &mut self.flash[at..at + data.len()];
        // Flash can only be programmed where it is erased.
        if target
            .iter()
            .zip(data)
            .any(|(old, new)| *old != 0xFF && old != new)
        {
            self.answer(&[NACK]);
            return;
        }
        target.copy_from_slice(data);
        self.answer(&[ACK]);
    }

    fn receive(&mut self, bytes: &[u8]) {
        match self.running {
            Running::Nothing | Running::Firmware => {}
            // The bootloader speaks even parity only: with none, what it gets is noise.
            Running::Bootloader { .. } if !self.even_parity => {}
            Running::Bootloader { listening: false } => {
                if bytes.first() == Some(&0x7F) {
                    self.attempts += 1;
                    if self.attempts > self.silent_attempts {
                        self.running = Running::Bootloader { listening: true };
                        self.answer(&[ACK]);
                    }
                }
            }
            Running::Bootloader { listening: true } => {
                if self.pending == Pending::Command && bytes == [0x7F] {
                    // A second start byte is refused.
                    self.answer(&[NACK]);
                    return;
                }
                self.rx.extend_from_slice(bytes);
                self.step();
            }
        }
    }
}

/// A handle on a simulated chip, as the flasher's serial port and lines. Clone it to keep one
/// for looking at the chip.
#[derive(Clone, Debug)]
pub struct Sim(pub Rc<RefCell<SimChip>>);

impl Sim {
    pub fn new(chip: Chip) -> Self {
        Sim(Rc::new(RefCell::new(SimChip::new(chip))))
    }
}

impl Port for Sim {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.borrow_mut().receive(bytes);
        Ok(())
    }

    fn read_exact(&mut self, buf: &mut [u8], _timeout: Duration) -> io::Result<()> {
        let mut chip = self.0.borrow_mut();
        if chip.tx.len() < buf.len() {
            chip.tx.clear();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "no answer"));
        }
        for byte in buf.iter_mut() {
            *byte = chip.tx.pop_front().unwrap_or(0);
        }
        Ok(())
    }

    fn set_even_parity(&mut self, even: bool) -> io::Result<()> {
        self.0.borrow_mut().even_parity = even;
        Ok(())
    }

    fn discard_input(&mut self) -> io::Result<()> {
        self.0.borrow_mut().tx.clear();
        Ok(())
    }
}

impl Lines for Sim {
    fn set_boot0(&mut self, high: bool) -> io::Result<()> {
        self.0.borrow_mut().boot0 = high;
        Ok(())
    }

    fn hold_reset(&mut self) -> io::Result<()> {
        let mut chip = self.0.borrow_mut();
        chip.reset_held = true;
        chip.running = Running::Nothing;
        Ok(())
    }

    fn release_reset(&mut self) -> io::Result<()> {
        let mut chip = self.0.borrow_mut();
        if chip.reset_held {
            chip.reset_held = false;
            chip.boot(true);
        }
        Ok(())
    }

    fn sleep(&mut self, duration: Duration) {
        self.0.borrow_mut().sleeps.push(duration.as_millis() as u64);
    }
}
