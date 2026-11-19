//! Bench (T22): the north bridge's sub-GHz radio as TOWER's radio sends, without a TOWER node.
//!
//! On the STM32WL55's M4 core, with plain registers. It brings the radio up (the TCXO at 1.7 V,
//! the calibrations for 863 to 870 MHz, the RF switch to receive), sets GFSK as TOWER sends it
//! (19 216.6 bit/s, 19.8 kHz deviation, BT 1, sync word 0xDB624715) on EU 868 channel 0
//! (868.1 MHz), and receives raw: fixed length, the radio's own whitening and CRC off. It reports
//! on USART1 (PA9, 115200 baud, 8N1), one line at a time:
//!
//! - `step ...`: each step of the bring-up, with the radio's status and its device errors;
//! - `floor ...`: the channel's RSSI, 500 samples about 70 ms apart (a pause of 20 ms and the
//!   commands around it), for each receive filter in four settings in turn (the linear or the
//!   switching regulator, the normal or the boosted receive gain): about 7 minutes a round;
//! - `frame ...`: the bytes after any sync word it heard, with the packet's RSSI.
//!
//! Written from RM0453 (the STM32WL reference manual, section 5) and docs/hardware.md. A bench
//! tool, not the north bridge's firmware (that is T25).
#![no_std]
#![no_main]

use core::fmt::{self, Write};
use core::panic::PanicInfo;

use cortex_m_rt::entry;
use stm32_metapac as pac;

/// The core's clock after `clocks()`: MSI at 16 MHz.
const HZ: u32 = 16_000_000;

/// EU 868 channel 0 of TOWER (`firmware/src/radio/config.rs`).
const FREQUENCY: u64 = 868_100_000;
/// The radio's TCXO (inside the module): 32 MHz.
const XTAL: u64 = 32_000_000;
/// The bytes kept after a sync word.
const FRAME: u8 = 64;

fn delay_us(us: u32) {
    cortex_m::asm::delay(us * (HZ / 1_000_000));
}

// ---- The serial port ---------------------------------------------------------------------------

struct Serial;

impl Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let usart = pac::USART1;
        for byte in s.bytes() {
            while !usart.isr().read().txe() {}
            usart.tdr().write(|w| w.set_dr(u16::from(byte)));
        }
        Ok(())
    }
}

macro_rules! say {
    ($($arg:tt)*) => {{
        let _ = writeln!(Serial, $($arg)*);
    }};
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    say!("panic {}", info.message());
    loop {
        cortex_m::asm::nop();
    }
}

/// MSI at 16 MHz (0 wait states are enough up to 18 MHz); USART1 on PA9 and PA10; PC3 to PC5
/// as outputs for the RF switch; the radio's SPI. Whatever ran before (the ROM bootloader, when
/// it was started with Go) may have left another clock selected and USART1 enabled.
fn clocks() {
    let rcc = pac::RCC;
    rcc.cr().modify(|w| w.set_msion(true));
    while !rcc.cr().read().msirdy() {}
    rcc.cfgr().modify(|w| w.set_sw(pac::rcc::vals::Sw::MSI));
    while rcc.cfgr().read().sws() != pac::rcc::vals::Sw::MSI {}
    rcc.cr().modify(|w| {
        w.set_msirange(pac::rcc::vals::Msirange::RANGE16M);
        w.set_msirgsel(pac::rcc::vals::Msirgsel::CR);
    });
    while !rcc.cr().read().msirdy() {}
    rcc.ahb2enr().modify(|w| {
        w.set_gpioaen(true);
        w.set_gpiocen(true);
    });
    rcc.apb2enr().modify(|w| w.set_usart1en(true));
    rcc.apb3enr().modify(|w| w.set_subghzspien(true));

    let gpioa = pac::GPIOA;
    for pin in [9, 10] {
        gpioa.afr(pin / 8).modify(|w| w.set_afr(pin % 8, 7));
        gpioa
            .moder()
            .modify(|w| w.set_moder(pin, pac::gpio::vals::Moder::ALTERNATE));
    }
    let usart = pac::USART1;
    // The baud rate and the frame can only be set while it is off.
    usart.cr1().write(|_| {});
    usart.cr2().write(|_| {});
    usart.cr3().write(|_| {});
    usart.brr().write(|w| w.set_brr((HZ / 115_200) as u16));
    usart.cr1().write(|w| {
        w.set_ue(true);
        w.set_te(true);
    });

    let gpioc = pac::GPIOC;
    for pin in [3, 4, 5] {
        gpioc
            .moder()
            .modify(|w| w.set_moder(pin, pac::gpio::vals::Moder::OUTPUT));
    }

    let spi = pac::SUBGHZSPI;
    spi.cr1().write(|w| {
        w.set_mstr(pac::spi::vals::Mstr::MASTER);
        w.set_br(pac::spi::vals::Br::DIV2);
        w.set_ssm(true);
        w.set_ssi(true);
    });
    spi.cr2().write(|w| {
        w.set_ds(pac::spi::vals::Ds::BITS8);
        w.set_frxth(pac::spi::vals::Frxth::QUARTER);
    });
    spi.cr1().modify(|w| w.set_spe(true));
}

/// The RF switch: receive = PC3, PC4 and PC5 high (docs/hardware.md).
fn switch_to_receive() {
    pac::GPIOC.bsrr().write(|w| {
        w.set_bs(3, true);
        w.set_bs(4, true);
        w.set_bs(5, true);
    });
}

// ---- The radio's commands ----------------------------------------------------------------------

/// Waits until the radio is not busy; whether it was in time.
fn wait_busy() -> bool {
    for _ in 0..100_000 {
        if !pac::PWR.sr2().read().rfbusys() {
            return true;
        }
        delay_us(1);
    }
    false
}

fn byte(out: u8) -> u8 {
    let spi = pac::SUBGHZSPI;
    while !spi.sr().read().txe() {}
    spi.dr8().write_value(out);
    while !spi.sr().read().rxne() {}
    spi.dr8().read()
}

/// One command: the opcode and its parameters out, as many bytes back as `back` holds (after
/// the opcode, the first is the status).
fn command(opcode: u8, params: &[u8], back: &mut [u8]) {
    if !wait_busy() {
        say!("busy stuck before 0x{opcode:02x}");
    }
    pac::PWR.subghzspicr().modify(|w| w.set_nss(false));
    byte(opcode);
    for &p in params {
        byte(p);
    }
    for b in back.iter_mut() {
        *b = byte(0);
    }
    pac::PWR.subghzspicr().modify(|w| w.set_nss(true));
}

fn set(opcode: u8, params: &[u8]) {
    command(opcode, params, &mut []);
}

fn write_register(address: u16, values: &[u8]) {
    let mut params = [0u8; 10];
    params[0] = (address >> 8) as u8;
    params[1] = address as u8;
    params[2..2 + values.len()].copy_from_slice(values);
    set(0x0D, &params[..2 + values.len()]);
}

fn read_register(address: u16) -> u8 {
    let mut back = [0u8; 2];
    command(0x1D, &[(address >> 8) as u8, address as u8], &mut back);
    back[1]
}

fn status() -> u8 {
    let mut back = [0u8; 1];
    command(0xC0, &[], &mut back);
    back[0]
}

fn errors() -> u16 {
    let mut back = [0u8; 3];
    command(0x17, &[], &mut back);
    u16::from_be_bytes([back[1], back[2]])
}

fn irq() -> u16 {
    let mut back = [0u8; 3];
    command(0x12, &[], &mut back);
    u16::from_be_bytes([back[1], back[2]])
}

fn rssi() -> i16 {
    let mut back = [0u8; 2];
    command(0x15, &[], &mut back);
    -(i16::from(back[1]) / 2)
}

/// The status byte as words: the mode (bits 6..4) and the last command's result (bits 3..1).
struct Status(u8);

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mode = match (self.0 >> 4) & 7 {
            2 => "standby-rc",
            3 => "standby-hse32",
            4 => "fs",
            5 => "rx",
            6 => "tx",
            _ => "?",
        };
        let result = match (self.0 >> 1) & 7 {
            2 => "data",
            3 => "timeout",
            4 => "error",
            5 => "failed",
            6 => "tx-done",
            _ => "ok",
        };
        write!(f, "0x{:02x} {mode} {result}", self.0)
    }
}

fn step(what: &str) {
    say!(
        "step {what}: status {} errors 0x{:04x}",
        Status(status()),
        errors()
    );
}

// ---- The bring-up ------------------------------------------------------------------------------

/// The radio out of reset, on its TCXO, calibrated, in standby.
fn bring_up() {
    let rcc = pac::RCC;
    rcc.csr().modify(|w| w.set_rfrst(true));
    delay_us(100);
    rcc.csr().modify(|w| w.set_rfrst(false));
    while rcc.csr().read().rfrstf() {}
    // A falling edge of NSS wakes the radio.
    pac::PWR.subghzspicr().modify(|w| w.set_nss(false));
    delay_us(100);
    pac::PWR.subghzspicr().modify(|w| w.set_nss(true));
    step("out of reset");
    set(0x80, &[0x00]); // standby on the RC oscillator
    // The TCXO at 1.7 V, 1 ms to settle (64 steps of 15.625 us).
    set(0x97, &[0x02, 0x00, 0x00, 0x40]);
    set(0x02, &[0xFF, 0xFF]); // clear the IRQs
    set(0x89, &[0x7F]); // calibrate everything, now on the TCXO
    wait_busy();
    step("tcxo and calibration");
    set(0x98, &[0xD7, 0xDB]); // the image calibration for 863 to 870 MHz
    wait_busy();
    step("image calibration");
    // The radio sets XOSC_START_ERR (0x0020) at its start, before it is told about the TCXO. A
    // calibration that failed would have set one of the other bits.
    set(0x07, &[0x00, 0x00]);
    step("errors cleared");
}

/// GFSK as TOWER sends it, raw, on channel 0; `bandwidth` is the radio's code for the receive
/// filter.
fn gfsk(bandwidth: u8) {
    set(0x8A, &[0x00]); // GFSK
    let f = (FREQUENCY << 25) / XTAL;
    set(0x86, &(f as u32).to_be_bytes());
    // The bit rate 32 * 32 MHz / 19 216.6 = 53 286; BT 1 (0x0B); the deviation 19.8 kHz * 2^25 /
    // 32 MHz = 20 762.
    let br: u32 = 53_286;
    let fdev: u32 = 20_762;
    let br = br.to_be_bytes();
    let fdev = fdev.to_be_bytes();
    set(
        0x8B,
        &[br[1], br[2], br[3], 0x0B, bandwidth, fdev[1], fdev[2], fdev[3]],
    );
    // A preamble of 32 bits (for sending), a detector of 16 bits, a sync word of 32 bits, no
    // address, fixed length, FRAME bytes, no CRC, no whitening.
    set(0x8C, &[0x00, 0x20, 0x05, 0x20, 0x00, 0x00, FRAME, 0x01, 0x00]);
    write_register(0x06C0, &[0xDB, 0x62, 0x47, 0x15]);
    set(0x8F, &[0x00, 0x00]); // both buffers from 0
    // IRQs: rx done, preamble, sync word, CRC error, timeout.
    set(0x08, &[0x02, 0x4E, 0x02, 0x4E, 0x00, 0x00, 0x00, 0x00]);
    step("gfsk");
}

fn receive() {
    switch_to_receive();
    set(0x02, &[0xFF, 0xFF]);
    set(0x82, &[0xFF, 0xFF, 0xFF]); // continuous
}

/// A frame the radio heard: its bytes and RSSI, then it listens on.
fn take_frame() {
    let mut back = [0u8; 3];
    command(0x13, &[], &mut back); // the buffer: length, start
    let (length, start) = (back[1], back[2]);
    let mut packet = [0u8; 4];
    command(0x14, &[], &mut packet); // rx status, RSSI at the sync word, average RSSI
    let mut data = [0u8; FRAME as usize + 1];
    let n = usize::from(length.min(FRAME)) + 1;
    command(0x1E, &[start], &mut data[..n]);
    let mut line = Line::new();
    let _ = write!(
        line,
        "frame rssi {} dBm status 0x{:02x} len {length}:",
        -(i16::from(packet[2]) / 2),
        packet[1]
    );
    for b in &data[1..n] {
        let _ = write!(line, " {b:02x}");
    }
    say!("{}", line.as_str());
}

/// A line of text built before it is sent, so that a frame is one line.
struct Line {
    buf: [u8; 256],
    len: usize,
}

impl Line {
    fn new() -> Self {
        Line {
            buf: [0; 256],
            len: 0,
        }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
    }
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let n = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// The four settings the noise floor is measured in.
const SETTINGS: [(&str, u8, &str, u8); 4] = [
    ("ldo", 0x00, "normal", 0x94),
    ("ldo", 0x00, "boosted", 0x96),
    ("smps", 0x01, "normal", 0x94),
    ("smps", 0x01, "boosted", 0x96),
];

/// The receive filters tried: 117.3, 156.2 and 187.2 kHz (the radio's codes).
const BANDWIDTHS: [(u32, u8); 3] = [(117_300, 0x0B), (156_200, 0x1A), (187_200, 0x12)];

#[entry]
fn main() -> ! {
    clocks();
    say!("fiber-radio-spike: north bridge, EU 868 channel 0, {FREQUENCY} Hz");
    bring_up();
    let mut round = 0u32;
    loop {
        for (bw_hz, bw) in BANDWIDTHS {
            for (regulator, reg_code, gain, gain_code) in SETTINGS {
                set(0x80, &[0x00]);
                set(0x96, &[reg_code]);
                gfsk(bw);
                receive();
                write_register(0x08AC, &[gain_code]);
                let (mut min, mut max, mut sum, mut n) = (i16::MAX, i16::MIN, 0i32, 0i32);
                let mut syncs = 0;
                for _ in 0..500 {
                    let flags = irq();
                    if flags & (1 << 3) != 0 {
                        syncs += 1;
                    }
                    if flags & (1 << 1) != 0 {
                        take_frame();
                    }
                    if flags != 0 {
                        set(0x02, &flags.to_be_bytes());
                    }
                    let r = rssi();
                    min = min.min(r);
                    max = max.max(r);
                    sum += i32::from(r);
                    n += 1;
                    delay_us(20_000);
                }
                say!(
                    "floor round {round} bw {bw_hz} {regulator} {gain}: rssi min {min} avg {} max {max} dBm, \
                     gain register 0x{:02x}, syncs {syncs}, status {}",
                    sum / n,
                    read_register(0x08AC),
                    Status(status())
                );
            }
        }
        round += 1;
    }
}
