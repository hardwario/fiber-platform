//! fiber-subghz (`crates/subghz`): the sub-GHz radio inside the STM32WL, the FIBER's north bridge
//! (RM0453, section 5): its commands, over a [`Bus`] that the firmware fills with the SUBGHZSPI
//! and its busy line, and the RF switch as a [`Switch`]. `no_std`, nothing allocated.
//!
//! What it sends was proved on the bench by the radio spike (`hil/radio-spike`, T22) and is
//! written down in `docs/firmware/radio.md`; the tests hold the bytes. GFSK is the packet type the
//! TOWER personality needs; LoRa is another packet type here (D4), its settings to come with it.

#![no_std]

/// The radio's SPI with its chip select and busy line.
pub trait Bus {
    type Error;

    /// One command: waits until the radio is not busy, takes the chip select, writes each of
    /// `write` in turn, then reads `read.len()` bytes, and lets the chip select go. What the
    /// radio answers while the command is written is not kept.
    fn transfer(&mut self, write: &[&[u8]], read: &mut [u8]) -> Result<(), Self::Error>;
}

/// Where the RF switch connects the antenna (docs/hardware.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    Off,
    Receive,
    /// To the low-power PA, the only one wired on this module's board.
    TransmitLowPower,
}

/// The RF switch: on the north bridge, PC3, PC4 and PC5.
pub trait Switch {
    fn set(&mut self, path: Path);
}

/// The byte sent where the radio answers.
const NOP: u8 = 0x00;

/// The radio's commands (RM0453, table 36).
mod opcode {
    pub const SET_STANDBY: u8 = 0x80;
    pub const SET_RX: u8 = 0x82;
    pub const SET_TX: u8 = 0x83;
    pub const SET_RF_FREQUENCY: u8 = 0x86;
    pub const CALIBRATE: u8 = 0x89;
    pub const SET_PACKET_TYPE: u8 = 0x8A;
    pub const SET_MODULATION_PARAMS: u8 = 0x8B;
    pub const SET_PACKET_PARAMS: u8 = 0x8C;
    pub const SET_TX_PARAMS: u8 = 0x8E;
    pub const SET_BUFFER_BASE_ADDRESS: u8 = 0x8F;
    pub const SET_PA_CONFIG: u8 = 0x95;
    pub const SET_REGULATOR_MODE: u8 = 0x96;
    pub const SET_TCXO_MODE: u8 = 0x97;
    pub const CALIBRATE_IMAGE: u8 = 0x98;
    pub const SET_DIO_IRQ_PARAMS: u8 = 0x08;
    pub const CLEAR_IRQ_STATUS: u8 = 0x02;
    pub const CLEAR_DEVICE_ERRORS: u8 = 0x07;
    pub const WRITE_REGISTER: u8 = 0x0D;
    pub const WRITE_BUFFER: u8 = 0x0E;
    pub const GET_IRQ_STATUS: u8 = 0x12;
    pub const GET_RX_BUFFER_STATUS: u8 = 0x13;
    pub const GET_PACKET_STATUS: u8 = 0x14;
    pub const GET_RSSI_INST: u8 = 0x15;
    pub const GET_DEVICE_ERRORS: u8 = 0x17;
    pub const READ_REGISTER: u8 = 0x1D;
    pub const READ_BUFFER: u8 = 0x1E;
    pub const GET_STATUS: u8 = 0xC0;
}

/// The registers used.
mod register {
    /// The GFSK sync word, up to 8 bytes.
    pub const SYNC_WORD: u16 = 0x06C0;
    /// The receiver's gain.
    pub const RX_GAIN: u16 = 0x08AC;
}

/// The radio's crystal, the TCXO inside the module.
const XTAL_HZ: u64 = 32_000_000;

/// The clock in standby.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standby {
    /// The 13 MHz RC oscillator (the TCXO may be off).
    Rc = 0,
    /// The TCXO.
    Xosc = 1,
}

/// The regulator that feeds the radio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Regulator {
    Ldo = 0,
    Smps = 1,
}

/// The voltage the radio gives its TCXO.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcxoVoltage {
    /// What the north bridge's module wants.
    V1_7 = 0x02,
    V3_0 = 0x06,
}

/// The band the image calibration is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageBand {
    /// 863 to 870 MHz.
    Eu868,
    /// 902 to 928 MHz.
    Us915,
}

/// The packet type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketType {
    Gfsk = 0,
    Lora = 1,
}

/// The GFSK receive filter (RM0453, table 41): its width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bandwidth {
    Khz117 = 0x0B,
    Khz156 = 0x1A,
    Khz187 = 0x12,
    Khz234 = 0x0A,
    Khz312 = 0x19,
}

/// GFSK's modulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GfskModulation {
    /// The bit rate in thousandths of a bit a second (TOWER: 19 216 600, 19 216.6 bit/s).
    pub millibits: u32,
    /// The frequency deviation in Hz.
    pub deviation_hz: u32,
    /// The Gaussian filter: BT 1 is `0x0B`, BT 0.5 `0x09`, none `0x00`.
    pub pulse_shape: u8,
    pub bandwidth: Bandwidth,
}

impl GfskModulation {
    /// TOWER's (`docs/firmware/radio.md`): 19 216.6 bit/s, 19.8 kHz, BT 1.
    pub const fn tower(bandwidth: Bandwidth) -> Self {
        GfskModulation {
            millibits: 19_216_600,
            deviation_hz: 19_800,
            pulse_shape: 0x0B,
            bandwidth,
        }
    }

    /// The bit rate's register value: 32 x the crystal / the bit rate, rounded.
    pub fn bitrate_word(&self) -> u32 {
        let rate = u64::from(self.millibits.max(1));
        ((32 * XTAL_HZ * 1000 + rate / 2) / rate) as u32
    }

    /// The deviation's register value: the deviation x 2^25 / the crystal, rounded.
    pub fn deviation_word(&self) -> u32 {
        (((u64::from(self.deviation_hz) << 25) + XTAL_HZ / 2) / XTAL_HZ) as u32
    }
}

/// How long the preamble detector waits for (RM0453, `PreambleDetectorLength`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreambleDetector {
    Off = 0x00,
    Bits8 = 0x04,
    Bits16 = 0x05,
    Bits24 = 0x06,
    Bits32 = 0x07,
}

/// GFSK's packet: always raw here, fixed length, the radio's CRC and whitening off (TOWER's are
/// made in software, `fiber-tower-air`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GfskPacket {
    /// The preamble sent, in bits.
    pub preamble_bits: u16,
    pub detector: PreambleDetector,
    /// The sync word's length in bits (its bytes: [`SubGhz::set_sync_word`]); 0 for none.
    pub sync_bits: u8,
    /// The bytes sent or received after the sync word.
    pub length: u8,
}

/// The interrupts (RM0453, table 37).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Irq(pub u16);

impl Irq {
    pub const TX_DONE: Irq = Irq(1 << 0);
    pub const RX_DONE: Irq = Irq(1 << 1);
    pub const PREAMBLE: Irq = Irq(1 << 2);
    pub const SYNC_WORD: Irq = Irq(1 << 3);
    pub const CRC_ERROR: Irq = Irq(1 << 6);
    pub const TIMEOUT: Irq = Irq(1 << 9);
    pub const ALL: Irq = Irq(0x03FF);

    pub const fn or(self, other: Irq) -> Irq {
        Irq(self.0 | other.0)
    }

    pub const fn contains(self, other: Irq) -> bool {
        self.0 & other.0 == other.0
    }
}

/// How long a receive or a send may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timeout {
    /// Once, with no end of its own.
    None,
    /// Receive until told otherwise, packet after packet.
    Continuous,
    /// In steps of 15.625 us, at most 2^24 - 1.
    Steps(u32),
}

impl Timeout {
    fn bytes(self) -> [u8; 3] {
        let steps = match self {
            Timeout::None => 0,
            Timeout::Continuous => 0xFF_FFFF,
            Timeout::Steps(steps) => steps.min(0xFF_FFFE),
        };
        let [_, a, b, c] = steps.to_be_bytes();
        [a, b, c]
    }
}

/// The low-power PA: its configuration and the power asked of it (RM0453, the PA's optimal
/// settings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LowPowerPa {
    pub duty_cycle: u8,
    /// In dBm, -17 to +14.
    pub power: i8,
}

impl LowPowerPa {
    /// +14 dBm, the limit of the 868.0 to 868.6 MHz band (25 mW).
    pub const DBM_14: LowPowerPa = LowPowerPa {
        duty_cycle: 0x04,
        power: 14,
    };
    /// +10 dBm.
    pub const DBM_10: LowPowerPa = LowPowerPa {
        duty_cycle: 0x01,
        power: 13,
    };
}

/// How long the PA takes to rise and fall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ramp {
    Us10 = 0x00,
    Us40 = 0x02,
    Us200 = 0x04,
    Us800 = 0x05,
}

/// The radio's status byte (RM0453, `GetStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status(pub u8);

/// The radio's mode, from its status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    StandbyRc,
    StandbyXosc,
    Fs,
    Rx,
    Tx,
    Other(u8),
}

impl Status {
    pub fn mode(self) -> Mode {
        match (self.0 >> 4) & 7 {
            2 => Mode::StandbyRc,
            3 => Mode::StandbyXosc,
            4 => Mode::Fs,
            5 => Mode::Rx,
            6 => Mode::Tx,
            other => Mode::Other(other),
        }
    }

    /// The last command's result: 2 data, 3 timeout, 4 error, 5 failed, 6 sent.
    pub fn command(self) -> u8 {
        (self.0 >> 1) & 7
    }
}

/// What a GFSK packet was received with (`GetPacketStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GfskPacketStatus {
    pub rx_status: u8,
    /// The signal strength at the sync word and over the packet, in dBm.
    pub rssi_sync: i16,
    pub rssi_average: i16,
}

/// A signal strength byte in dBm: its value / 2, negative.
fn dbm(raw: u8) -> i16 {
    -(i16::from(raw) / 2)
}

/// The radio.
pub struct SubGhz<B: Bus> {
    bus: B,
}

impl<B: Bus> SubGhz<B> {
    pub fn new(bus: B) -> Self {
        SubGhz { bus }
    }

    pub fn bus(&mut self) -> &mut B {
        &mut self.bus
    }

    fn set(&mut self, opcode: u8, params: &[u8]) -> Result<(), B::Error> {
        self.bus.transfer(&[&[opcode], params], &mut [])
    }

    /// A command that answers: `read.len()` bytes after its status.
    fn get(&mut self, opcode: u8, params: &[u8], read: &mut [u8]) -> Result<(), B::Error> {
        self.bus.transfer(&[&[opcode], params, &[NOP]], read)
    }

    pub fn status(&mut self) -> Result<Status, B::Error> {
        let mut status = [0];
        self.bus.transfer(&[&[opcode::GET_STATUS]], &mut status)?;
        Ok(Status(status[0]))
    }

    pub fn standby(&mut self, clock: Standby) -> Result<(), B::Error> {
        self.set(opcode::SET_STANDBY, &[clock as u8])
    }

    pub fn set_regulator(&mut self, regulator: Regulator) -> Result<(), B::Error> {
        self.set(opcode::SET_REGULATOR_MODE, &[regulator as u8])
    }

    /// The TCXO's supply, and how long it takes to settle, in steps of 15.625 us.
    pub fn set_tcxo(&mut self, voltage: TcxoVoltage, settle_steps: u32) -> Result<(), B::Error> {
        let [_, a, b, c] = settle_steps.min(0xFF_FFFF).to_be_bytes();
        self.set(opcode::SET_TCXO_MODE, &[voltage as u8, a, b, c])
    }

    /// Every calibration: the oscillators, the PLL, the ADC, the image.
    pub fn calibrate_all(&mut self) -> Result<(), B::Error> {
        self.set(opcode::CALIBRATE, &[0x7F])
    }

    pub fn calibrate_image(&mut self, band: ImageBand) -> Result<(), B::Error> {
        let range = match band {
            ImageBand::Eu868 => [0xD7, 0xDB],
            ImageBand::Us915 => [0xE1, 0xE9],
        };
        self.set(opcode::CALIBRATE_IMAGE, &range)
    }

    /// What went wrong at the start or in a calibration (RM0453, `GetDeviceErrors`). The radio
    /// sets `XOSC_START_ERR` (0x0020) at its start, before it is told about the TCXO.
    pub fn device_errors(&mut self) -> Result<u16, B::Error> {
        let mut errors = [0; 2];
        self.get(opcode::GET_DEVICE_ERRORS, &[], &mut errors)?;
        Ok(u16::from_be_bytes(errors))
    }

    pub fn clear_device_errors(&mut self) -> Result<(), B::Error> {
        self.set(opcode::CLEAR_DEVICE_ERRORS, &[0, 0])
    }

    pub fn set_packet_type(&mut self, packet_type: PacketType) -> Result<(), B::Error> {
        self.set(opcode::SET_PACKET_TYPE, &[packet_type as u8])
    }

    pub fn set_rf_frequency(&mut self, hz: u32) -> Result<(), B::Error> {
        let word = ((u64::from(hz) << 25) / XTAL_HZ) as u32;
        self.set(opcode::SET_RF_FREQUENCY, &word.to_be_bytes())
    }

    pub fn set_gfsk_modulation(&mut self, modulation: &GfskModulation) -> Result<(), B::Error> {
        let [_, br0, br1, br2] = modulation.bitrate_word().to_be_bytes();
        let [_, fd0, fd1, fd2] = modulation.deviation_word().to_be_bytes();
        let params = [
            br0,
            br1,
            br2,
            modulation.pulse_shape,
            modulation.bandwidth as u8,
            fd0,
            fd1,
            fd2,
        ];
        self.set(opcode::SET_MODULATION_PARAMS, &params)
    }

    pub fn set_gfsk_packet(&mut self, packet: &GfskPacket) -> Result<(), B::Error> {
        let [p0, p1] = packet.preamble_bits.to_be_bytes();
        // No address filter, fixed length, the radio's CRC off (0x01), its whitening off.
        let params = [
            p0,
            p1,
            packet.detector as u8,
            packet.sync_bits,
            0x00,
            0x00,
            packet.length,
            0x01,
            0x00,
        ];
        self.set(opcode::SET_PACKET_PARAMS, &params)
    }

    /// The sync word's bytes, as they go on air; at most 8.
    pub fn set_sync_word(&mut self, sync: &[u8]) -> Result<(), B::Error> {
        self.write_register(register::SYNC_WORD, &sync[..sync.len().min(8)])
    }

    /// The boosted gain reads 1 to 2 dB lower on the noise floor (docs/bench-checks.md, check 4).
    pub fn set_rx_gain(&mut self, boosted: bool) -> Result<(), B::Error> {
        self.write_register(register::RX_GAIN, &[if boosted { 0x96 } else { 0x94 }])
    }

    pub fn write_register(&mut self, address: u16, values: &[u8]) -> Result<(), B::Error> {
        self.bus.transfer(
            &[&[opcode::WRITE_REGISTER], &address.to_be_bytes(), values],
            &mut [],
        )
    }

    pub fn read_register(&mut self, address: u16) -> Result<u8, B::Error> {
        let mut value = [0];
        self.get(opcode::READ_REGISTER, &address.to_be_bytes(), &mut value)?;
        Ok(value[0])
    }

    /// Where in the radio's 256-byte buffer a packet to send starts, and a received one.
    pub fn set_buffer_base(&mut self, tx: u8, rx: u8) -> Result<(), B::Error> {
        self.set(opcode::SET_BUFFER_BASE_ADDRESS, &[tx, rx])
    }

    pub fn write_buffer(&mut self, offset: u8, data: &[u8]) -> Result<(), B::Error> {
        self.bus
            .transfer(&[&[opcode::WRITE_BUFFER, offset], data], &mut [])
    }

    pub fn read_buffer(&mut self, offset: u8, out: &mut [u8]) -> Result<(), B::Error> {
        self.get(opcode::READ_BUFFER, &[offset], out)
    }

    /// Which interrupts there are; on the STM32WL they reach the core as one, the radio's.
    pub fn set_irq(&mut self, irq: Irq) -> Result<(), B::Error> {
        let [a, b] = irq.0.to_be_bytes();
        self.set(opcode::SET_DIO_IRQ_PARAMS, &[a, b, a, b, 0, 0, 0, 0])
    }

    pub fn irq(&mut self) -> Result<Irq, B::Error> {
        let mut irq = [0; 2];
        self.get(opcode::GET_IRQ_STATUS, &[], &mut irq)?;
        Ok(Irq(u16::from_be_bytes(irq)))
    }

    pub fn clear_irq(&mut self, irq: Irq) -> Result<(), B::Error> {
        self.set(opcode::CLEAR_IRQ_STATUS, &irq.0.to_be_bytes())
    }

    pub fn receive(&mut self, timeout: Timeout) -> Result<(), B::Error> {
        self.set(opcode::SET_RX, &timeout.bytes())
    }

    pub fn send(&mut self, timeout: Timeout) -> Result<(), B::Error> {
        self.set(opcode::SET_TX, &timeout.bytes())
    }

    /// The channel's signal strength now, in dBm.
    pub fn rssi(&mut self) -> Result<i16, B::Error> {
        let mut raw = [0];
        self.get(opcode::GET_RSSI_INST, &[], &mut raw)?;
        Ok(dbm(raw[0]))
    }

    /// The received packet's length and where it starts in the buffer.
    pub fn rx_buffer_status(&mut self) -> Result<(u8, u8), B::Error> {
        let mut status = [0; 2];
        self.get(opcode::GET_RX_BUFFER_STATUS, &[], &mut status)?;
        Ok((status[0], status[1]))
    }

    pub fn gfsk_packet_status(&mut self) -> Result<GfskPacketStatus, B::Error> {
        let mut status = [0; 3];
        self.get(opcode::GET_PACKET_STATUS, &[], &mut status)?;
        Ok(GfskPacketStatus {
            rx_status: status[0],
            rssi_sync: dbm(status[1]),
            rssi_average: dbm(status[2]),
        })
    }

    /// The low-power PA at `pa`, and its ramp.
    pub fn set_low_power_pa(&mut self, pa: LowPowerPa, ramp: Ramp) -> Result<(), B::Error> {
        // The duty cycle, no high-power PA (0), the low-power PA (1), and 1.
        self.set(opcode::SET_PA_CONFIG, &[pa.duty_cycle, 0x00, 0x01, 0x01])?;
        self.set(
            opcode::SET_TX_PARAMS,
            &[pa.power.clamp(-17, 14) as u8, ramp as u8],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::vec::Vec;

    /// What was written in each transfer, and the bytes to answer with.
    #[derive(Default)]
    struct Recorder {
        written: Vec<Vec<u8>>,
        answers: Vec<Vec<u8>>,
    }

    impl Bus for Recorder {
        type Error = ();
        fn transfer(&mut self, write: &[&[u8]], read: &mut [u8]) -> Result<(), ()> {
            self.written.push(write.concat());
            if !read.is_empty() {
                let answer = self.answers.remove(0);
                read.copy_from_slice(&answer);
            }
            Ok(())
        }
    }

    fn radio() -> SubGhz<Recorder> {
        SubGhz::new(Recorder::default())
    }

    /// The bytes the radio spike sent on the bench (hil/radio-spike, docs/firmware/radio.md),
    /// command by command.
    #[test]
    fn the_bring_up_as_the_bench_proved_it() {
        let mut r = radio();
        r.standby(Standby::Rc).unwrap();
        r.set_tcxo(TcxoVoltage::V1_7, 64).unwrap();
        r.clear_irq(Irq::ALL).unwrap();
        r.calibrate_all().unwrap();
        r.calibrate_image(ImageBand::Eu868).unwrap();
        r.clear_device_errors().unwrap();
        r.set_regulator(Regulator::Ldo).unwrap();
        let written: &[&[u8]] = &[
            &[0x80, 0x00],
            &[0x97, 0x02, 0x00, 0x00, 0x40],
            &[0x02, 0x03, 0xFF],
            &[0x89, 0x7F],
            &[0x98, 0xD7, 0xDB],
            &[0x07, 0x00, 0x00],
            &[0x96, 0x00],
        ];
        assert_eq!(r.bus.written, written);
    }

    #[test]
    fn gfsk_as_tower_sends_it() {
        let mut r = radio();
        r.set_packet_type(PacketType::Gfsk).unwrap();
        r.set_rf_frequency(868_100_000).unwrap();
        r.set_gfsk_modulation(&GfskModulation::tower(Bandwidth::Khz187))
            .unwrap();
        r.set_gfsk_packet(&GfskPacket {
            preamble_bits: 32,
            detector: PreambleDetector::Bits16,
            sync_bits: 32,
            length: 64,
        })
        .unwrap();
        r.set_sync_word(&[0x15, 0x47, 0x62, 0xDB]).unwrap();
        r.set_buffer_base(0x00, 0x80).unwrap();
        r.set_irq(
            Irq::TX_DONE
                .or(Irq::RX_DONE)
                .or(Irq::PREAMBLE)
                .or(Irq::SYNC_WORD)
                .or(Irq::CRC_ERROR)
                .or(Irq::TIMEOUT),
        )
        .unwrap();
        r.receive(Timeout::Continuous).unwrap();
        r.set_rx_gain(true).unwrap();
        let written: &[&[u8]] = &[
            &[0x8A, 0x00],
            &[0x86, 0x36, 0x41, 0x99, 0x99],
            // 32 x 32 MHz / 19 216.6 bit/s = 53 287.3, and 19.8 kHz x 2^25 / 32 MHz = 20 761.6.
            &[0x8B, 0x00, 0xD0, 0x27, 0x0B, 0x12, 0x00, 0x51, 0x1A],
            &[0x8C, 0x00, 0x20, 0x05, 0x20, 0x00, 0x00, 0x40, 0x01, 0x00],
            &[0x0D, 0x06, 0xC0, 0x15, 0x47, 0x62, 0xDB],
            &[0x8F, 0x00, 0x80],
            &[0x08, 0x02, 0x4F, 0x02, 0x4F, 0x00, 0x00, 0x00, 0x00],
            &[0x82, 0xFF, 0xFF, 0xFF],
            &[0x0D, 0x08, 0xAC, 0x96],
        ];
        assert_eq!(r.bus.written, written);
    }

    #[test]
    fn sending_on_the_low_power_pa() {
        let mut r = radio();
        r.set_low_power_pa(LowPowerPa::DBM_14, Ramp::Us200).unwrap();
        r.set_low_power_pa(LowPowerPa::DBM_10, Ramp::Us200).unwrap();
        r.write_buffer(0, &[0xEF, 0x6F]).unwrap();
        r.send(Timeout::None).unwrap();
        let written: &[&[u8]] = &[
            &[0x95, 0x04, 0x00, 0x01, 0x01],
            &[0x8E, 0x0E, 0x04],
            &[0x95, 0x01, 0x00, 0x01, 0x01],
            &[0x8E, 0x0D, 0x04],
            &[0x0E, 0x00, 0xEF, 0x6F],
            &[0x83, 0x00, 0x00, 0x00],
        ];
        assert_eq!(r.bus.written, written);
        // Never more than +14 dBm, nor less than -17.
        r.set_low_power_pa(
            LowPowerPa {
                duty_cycle: 4,
                power: 20,
            },
            Ramp::Us40,
        )
        .unwrap();
        assert_eq!(r.bus.written.last().unwrap(), &[0x8E, 0x0E, 0x02]);
        r.set_low_power_pa(
            LowPowerPa {
                duty_cycle: 1,
                power: -30,
            },
            Ramp::Us40,
        )
        .unwrap();
        assert_eq!(r.bus.written.last().unwrap(), &[0x8E, 0xEF, 0x02]);
    }

    #[test]
    fn what_the_radio_answers() {
        let mut r = radio();
        r.bus.answers = std::vec![
            std::vec![0xD2],
            std::vec![0x02, 0x4A],
            std::vec![0x00, 0x20],
            std::vec![0xE8],
            std::vec![0x13, 0x80],
            std::vec![0x02, 0x96, 0x98],
            std::vec![0xA5, 0x5A],
            std::vec![0x94],
        ];
        let status = r.status().unwrap();
        assert_eq!((status.mode(), status.command()), (Mode::Rx, 1));
        let irq = r.irq().unwrap();
        assert!(irq.contains(Irq::RX_DONE) && irq.contains(Irq::SYNC_WORD) && !irq.contains(Irq::TX_DONE));
        assert_eq!(r.device_errors().unwrap(), 0x0020);
        assert_eq!(r.rssi().unwrap(), -116);
        assert_eq!(r.rx_buffer_status().unwrap(), (0x13, 0x80));
        let packet = r.gfsk_packet_status().unwrap();
        assert_eq!((packet.rssi_sync, packet.rssi_average), (-75, -76));
        let mut out = [0; 2];
        r.read_buffer(0x80, &mut out).unwrap();
        assert_eq!(out, [0xA5, 0x5A]);
        assert_eq!(r.read_register(0x08AC).unwrap(), 0x94);
        // A command that answers writes a NOP where its status comes back.
        let written: &[&[u8]] = &[
            &[0xC0],
            &[0x12, 0x00],
            &[0x17, 0x00],
            &[0x15, 0x00],
            &[0x13, 0x00],
            &[0x14, 0x00],
            &[0x1E, 0x80, 0x00],
            &[0x1D, 0x08, 0xAC, 0x00],
        ];
        assert_eq!(r.bus.written, written);
    }
}
