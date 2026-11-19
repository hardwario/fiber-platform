//! Bench (T22): the north bridge's sub-GHz radio as TOWER's radio sends, without a TOWER node.
//!
//! On the STM32WL55's M4 core, with plain registers. It brings the radio up (the TCXO at 1.7 V,
//! the calibrations for 863 to 870 MHz, the RF switch to receive), sets GFSK as TOWER sends it
//! (19 216.6 bit/s, 19.8 kHz deviation, BT 1, sync word `15 47 62 DB` on air) on EU 868 channel 0
//! (868.1 MHz), and receives raw: fixed length, the radio's own whitening and CRC off. After one
//! round of the noise floor it listens on TOWER's three channels in turn (868.1, 868.3 and 868.5
//! MHz). It reports on USART1 (PA9, 115200 baud, 8N1), one line at a time:
//!
//! - `step ...`: each step of the bring-up, with the radio's status and its device errors;
//! - `floor ...`: the channel's RSSI, 500 samples about 70 ms apart (a pause of 20 ms and the
//!   commands around it), for each receive filter in four settings in turn (the linear or the
//!   switching regulator, the normal or the boosted receive gain): about 7 minutes, once;
//! - `listen ...`: for each channel, the preambles and sync words heard in about 35 s;
//! - in the send mode, `sent ...`: each frame built here and sent, with the PA's level;
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

/// TOWER's EU 868 channels 0 to 2 (`firmware/src/radio/config.rs`, `tower_protocol::mgmt`).
const CHANNELS: [u64; 3] = [868_100_000, 868_300_000, 868_500_000];
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

/// The RF switch: transmit on the low-power PA = PC3 and PC4 high, PC5 low (docs/hardware.md).
fn switch_to_transmit() {
    pac::GPIOC.bsrr().write(|w| {
        w.set_bs(3, true);
        w.set_bs(4, true);
        w.set_br(5, true);
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

/// The sync word as it goes on air: TOWER's SYNC4 to SYNC1 (`0xDB624715`,
/// `firmware/src/radio/config.rs`) byte-reversed, as the captures showed (the SPIRIT1 sends SYNC1
/// first).
const SYNC: [u8; 4] = [0x15, 0x47, 0x62, 0xDB];

/// GFSK as TOWER sends it, raw, on `frequency`; `bandwidth` is the radio's code for the receive
/// filter. With no `sync` the radio takes the bytes right after the preamble, wherever in it it
/// stood: the sync word is then looked for in those bytes.
fn gfsk(frequency: u64, bandwidth: u8, sync: &[u8]) {
    set(0x8A, &[0x00]); // GFSK
    let f = (frequency << 25) / XTAL;
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
    // A preamble of 32 bits (for sending), a detector of 16 bits, the sync word's bits, no
    // address, fixed length, FRAME bytes, no CRC, no whitening.
    let sync_bits = (sync.len() * 8) as u8;
    set(
        0x8C,
        &[0x00, 0x20, 0x05, sync_bits, 0x00, 0x00, FRAME, 0x01, 0x00],
    );
    if !sync.is_empty() {
        write_register(0x06C0, sync);
    }
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

/// What the radio heard in about 35 s: the RSSI's lowest, average and highest, the preambles
/// and the sync words; each frame is printed as it comes.
struct Heard {
    min: i16,
    avg: i32,
    max: i16,
    preambles: u32,
    syncs: u32,
}

fn listen() -> Heard {
    let (mut min, mut max, mut sum, mut n) = (i16::MAX, i16::MIN, 0i32, 0i32);
    let (mut preambles, mut syncs) = (0, 0);
    for _ in 0..500 {
        let flags = irq();
        if flags & (1 << 2) != 0 {
            preambles += 1;
        }
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
    Heard {
        min,
        avg: sum / n,
        max,
        preambles,
        syncs,
    }
}

/// Standby, the regulator, GFSK, receive, the gain: one setting.
fn tune(frequency: u64, bandwidth: u8, regulator: u8, gain: u8, sync: &[u8]) {
    set(0x80, &[0x00]);
    set(0x96, &[regulator]);
    gfsk(frequency, bandwidth, sync);
    receive();
    write_register(0x08AC, &[gain]);
}

/// What the spike does after the bring-up.
#[allow(dead_code)]
enum Mode {
    /// The noise floor in every setting, then listening on the three channels.
    Survey,
    /// What follows any preamble on channel 0, printed.
    Capture,
    /// TOWER frames built here, sent on channel 0.
    Send,
}

const MODE: Mode = Mode::Capture;

// ---- TOWER's packet, as the captures showed it (docs/bench-checks.md, check 4) -----------------

/// PN9 (x^9 + x^5 + 1) from all ones, each byte filled from its least significant bit, over the
/// length byte, the payload and the CRC.
fn whiten(data: &mut [u8]) {
    let mut state: u16 = 0x1FF;
    for byte in data.iter_mut() {
        let mut mask = 0u8;
        for bit in 0..8 {
            mask |= ((state & 1) as u8) << bit;
            let feedback = (state ^ (state >> 5)) & 1;
            state = (state >> 1) | (feedback << 8);
        }
        *byte ^= mask;
    }
}

/// CRC-16/CCITT-FALSE over the length byte and the payload.
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// The payload of TOWER's `radio_beacon`: the sequence number, then 0xA5.
const PAYLOAD: usize = 16;
/// The length byte, the payload and the CRC.
const PACKET: usize = 1 + PAYLOAD + 2;

/// `radio_beacon`'s frame number `seq`, as it goes into the radio's buffer.
fn beacon_packet(seq: u32) -> [u8; PACKET] {
    let mut packet = [0xA5u8; PACKET];
    packet[0] = PAYLOAD as u8;
    packet[1..5].copy_from_slice(&seq.to_le_bytes());
    let crc = crc16(&packet[..1 + PAYLOAD]);
    packet[1 + PAYLOAD..].copy_from_slice(&crc.to_be_bytes());
    whiten(&mut packet);
    packet
}

/// The low-power PA's levels tried: its configuration (duty cycle, HP max, the low-power PA, 1)
/// and the power asked of it, from RM0453's optimal settings: +14 dBm (0x04 and 14), +10 dBm (0x01
/// and 13), and the +10 dBm configuration asked for 0 dBm (about -3 dBm).
const LEVELS: [(&str, [u8; 4], u8); 3] = [
    ("-3 dBm", [0x01, 0x00, 0x01, 0x01], 0x00),
    ("+10 dBm", [0x01, 0x00, 0x01, 0x01], 0x0D),
    ("+14 dBm", [0x04, 0x00, 0x01, 0x01], 0x0E),
];

/// One frame on channel 0 from the low-power PA at `level`: a preamble of 32 bits, the sync word,
/// the packet; standby after it. Whether it was sent.
fn send_frame(packet: &[u8; PACKET], level: usize) -> bool {
    set(0x80, &[0x00]);
    let (_, pa, power) = LEVELS[level];
    set(0x95, &pa);
    set(0x8E, &[power, 0x04]); // a ramp of 200 us
    set(
        0x8C,
        &[0x00, 0x20, 0x05, 0x20, 0x00, 0x00, PACKET as u8, 0x01, 0x00],
    );
    write_register(0x06C0, &SYNC);
    set(0x8F, &[0x00, 0x80]);
    let mut params = [0u8; 1 + PACKET];
    params[1..].copy_from_slice(packet);
    set(0x0E, &params); // the buffer, from 0
    set(0x08, &[0x02, 0x4F, 0x02, 0x4F, 0x00, 0x00, 0x00, 0x00]);
    set(0x02, &[0xFF, 0xFF]);
    switch_to_transmit();
    set(0x83, &[0x00, 0x00, 0x00]); // once, no timeout
    let mut sent = false;
    for _ in 0..200 {
        if irq() & 1 != 0 {
            sent = true;
            break;
        }
        delay_us(1_000);
    }
    set(0x02, &[0xFF, 0xFF]);
    set(0x80, &[0x00]);
    switch_to_receive();
    sent
}

/// Frames like `radio_beacon`'s, about one every 2 s (11 ms on air: about 0.6 % of the time,
/// inside the 1 % of the 868.0 to 868.6 MHz band; the delay loop runs about three times slower
/// than it counts), numbered from 1 000 000, the levels in turn, 300 of them; then it waits.
fn send() -> ! {
    say!("send: radio_beacon frames on channel 0, low-power PA, the levels in turn");
    gfsk(CHANNELS[0], 0x12, &SYNC);
    for n in 0..300u32 {
        let seq = 1_000_000 + n;
        let level = n as usize % LEVELS.len();
        let ok = send_frame(&beacon_packet(seq), level);
        say!(
            "sent #{seq} at {}: {}, status {}",
            LEVELS[level].0,
            if ok { "tx done" } else { "NO tx done" },
            Status(status())
        );
        delay_us(700_000);
    }
    say!("send: done");
    loop {
        cortex_m::asm::wfi();
    }
}

/// Captures: channel 0, the 187.2 kHz filter (room for both crystals' error), the linear
/// regulator, the boosted gain, no sync word; every FRAME bytes after a preamble are printed.
fn capture() -> ! {
    say!("capture: channel 0, 187.2 kHz, boosted, no sync word, {FRAME} bytes after each preamble");
    tune(CHANNELS[0], 0x12, 0x00, 0x96, &[]);
    let mut round = 0u32;
    loop {
        let h = listen();
        say!(
            "capture round {round}: rssi min {} avg {} max {} dBm, preambles {}",
            h.min,
            h.avg,
            h.max,
            h.preambles
        );
        round += 1;
    }
}

#[entry]
fn main() -> ! {
    clocks();
    say!("fiber-radio-spike: north bridge, EU 868 channels 0 to 2");
    bring_up();
    match MODE {
        Mode::Capture => capture(),
        Mode::Send => send(),
        Mode::Survey => {}
    }
    for (bw_hz, bw) in BANDWIDTHS {
        for (regulator, reg_code, gain, gain_code) in SETTINGS {
            tune(CHANNELS[0], bw, reg_code, gain_code, &SYNC);
            let h = listen();
            say!(
                "floor bw {bw_hz} {regulator} {gain}: rssi min {} avg {} max {} dBm, gain register 0x{:02x}, \
                 preambles {}, syncs {}, status {}",
                h.min,
                h.avg,
                h.max,
                read_register(0x08AC),
                h.preambles,
                h.syncs,
                Status(status())
            );
        }
    }
    let mut round = 0u32;
    loop {
        for (channel, frequency) in CHANNELS.iter().enumerate() {
            tune(*frequency, 0x1A, 0x00, 0x96, &SYNC);
            let h = listen();
            say!(
                "listen round {round} channel {channel} ({frequency} Hz, 156.2 kHz, boosted): rssi min {} avg {} \
                 max {} dBm, preambles {}, syncs {}",
                h.min,
                h.avg,
                h.max,
                h.preambles,
                h.syncs
            );
        }
        round += 1;
    }
}
