//! Bench (T22, T23): the north bridge's sub-GHz radio as TOWER's radio sends, against TOWER Core
//! Modules.
//!
//! On the STM32WL55's M4 core, with plain registers for the board and the two crates the north
//! bridge's firmware will use: `fiber-subghz` for the radio's commands and `fiber-tower-air`'s
//! `packet` for TOWER's packet (length, CRC, PN9 whitening). It brings the radio up (the TCXO at
//! 1.7 V, the calibrations for 863 to 870 MHz), sets GFSK as TOWER sends it
//! (`docs/firmware/radio.md`), and does one of four things ([`MODE`]). It reports on USART1 (PA9,
//! 115200 baud, 8N1), one line at a time:
//!
//! - `step ...`: each step of the bring-up, with the radio's status and its device errors;
//! - receive mode, `rx ...`: each TOWER packet heard on channel 0, checked by the codec, with its
//!   signal strength and, for `radio_beacon`'s, its sequence number;
//! - send mode, `sent ...`: `radio_beacon`'s frames made by the codec and sent, the PA's levels in
//!   turn;
//! - capture mode, `frame ...`: the bytes after any preamble on channel 0 (no sync word);
//! - survey mode, `floor ...` and `listen ...`: the noise floor in every setting, then TOWER's
//!   three channels in turn.
//!
//! A bench tool, not the north bridge's firmware (that is T25).
#![no_std]
#![no_main]

use core::fmt::{self, Write};
use core::panic::PanicInfo;

use cortex_m::peripheral::DWT;
use cortex_m_rt::entry;
use fiber_subghz::{
    Bandwidth, Bus, GfskModulation, GfskPacket, ImageBand, Irq, LowPowerPa, PacketType, Path,
    PreambleDetector, Ramp, Regulator, Standby, Status, SubGhz, Switch, TcxoVoltage, Timeout,
};
use fiber_tower_air::packet::{self, MAX_PACKET, SYNC_WORD};
use stm32_metapac as pac;

/// The core's clock after `clocks()`: MSI at 16 MHz.
const HZ: u32 = 16_000_000;

/// TOWER's EU 868 channels 0 to 2 (`firmware/src/radio/config.rs`, `tower_protocol::mgmt`).
const CHANNELS: [u32; 3] = [868_100_000, 868_300_000, 868_500_000];

/// What the spike does after the bring-up.
#[allow(dead_code)]
enum Mode {
    /// The noise floor in every setting, then listening on the three channels.
    Survey,
    /// What follows any preamble on channel 0, printed raw.
    Capture,
    /// TOWER packets on channel 0, taken apart by the codec.
    Receive,
    /// `radio_beacon`'s frames, made by the codec, sent on channel 0.
    Send,
    /// The same frames at +10 dBm through each of the RF switch's 8 states in turn, to find the
    /// one that gets the most to the antenna.
    SwitchSweep,
}

const MODE: Mode = Mode::Receive;

// ---- Time, from the core's cycle counter ---------------------------------------------------------

fn now() -> u32 {
    DWT::cycle_count()
}

fn micros_since(start: u32) -> u32 {
    now().wrapping_sub(start) / (HZ / 1_000_000)
}

fn delay_us(us: u32) {
    let start = now();
    while micros_since(start) < us {}
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
/// as outputs for the RF switch; the radio's SPI; the cycle counter. Whatever ran before (the ROM
/// bootloader, when it was started with Go) may have left another clock selected and USART1 on.
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

    if let Some(mut core) = cortex_m::Peripherals::take() {
        core.DCB.enable_trace();
        core.DWT.enable_cycle_counter();
    }
}

// ---- The radio's bus and RF switch, for fiber-subghz ----------------------------------------------

/// The SUBGHZSPI, its chip select in `PWR_SUBGHZSPICR` and its busy line in `PWR_SR2`.
struct SpiBus;

fn byte(out: u8) -> u8 {
    let spi = pac::SUBGHZSPI;
    while !spi.sr().read().txe() {}
    spi.dr8().write_value(out);
    while !spi.sr().read().rxne() {}
    spi.dr8().read()
}

impl Bus for SpiBus {
    /// The radio stayed busy for 100 ms.
    type Error = ();

    fn transfer(&mut self, write: &[&[u8]], read: &mut [u8]) -> Result<(), ()> {
        let start = now();
        while pac::PWR.sr2().read().rfbusys() {
            if micros_since(start) > 100_000 {
                return Err(());
            }
        }
        pac::PWR.subghzspicr().modify(|w| w.set_nss(false));
        for part in write {
            for &b in *part {
                byte(b);
            }
        }
        for b in read.iter_mut() {
            *b = byte(0);
        }
        pac::PWR.subghzspicr().modify(|w| w.set_nss(true));
        Ok(())
    }
}

/// The RF switch on PC3, PC4 and PC5 (docs/hardware.md).
struct RfSwitch;

impl Switch for RfSwitch {
    fn set(&mut self, path: Path) {
        let (pc3, pc4, pc5) = match path {
            Path::Off => (false, false, false),
            Path::Receive => (true, true, true),
            Path::TransmitLowPower => (true, true, false),
        };
        pac::GPIOC.bsrr().write(|w| {
            for (pin, high) in [(3, pc3), (4, pc4), (5, pc5)] {
                if high {
                    w.set_bs(pin, true);
                } else {
                    w.set_br(pin, true);
                }
            }
        });
    }
}

type Radio = SubGhz<SpiBus>;

/// The status byte as words.
struct Shown(Status);

impl fmt::Display for Shown {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "0x{:02x} {:?} {}", self.0.0, self.0.mode(), self.0.command())
    }
}

fn step(radio: &mut Radio, what: &str) {
    let status = radio.status().unwrap_or(Status(0));
    let errors = radio.device_errors().unwrap_or(0xFFFF);
    say!("step {what}: status {} errors 0x{errors:04x}", Shown(status));
}

// ---- The bring-up ------------------------------------------------------------------------------

/// The radio out of reset, on its TCXO, calibrated, in standby, GFSK.
fn bring_up(radio: &mut Radio) {
    let rcc = pac::RCC;
    rcc.csr().modify(|w| w.set_rfrst(true));
    delay_us(100);
    rcc.csr().modify(|w| w.set_rfrst(false));
    while rcc.csr().read().rfrstf() {}
    // A falling edge of the chip select wakes the radio.
    pac::PWR.subghzspicr().modify(|w| w.set_nss(false));
    delay_us(100);
    pac::PWR.subghzspicr().modify(|w| w.set_nss(true));
    step(radio, "out of reset");
    radio.standby(Standby::Rc).ok();
    // The TCXO at 1.7 V, 1 ms to settle (64 steps of 15.625 us); then every calibration on it.
    radio.set_tcxo(TcxoVoltage::V1_7, 64).ok();
    radio.clear_irq(Irq::ALL).ok();
    radio.calibrate_all().ok();
    radio.calibrate_image(ImageBand::Eu868).ok();
    step(radio, "tcxo and calibrations");
    // The radio sets XOSC_START_ERR at its start, before it is told about the TCXO.
    radio.clear_device_errors().ok();
    radio.set_packet_type(PacketType::Gfsk).ok();
    // The switching regulator, as ST's code for the module runs it (`HAL_RADIO_IsDCDC`). Measured
    // against the linear one on 2026-11-19: the same levels both ways, within 1 dB.
    radio.set_regulator(Regulator::Smps).ok();
    step(radio, "errors cleared, gfsk, smps");
}

/// GFSK as TOWER sends it on `frequency`, raw, `length` bytes after the sync word (none: after
/// the preamble).
fn tune(radio: &mut Radio, frequency: u32, bandwidth: Bandwidth, sync: &[u8], length: u8) {
    radio.standby(Standby::Rc).ok();
    radio.set_rf_frequency(frequency).ok();
    radio.set_gfsk_modulation(&GfskModulation::tower(bandwidth)).ok();
    radio
        .set_gfsk_packet(&GfskPacket {
            preamble_bits: 32,
            detector: PreambleDetector::Bits16,
            sync_bits: (sync.len() * 8) as u8,
            length,
        })
        .ok();
    if !sync.is_empty() {
        radio.set_sync_word(sync).ok();
    }
    radio.set_buffer_base(0x00, 0x00).ok();
    let irq = Irq::TX_DONE
        .or(Irq::RX_DONE)
        .or(Irq::PREAMBLE)
        .or(Irq::SYNC_WORD)
        .or(Irq::CRC_ERROR)
        .or(Irq::TIMEOUT);
    radio.set_irq(irq).ok();
}

fn listen_on(radio: &mut Radio, boosted: bool) {
    RfSwitch.set(Path::Receive);
    radio.clear_irq(Irq::ALL).ok();
    radio.receive(Timeout::Continuous).ok();
    radio.set_rx_gain(boosted).ok();
}

/// A line of text built before it is sent, so that a frame is one line.
struct Line {
    buf: [u8; 320],
    len: usize,
}

impl Line {
    fn new() -> Self {
        Line {
            buf: [0; 320],
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

// ---- Receive: TOWER packets through the codec ----------------------------------------------------

/// One byte on air at 19 216.6 bit/s, in microseconds.
const BYTE_US: u32 = 417;

/// TOWER packets on channel 0: the radio listens for the sync word, takes up to [`MAX_PACKET`]
/// bytes after it, and is stopped once the packet's own length is in. The length is read from the
/// radio's buffer two byte times after the sync word.
fn receive(radio: &mut Radio) -> ! {
    say!("receive: channel 0, 187.2 kHz, boosted, TOWER packets through the codec");
    tune(radio, CHANNELS[0], Bandwidth::Khz187, &SYNC_WORD, u8::MAX);
    listen_on(radio, true);
    let (mut good, mut bad, mut last_seq) = (0u32, 0u32, None::<u32>);
    let mut since_report = now();
    loop {
        let irq = radio.irq().unwrap_or_default();
        if irq.contains(Irq::SYNC_WORD) {
            let sync_at = now();
            // The length byte, as soon as it is in.
            while micros_since(sync_at) < 2 * BYTE_US {}
            let mut first = [0u8];
            radio.read_buffer(0, &mut first).ok();
            let len = packet::packet_len(first[0]);
            // The rest: one byte time each, and one more for the radio to have put it away.
            while micros_since(sync_at) < (len as u32 + 1) * BYTE_US {}
            let status = radio.gfsk_packet_status().ok();
            radio.standby(Standby::Rc).ok();
            let mut buf = [0u8; MAX_PACKET];
            let n = len.min(MAX_PACKET);
            radio.read_buffer(0, &mut buf[..n]).ok();
            let rssi = status.map_or(0, |s| s.rssi_sync);
            let mut line = Line::new();
            match packet::decode(&mut buf[..n]) {
                Ok(payload) => {
                    good += 1;
                    let _ = write!(line, "rx {} B, {rssi} dBm:", payload.len());
                    for b in payload {
                        let _ = write!(line, " {b:02x}");
                    }
                    if payload.len() >= 4 {
                        let seq = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                        let missed = last_seq.map_or(0, |last| seq.wrapping_sub(last).wrapping_sub(1));
                        let _ = write!(line, " (#{seq}, {missed} missed)");
                        last_seq = Some(seq);
                    }
                }
                Err(e) => {
                    bad += 1;
                    let _ = write!(line, "rx refused ({e:?}), length byte says {len}, {rssi} dBm");
                }
            }
            say!("{}", line.as_str());
            listen_on(radio, true);
        } else if irq.0 != 0 {
            radio.clear_irq(irq).ok();
        }
        if micros_since(since_report) > 30_000_000 {
            say!("receive: {good} packets passed the codec, {bad} refused");
            since_report = now();
        }
    }
}

// ---- Send: radio_beacon's frames through the codec ------------------------------------------------

/// The low-power PA's levels tried (RM0453's settings): +14, +10, and +10's configuration asked
/// for 0 dBm (about -3 dBm).
const LEVELS: [(&str, LowPowerPa); 3] = [
    (
        "-3 dBm",
        LowPowerPa {
            duty_cycle: 0x01,
            power: 0,
        },
    ),
    ("+10 dBm", LowPowerPa::DBM_10),
    ("+14 dBm", LowPowerPa::DBM_14),
];

/// `radio_beacon`'s frames, one every 2 s (11 ms on air: about 0.6 % of the time, inside the 1 %
/// of the 868.0 to 868.6 MHz band), numbered from 1 000 000, the levels in turn, 300 of them;
/// then it waits.
fn send(radio: &mut Radio) -> ! {
    say!("send: radio_beacon frames through the codec on channel 0, the levels in turn");
    let mut packet = [0u8; MAX_PACKET];
    for n in 0..300u32 {
        let seq = 1_000_000 + n;
        let mut payload = [0xA5u8; 16];
        payload[..4].copy_from_slice(&seq.to_le_bytes());
        let len = packet::encode(&payload, &mut packet).unwrap_or(0);
        let (level, pa) = LEVELS[n as usize % LEVELS.len()];
        tune(radio, CHANNELS[0], Bandwidth::Khz187, &SYNC_WORD, len as u8);
        radio.set_low_power_pa(pa, Ramp::Us200).ok();
        radio.write_buffer(0, &packet[..len]).ok();
        radio.clear_irq(Irq::ALL).ok();
        RfSwitch.set(Path::TransmitLowPower);
        radio.send(Timeout::None).ok();
        let start = now();
        let mut sent = false;
        while micros_since(start) < 200_000 {
            if radio.irq().unwrap_or_default().contains(Irq::TX_DONE) {
                sent = true;
                break;
            }
        }
        radio.clear_irq(Irq::ALL).ok();
        radio.standby(Standby::Rc).ok();
        RfSwitch.set(Path::Off);
        say!(
            "sent #{seq} at {level}: {}",
            if sent { "tx done" } else { "NO tx done" }
        );
        delay_us(2_000_000);
    }
    say!("send: done");
    loop {
        cortex_m::asm::wfi();
    }
}

/// PC3, PC4 and PC5 as bits 0, 1 and 2 of `state`, whatever ST's table says.
fn switch_pins(state: u32) {
    pac::GPIOC.bsrr().write(|w| {
        for (bit, pin) in [(0, 3), (1, 4), (2, 5)] {
            if state & (1 << bit) != 0 {
                w.set_bs(pin, true);
            } else {
                w.set_br(pin, true);
            }
        }
    });
}

/// Frames numbered 2 000 000 + 8 x round + state, at +10 dBm, one every second, the switch in
/// `state` (PC3 bit 0, PC4 bit 1, PC5 bit 2) while it sends; 6 rounds.
fn switch_sweep(radio: &mut Radio) -> ! {
    say!("switch sweep: +10 dBm, the 8 states of PC3/PC4/PC5 in turn, 6 rounds");
    let mut packet = [0u8; MAX_PACKET];
    for n in 0..48u32 {
        let state = n % 8;
        let seq = 2_000_000 + n;
        let mut payload = [0xA5u8; 16];
        payload[..4].copy_from_slice(&seq.to_le_bytes());
        let len = packet::encode(&payload, &mut packet).unwrap_or(0);
        tune(radio, CHANNELS[0], Bandwidth::Khz187, &SYNC_WORD, len as u8);
        radio.set_low_power_pa(LowPowerPa::DBM_10, Ramp::Us200).ok();
        radio.write_buffer(0, &packet[..len]).ok();
        radio.clear_irq(Irq::ALL).ok();
        switch_pins(state);
        radio.send(Timeout::None).ok();
        let start = now();
        while micros_since(start) < 200_000 {
            if radio.irq().unwrap_or_default().contains(Irq::TX_DONE) {
                break;
            }
        }
        radio.clear_irq(Irq::ALL).ok();
        radio.standby(Standby::Rc).ok();
        RfSwitch.set(Path::Off);
        say!(
            "sent #{seq}: PC3 {} PC4 {} PC5 {}",
            state & 1,
            (state >> 1) & 1,
            (state >> 2) & 1
        );
        delay_us(1_000_000);
    }
    say!("switch sweep: done");
    loop {
        cortex_m::asm::wfi();
    }
}

// ---- Capture and survey ------------------------------------------------------------------------

/// The bytes kept after a preamble in capture mode.
const FRAME: u8 = 64;

/// What the radio heard in about 10 s: the RSSI's lowest, average and highest, the preambles
/// and the sync words; each frame is printed as it comes.
struct Heard {
    min: i16,
    avg: i32,
    max: i16,
    preambles: u32,
    syncs: u32,
}

fn heard(radio: &mut Radio) -> Heard {
    let (mut min, mut max, mut sum, mut n) = (i16::MAX, i16::MIN, 0i32, 0i32);
    let (mut preambles, mut syncs) = (0, 0);
    for _ in 0..500 {
        let irq = radio.irq().unwrap_or_default();
        if irq.contains(Irq::PREAMBLE) {
            preambles += 1;
        }
        if irq.contains(Irq::SYNC_WORD) {
            syncs += 1;
        }
        if irq.contains(Irq::RX_DONE) {
            print_frame(radio);
        }
        if irq.0 != 0 {
            radio.clear_irq(irq).ok();
        }
        let r = radio.rssi().unwrap_or(0);
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

/// A raw frame, as the radio delivered it.
fn print_frame(radio: &mut Radio) {
    let (length, start) = radio.rx_buffer_status().unwrap_or((0, 0));
    let rssi = radio.gfsk_packet_status().map_or(0, |s| s.rssi_sync);
    let mut data = [0u8; FRAME as usize];
    let n = usize::from(length.min(FRAME));
    radio.read_buffer(start, &mut data[..n]).ok();
    let mut line = Line::new();
    let _ = write!(line, "frame rssi {rssi} dBm len {length}:");
    for b in &data[..n] {
        let _ = write!(line, " {b:02x}");
    }
    say!("{}", line.as_str());
}

fn capture(radio: &mut Radio) -> ! {
    say!("capture: channel 0, 187.2 kHz, boosted, no sync word, {FRAME} bytes after each preamble");
    tune(radio, CHANNELS[0], Bandwidth::Khz187, &[], FRAME);
    listen_on(radio, true);
    loop {
        let h = heard(radio);
        say!(
            "capture: rssi min {} avg {} max {} dBm, preambles {}",
            h.min,
            h.avg,
            h.max,
            h.preambles
        );
    }
}

/// The settings the noise floor is measured in.
const SETTINGS: [(&str, Regulator, &str, bool); 4] = [
    ("ldo", Regulator::Ldo, "normal", false),
    ("ldo", Regulator::Ldo, "boosted", true),
    ("smps", Regulator::Smps, "normal", false),
    ("smps", Regulator::Smps, "boosted", true),
];

const BANDWIDTHS: [(&str, Bandwidth); 3] = [
    ("117.3", Bandwidth::Khz117),
    ("156.2", Bandwidth::Khz156),
    ("187.2", Bandwidth::Khz187),
];

fn survey(radio: &mut Radio) -> ! {
    for (bw_name, bw) in BANDWIDTHS {
        for (reg_name, regulator, gain_name, boosted) in SETTINGS {
            radio.standby(Standby::Rc).ok();
            radio.set_regulator(regulator).ok();
            tune(radio, CHANNELS[0], bw, &SYNC_WORD, FRAME);
            listen_on(radio, boosted);
            let h = heard(radio);
            say!(
                "floor {bw_name} kHz {reg_name} {gain_name}: rssi min {} avg {} max {} dBm, preambles {}, syncs {}",
                h.min,
                h.avg,
                h.max,
                h.preambles,
                h.syncs
            );
        }
    }
    radio.set_regulator(Regulator::Ldo).ok();
    let mut round = 0u32;
    loop {
        for (channel, frequency) in CHANNELS.iter().enumerate() {
            tune(radio, *frequency, Bandwidth::Khz156, &SYNC_WORD, FRAME);
            listen_on(radio, true);
            let h = heard(radio);
            say!(
                "listen round {round} channel {channel} ({frequency} Hz): rssi min {} avg {} max {} dBm, preambles {}, syncs {}",
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

#[entry]
fn main() -> ! {
    clocks();
    say!("fiber-radio-spike: north bridge, EU 868");
    let mut radio = SubGhz::new(SpiBus);
    bring_up(&mut radio);
    match MODE {
        Mode::Survey => survey(&mut radio),
        Mode::Capture => capture(&mut radio),
        Mode::Receive => receive(&mut radio),
        Mode::Send => send(&mut radio),
        Mode::SwitchSweep => switch_sweep(&mut radio),
    }
}
