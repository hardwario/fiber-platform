//! The radio as the gateway sees it ([`fiber_tower_air::radio::Radio`]): the STM32WL's sub-GHz
//! radio through `fiber-subghz`, TOWER's packet through `fiber-tower-air`'s codec, with the
//! settings and the receive flow the radio spike proved on the bench (`docs/firmware/radio.md`,
//! `hil/radio-spike`).

use embassy_stm32::pac;
use embassy_time::{Duration, Instant, Timer};
use fiber_subghz::{
    Bandwidth, Bus, GfskModulation, GfskPacket, ImageBand, Irq, LowPowerPa, PacketType, Path,
    PreambleDetector, Ramp, Regulator, Standby, SubGhz, Switch, TcxoVoltage, Timeout,
};
use fiber_tower_air::packet::{self, MAX_PACKET, SYNC_WORD};
use fiber_tower_air::radio::{Radio, Received};

/// TOWER's channels in EU 868: 868.1, 868.3 and 868.5 MHz.
pub const CHANNELS: [u32; 3] = [868_100_000, 868_300_000, 868_500_000];

/// One byte on air at 19 216.6 bit/s, in microseconds.
const BYTE_US: u64 = 417;
/// How often the radio's interrupt status is read while it works.
const POLL: Duration = Duration::from_micros(100);
/// The longest a packet may take to go out: 255 bytes and the preamble are 111 ms.
const TX_LIMIT: Duration = Duration::from_millis(200);

/// The SUBGHZSPI, its chip select in `PWR_SUBGHZSPICR` and its busy line in `PWR_SR2`.
pub struct SpiBus;

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
        let start = Instant::now();
        while pac::PWR.sr2().read().rfbusys() {
            if start.elapsed() > Duration::from_millis(100) {
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

/// The RF switch on PC3, PC4 and PC5, as ST's HAL for the module sets it (`docs/hardware.md`).
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

/// The SUBGHZSPI at 12 MHz (48 MHz / 4; the radio takes up to 16 MHz), and PC3 to PC5 as the RF
/// switch's outputs.
fn board() {
    let rcc = pac::RCC;
    rcc.ahb2enr().modify(|w| w.set_gpiocen(true));
    rcc.apb3enr().modify(|w| w.set_subghzspien(true));
    let gpioc = pac::GPIOC;
    for pin in [3, 4, 5] {
        gpioc
            .moder()
            .modify(|w| w.set_moder(pin, pac::gpio::vals::Moder::OUTPUT));
    }
    let spi = pac::SUBGHZSPI;
    spi.cr1().write(|w| {
        w.set_mstr(pac::spi::vals::Mstr::MASTER);
        w.set_br(pac::spi::vals::Br::DIV4);
        w.set_ssm(true);
        w.set_ssi(true);
    });
    spi.cr2().write(|w| {
        w.set_ds(pac::spi::vals::Ds::BITS8);
        w.set_frxth(pac::spi::vals::Frxth::QUARTER);
    });
    spi.cr1().modify(|w| w.set_spe(true));
}

/// The radio with TOWER's packet.
pub struct AirRadio {
    radio: SubGhz<SpiBus>,
    switch: RfSwitch,
    frequency: u32,
}

impl AirRadio {
    /// The radio out of reset, on its TCXO at 1.7 V, calibrated (the image for 863 to 870 MHz),
    /// on the switching regulator, GFSK, on channel 0. `Err` when it does not answer.
    pub async fn new() -> Result<Self, ()> {
        board();
        let rcc = pac::RCC;
        rcc.csr().modify(|w| w.set_rfrst(true));
        Timer::after_micros(100).await;
        rcc.csr().modify(|w| w.set_rfrst(false));
        while rcc.csr().read().rfrstf() {}
        // A falling edge of the chip select wakes the radio.
        pac::PWR.subghzspicr().modify(|w| w.set_nss(false));
        Timer::after_micros(100).await;
        pac::PWR.subghzspicr().modify(|w| w.set_nss(true));
        let mut radio = SubGhz::new(SpiBus);
        radio.standby(Standby::Rc)?;
        // 1 ms to settle (64 steps of 15.625 us), then every calibration on it.
        radio.set_tcxo(TcxoVoltage::V1_7, 64)?;
        radio.clear_irq(Irq::ALL)?;
        radio.calibrate_all()?;
        radio.calibrate_image(ImageBand::Eu868)?;
        // The radio sets XOSC_START_ERR at its start, before it is told about the TCXO.
        radio.clear_device_errors()?;
        radio.set_packet_type(PacketType::Gfsk)?;
        radio.set_regulator(Regulator::Smps)?;
        let mut this = AirRadio {
            radio,
            switch: RfSwitch,
            frequency: CHANNELS[0],
        };
        this.switch.set(Path::Off);
        Ok(this)
    }

    /// GFSK as TOWER sends it, raw, `length` bytes after the sync word.
    fn tune(&mut self, length: u8) -> Result<(), ()> {
        let r = &mut self.radio;
        r.standby(Standby::Rc)?;
        r.set_rf_frequency(self.frequency)?;
        r.set_gfsk_modulation(&GfskModulation::tower(Bandwidth::Khz187))?;
        r.set_gfsk_packet(&GfskPacket {
            preamble_bits: 32,
            detector: PreambleDetector::Bits16,
            sync_bits: 32,
            length,
        })?;
        r.set_sync_word(&SYNC_WORD)?;
        r.set_buffer_base(0x00, 0x00)?;
        r.set_irq(Irq::TX_DONE.or(Irq::SYNC_WORD).or(Irq::RX_DONE))
    }

    /// Listening for the sync word, the packet as long as it may be.
    fn listen(&mut self) -> Result<(), ()> {
        self.tune(u8::MAX)?;
        self.switch.set(Path::Receive);
        self.radio.clear_irq(Irq::ALL)?;
        self.radio.receive(Timeout::Continuous)?;
        // The boosted gain, written after the radio entered receive (RM0453).
        self.radio.set_rx_gain(true)
    }
}

impl Radio for AirRadio {
    type Error = ();

    fn set_channel(&mut self, channel: u8) -> Result<(), ()> {
        self.frequency = *CHANNELS.get(usize::from(channel)).ok_or(())?;
        Ok(())
    }

    async fn send(&mut self, frame: &[u8]) -> Result<(), ()> {
        let mut out = [0u8; MAX_PACKET];
        let len = packet::encode(frame, &mut out).map_err(|_| ())?;
        self.tune(u8::try_from(len).map_err(|_| ())?)?;
        self.radio.set_low_power_pa(LowPowerPa::DBM_14, Ramp::Us200)?;
        self.radio.write_buffer(0, &out[..len])?;
        self.radio.clear_irq(Irq::ALL)?;
        self.switch.set(Path::TransmitLowPower);
        self.radio.send(Timeout::None)?;
        let start = Instant::now();
        let mut done = false;
        while start.elapsed() < TX_LIMIT {
            if self.radio.irq()?.contains(Irq::TX_DONE) {
                done = true;
                break;
            }
            Timer::after(POLL).await;
        }
        self.radio.clear_irq(Irq::ALL)?;
        self.radio.standby(Standby::Rc)?;
        self.switch.set(Path::Off);
        if done { Ok(()) } else { Err(()) }
    }

    /// Listens until a packet passes the codec: two byte times after the sync word its length is
    /// in, and one byte time after its end the radio is stopped and the packet read, so that it
    /// listens again at once (`docs/firmware/radio.md`, "Receiving a packet of any length"). A
    /// packet that does not pass is passed over. Dropped half way, the radio is left listening:
    /// the next send or receive tunes it again.
    async fn receive(&mut self, buf: &mut [u8]) -> Result<Received, ()> {
        self.listen()?;
        loop {
            let irq = self.radio.irq()?;
            if irq.contains(Irq::SYNC_WORD) {
                let sync_at = Instant::now();
                Timer::at(sync_at + Duration::from_micros(2 * BYTE_US)).await;
                let mut first = [0u8];
                self.radio.read_buffer(0, &mut first)?;
                let len = packet::packet_len(first[0]);
                let end = (len as u64 + 1) * BYTE_US;
                Timer::at(sync_at + Duration::from_micros(end)).await;
                let rssi = self.radio.gfsk_packet_status()?.rssi_sync;
                self.radio.standby(Standby::Rc)?;
                let mut raw = [0u8; MAX_PACKET];
                let n = len.min(MAX_PACKET);
                self.radio.read_buffer(0, &mut raw[..n])?;
                if let Ok(payload) = packet::decode(&mut raw[..n])
                    && let Some(out) = buf.get_mut(..payload.len())
                {
                    out.copy_from_slice(payload);
                    return Ok(Received {
                        len: payload.len(),
                        rssi,
                    });
                }
                self.listen()?;
            } else if irq.0 != 0 {
                self.radio.clear_irq(irq)?;
            }
            Timer::after(POLL).await;
        }
    }
}
