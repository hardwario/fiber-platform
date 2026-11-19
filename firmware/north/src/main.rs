//! The north bridge's firmware (STM32WL55, its Cortex-M4 core): the FIBER's TOWER gateway on the
//! radio, on the agent's word (docs/firmware/north.md, docs/firmware/north-link.md).
//!
//! The gateway is `fiber-tower-air`'s (T24, T47), the radio's commands `fiber-subghz`'s, the
//! messages `fiber-protocol`'s; this file is the board: the clock, the serial link, the random
//! numbers, the watchdog, the faults kept across a reset, and the loop that gives the radio its
//! time between the agent's messages. It keeps no list of nodes (D8): only the counter's
//! watermark (src/watermark.rs).
//!
//! Built and checked in CI; not run on a FIBER yet (TASKS.md, T25).
#![no_std]
#![no_main]

mod radio;
mod watermark;

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU16, Ordering};

use cortex_m_rt::{ExceptionFrame, exception};
use embassy_executor::Spawner;
use embassy_stm32::flash::Flash;
use embassy_stm32::rcc::{
    Hse, HseMode, HsePrescaler, Pll, PllMul, PllPreDiv, PllQDiv, PllRDiv, PllSource, Sysclk, mux,
};
use embassy_stm32::rng::{self, Rng};
use embassy_stm32::time::Hertz;
use embassy_stm32::usart::{self, BufferedUart, BufferedUartRx, BufferedUartTx};
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_stm32::{SharedData, bind_interrupts, pac, peripherals};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Timer};
use embedded_io_async::{Read, Write};
use fiber_protocol::north::{
    self, Answer, Bytes, Delivery, Fault, Hello, JoinRefused, KeyedJoin, Message, Node, Outcome, Paired,
    Personality, Reset, Sent, Status, Uplink,
};
use fiber_protocol::{FrameDecoder, MAX_WIRE, crc};
use fiber_tower_air::frame::SoftAes;
use fiber_tower_air::gateway::{Clock, Entropy, Gateway, JoinRefusal, PairingHost, SendResult, Watermark};

use crate::radio::AirRadio;
use crate::watermark::FlashWatermark;

bind_interrupts!(struct Irqs {
    USART1 => usart::BufferedInterruptHandler<peripherals::USART1>;
    RNG => rng::InterruptHandler<peripherals::RNG>;
});

/// The firmware's version, from its Cargo.toml.
const VERSION: [u8; 3] = [
    digits(env!("CARGO_PKG_VERSION_MAJOR")),
    digits(env!("CARGO_PKG_VERSION_MINOR")),
    digits(env!("CARGO_PKG_VERSION_PATCH")),
];

const fn digits(text: &str) -> u8 {
    let bytes = text.as_bytes();
    let mut value = 0u8;
    let mut i = 0;
    while i < bytes.len() {
        value = value * 10 + (bytes[i] - b'0');
        i += 1;
    }
    value
}

/// How long the radio listens before the agent's messages are looked at again.
const SLICE_MS: u64 = 100;
/// The same while a pairing window is open: a handshake answered in it runs on to its end.
const PAIR_SLICE_MS: u64 = 500;
/// A `Status` this often.
const STATUS_EVERY: Duration = Duration::from_secs(10);
/// At most one `JoinRefused` this often: a refused node asks again for its whole window.
const REFUSAL_EVERY: Duration = Duration::from_secs(1);
/// The watchdog: longer than the longest turn of the loop (a pairing slice and a whole
/// handshake, under 5 s).
const WATCHDOG_US: u32 = 8_000_000;

// ---- Faults kept across a reset (as the south bridge's) -----------------------------------------

const FAULT_MAGIC: u32 = 0xFA17_C0DE;
const FAULT_PANIC: u32 = 1;
const FAULT_HARD: u32 = 2;

/// In `.uninit`, which the start-up code leaves alone: a reset keeps it.
#[unsafe(link_section = ".uninit.FAULT")]
static mut FAULT: MaybeUninit<[u32; 3]> = MaybeUninit::uninit();

fn fault_store(code: u32, address: u32) -> ! {
    // SAFETY: one word-aligned static, written whole, just before the reset.
    unsafe {
        (&raw mut FAULT)
            .cast::<[u32; 3]>()
            .write_volatile([FAULT_MAGIC, code, address])
    };
    cortex_m::peripheral::SCB::sys_reset()
}

/// The fault from before the reset, once.
fn fault_take() -> Option<Fault> {
    // SAFETY: as above; read once at start, before anything else can write it.
    let kept = unsafe { (&raw const FAULT).cast::<[u32; 3]>().read_volatile() };
    unsafe { (&raw mut FAULT).cast::<[u32; 3]>().write_volatile([0; 3]) };
    (kept[0] == FAULT_MAGIC).then_some(Fault {
        code: kept[1] as u8,
        address: kept[2],
    })
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    fault_store(FAULT_PANIC, 0)
}

#[exception]
unsafe fn HardFault(frame: &ExceptionFrame) -> ! {
    fault_store(FAULT_HARD, frame.pc())
}

// ---- What the chip says about itself -----------------------------------------------------------

unsafe extern "C" {
    static __sidata: u32;
    static __sdata: u32;
    static __edata: u32;
}

/// The CRC-32 of the image in flash: from its start to the end of the initial data.
fn image_crc() -> u32 {
    let start = 0x0800_0000usize;
    let lma = &raw const __sidata as usize;
    let end = lma + (&raw const __edata as usize - &raw const __sdata as usize);
    // SAFETY: the flash from its start to the end of the image is mapped and read-only.
    let image = unsafe { core::slice::from_raw_parts(start as *const u8, end - start) };
    crc::crc32(image)
}

/// Why the chip last started, from RCC_CSR; the flags are cleared after.
fn reset_cause() -> Reset {
    let csr = pac::RCC.csr().read();
    let cause = if csr.iwdgrstf() || csr.wwdgrstf() {
        Reset::Watchdog
    } else if csr.sftrstf() {
        Reset::Software
    } else if csr.borrstf() {
        Reset::BrownOut
    } else if csr.pinrstf() {
        Reset::Pin
    } else {
        Reset::Other
    };
    pac::RCC.csr().modify(|w| w.set_rmvf(true));
    cause
}

// ---- The time and the random numbers, for the gateway ------------------------------------------

struct EmbassyClock;

impl Clock for EmbassyClock {
    fn now_ms(&self) -> u64 {
        Instant::now().as_millis()
    }

    async fn sleep_until(&self, at_ms: u64) {
        Timer::at(Instant::from_millis(at_ms)).await
    }
}

/// The STM32WL's true random number generator.
struct Trng(Rng<'static, peripherals::RNG>);

impl Entropy for Trng {
    fn fill(&mut self, out: &mut [u8]) -> bool {
        self.0.fill_bytes(out);
        true
    }
}

type Gw = Gateway<AirRadio, EmbassyClock, SoftAes, FlashWatermark>;

// ---- The link ----------------------------------------------------------------------------------

static INCOMING: Channel<CriticalSectionRawMutex, (u16, Message), 4> = Channel::new();
/// Frames dropped since start. Only the link task writes it.
static DROPPED: AtomicU16 = AtomicU16::new(0);

fn dropped() {
    DROPPED.store(DROPPED.load(Ordering::Relaxed).wrapping_add(1), Ordering::Relaxed);
}

#[embassy_executor::task]
async fn link(mut rx: BufferedUartRx<'static>) {
    let mut decoder = FrameDecoder::<MAX_WIRE>::new();
    let mut buf = [0u8; 32];
    loop {
        let Ok(n) = rx.read(&mut buf).await else { continue };
        for &byte in &buf[..n] {
            let Some(frame) = decoder.push(byte) else { continue };
            match Message::decode(frame) {
                Ok(message) => INCOMING.send(message).await,
                Err(_) => dropped(),
            }
        }
    }
}

struct Sender {
    tx: BufferedUartTx<'static>,
    seq: u16,
}

impl Sender {
    async fn send(&mut self, message: &Message) {
        let mut wire = [0u8; MAX_WIRE];
        if let Ok(n) = message.encode(self.seq, &mut wire) {
            let _ = self.tx.write_all(&wire[..n]).await;
        }
        self.seq = self.seq.wrapping_add(1);
    }
}

// ---- The north bridge --------------------------------------------------------------------------

/// Everything the loop owns: the gateway, made by the first `Settings` from the radio and the
/// watermark, which wait here until then.
struct North {
    sender: Sender,
    hello: Hello,
    radio: Option<AirRadio>,
    watermark: Option<FlashWatermark>,
    gateway: Option<Gw>,
    window: Option<PairingHost>,
    rng: Trng,
    last_refusal: Option<Instant>,
}

impl North {
    async fn answer(&mut self, seq: u16, result: Outcome) {
        self.sender.send(&Message::Answer(Answer { seq, result })).await;
    }

    fn status(&self) -> Status {
        let mut status = Status {
            uptime: Instant::now().as_secs() as u32,
            tx_floor: self.hello.tx_floor,
            rx_dropped: DROPPED.load(Ordering::Relaxed),
            ..Status::default()
        };
        if let Some(gw) = &self.gateway {
            let (taken_in, passed_over) = gw.counts();
            status.tx_floor = gw.tx_floor();
            status.can_send = gw.can_send();
            status.duty_left_ms = gw.duty_left_ms();
            status.taken_in = taken_in;
            status.passed_over = passed_over;
        }
        status
    }

    /// One message from the agent.
    async fn handle(&mut self, seq: u16, message: Message) {
        let result = match message {
            Message::HelloRequest => {
                let mut hello = self.hello;
                if let Some(gw) = &self.gateway {
                    hello.tx_floor = gw.tx_floor();
                }
                self.sender.send(&Message::Hello(hello)).await;
                return;
            }
            Message::SetPersonality(m) => match m.personality {
                Personality::TowerFsk => Outcome::Done,
            },
            Message::Settings(s) => self.settings(s),
            Message::NodeAdd(m) => match &mut self.gateway {
                None => Outcome::NotNow,
                Some(gw) if m.node == 0 || m.node == u32::MAX || m.node == gw.addr() => Outcome::Invalid,
                Some(gw) => done_or(gw.add_peer(m.node, &m.key, m.last_seen), Outcome::Full),
            },
            Message::NodeRemove(m) => match &mut self.gateway {
                None => Outcome::NotNow,
                Some(gw) => done_or(gw.remove_peer(m.node), Outcome::Unknown),
            },
            Message::ListNodes => {
                let mut nodes = [None; fiber_tower_air::gateway::MAX_PEERS];
                if let Some(gw) = &self.gateway {
                    for (slot, (node, last_seen, queued)) in nodes.iter_mut().zip(gw.peers()) {
                        *slot = Some(Node {
                            node,
                            last_seen,
                            queued,
                        });
                    }
                }
                for node in nodes.into_iter().flatten() {
                    self.sender.send(&Message::Node(node)).await;
                }
                Outcome::Done
            }
            Message::Queue(m) => match &mut self.gateway {
                None => Outcome::NotNow,
                Some(gw) => done_or(gw.set_waiting(m.node, m.data.as_slice()), Outcome::Unknown),
            },
            Message::Unqueue(m) => match &mut self.gateway {
                None => Outcome::NotNow,
                Some(gw) if gw.peer_last_seen(m.node).is_some() => {
                    gw.clear_waiting(m.node);
                    Outcome::Done
                }
                Some(_) => Outcome::Unknown,
            },
            Message::PairingOpen(m) => self.open_window(m.seconds),
            Message::PairingCancel => {
                if self.window.take().is_some() {
                    self.sender.send(&Message::PairingClosed).await;
                }
                Outcome::Done
            }
            Message::StatusRequest => {
                let status = self.status();
                self.sender.send(&Message::Status(status)).await;
                Outcome::Done
            }
            // What the north bridge sends, coming back: not for it.
            _ => {
                dropped();
                return;
            }
        };
        self.answer(seq, result).await;
    }

    /// The first `Settings` makes the gateway; its address is then fixed until a reset.
    fn settings(&mut self, s: north::Settings) -> Outcome {
        if s.address == 0 || s.address == u32::MAX || s.channel > 2 {
            return Outcome::Invalid;
        }
        if self.gateway.is_none() {
            let (Some(radio), Some(watermark)) = (self.radio.take(), self.watermark.take()) else {
                return Outcome::NotNow; // the radio did not answer at start
            };
            self.gateway = Some(Gateway::new(radio, EmbassyClock, SoftAes, watermark, s.address));
        }
        let Some(gw) = &mut self.gateway else {
            return Outcome::NotNow;
        };
        if gw.addr() != s.address {
            return Outcome::Invalid;
        }
        if !gw.raise_tx_floor(s.tx_floor) {
            return Outcome::Locked;
        }
        if gw.set_channel(s.channel).is_err() {
            return Outcome::Invalid;
        }
        gw.set_keyed_join(s.keyed_join);
        Outcome::Done
    }

    fn open_window(&mut self, seconds: u16) -> Outcome {
        let Some(gw) = &self.gateway else {
            return Outcome::NotNow;
        };
        if !(10..=600).contains(&seconds) {
            return Outcome::Invalid;
        }
        if self.window.is_some() {
            return Outcome::NotNow;
        }
        match gw.open_window(&mut self.rng, u64::from(seconds) * 1000) {
            Some(host) => {
                self.window = Some(host);
                Outcome::Done
            }
            None => Outcome::NoEntropy,
        }
    }

    /// One slice of the radio's time: listening for uplinks, or for nodes that want to join.
    async fn work(&mut self) {
        let North {
            sender,
            gateway,
            window,
            rng,
            ..
        } = self;
        let Some(gw) = gateway else {
            Timer::after_millis(SLICE_MS).await;
            return;
        };
        let now = Instant::now().as_millis();
        if let Some(host) = window {
            if let Some(peer) = gw.pair(host, rng, now + PAIR_SLICE_MS).await {
                // Added from counter 0 at once: its first confirmed uplinks come within seconds.
                let _ = gw.add_peer(peer.addr, &peer.key, 0);
                let paired = Paired {
                    node: peer.addr,
                    key: peer.key,
                    sas: peer.sas,
                };
                sender.send(&Message::Paired(paired)).await;
                *window = None;
            } else if Instant::now().as_millis() >= host.window_end_ms() {
                sender.send(&Message::PairingClosed).await;
                *window = None;
            }
        } else if let Some(up) = gw.recv(now + SLICE_MS).await {
            // Passed on before the next frame is taken in.
            let uplink = Uplink {
                node: up.src,
                counter: up.counter,
                rssi: up.rssi,
                confirmed: up.confirmed,
                data: Bytes::new(up.data()).unwrap_or_default(),
            };
            sender.send(&Message::Uplink(uplink)).await;
            if let Some(result) = gw.deliver_waiting(up.src).await {
                let delivery = Delivery {
                    node: up.src,
                    result: sent(result),
                    ack_rssi: gw.last_ack().map(|a| a.rssi),
                };
                sender.send(&Message::Delivery(delivery)).await;
            }
        }
        if let Some(node) = gw.take_keyed_join() {
            sender.send(&Message::KeyedJoin(KeyedJoin { node })).await;
        }
        if let Some((node, why)) = gw.take_join_refusal()
            && self.last_refusal.is_none_or(|at| at.elapsed() >= REFUSAL_EVERY)
        {
            self.last_refusal = Some(Instant::now());
            let refused = JoinRefused {
                node,
                why: refusal(why),
            };
            self.sender.send(&Message::JoinRefused(refused)).await;
        }
    }
}

/// `Done` when it was, `otherwise` when not.
fn done_or(done: bool, otherwise: Outcome) -> Outcome {
    if done { Outcome::Done } else { otherwise }
}

fn sent(result: SendResult) -> Sent {
    match result {
        SendResult::Delivered => Sent::Delivered,
        SendResult::NotDelivered => Sent::NotDelivered,
        SendResult::DutyLimited => Sent::DutyLimited,
        SendResult::Refused(_) => Sent::Refused,
    }
}

fn refusal(why: JoinRefusal) -> north::JoinRefusal {
    use north::JoinRefusal as Wire;
    match why {
        JoinRefusal::NotV3 => Wire::NotV3,
        JoinRefusal::AlreadyRegistered => Wire::AlreadyRegistered,
        JoinRefusal::InvalidAddress => Wire::InvalidAddress,
        JoinRefusal::WindowClosing => Wire::WindowClosing,
        JoinRefusal::LowOrderKey => Wire::LowOrderKey,
        JoinRefusal::KeyedOff => Wire::KeyedOff,
        JoinRefusal::KeyedUnknown => Wire::KeyedUnknown,
        JoinRefusal::KeyedNotAuthentic => Wire::KeyedNotAuthentic,
        JoinRefusal::KeyedStale { counter, last_seen } => Wire::KeyedStale { counter, last_seen },
        JoinRefusal::CannotSend => Wire::CannotSend,
    }
}

// ---- The board ---------------------------------------------------------------------------------

/// The data embassy shares with the other core. The Cortex-M0+ never runs on a FIBER, so its
/// place does not matter: an ordinary static.
static SHARED: MaybeUninit<SharedData> = MaybeUninit::uninit();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // 48 MHz from the TCXO: the HSE in bypass (which powers the TCXO), 32 MHz / 2 x 6 / 2.
    let mut config = embassy_stm32::Config::default();
    config.rcc.hse = Some(Hse {
        freq: Hertz(32_000_000),
        mode: HseMode::Bypass,
        prescaler: HsePrescaler::DIV1,
    });
    config.rcc.pll = Some(Pll {
        source: PllSource::HSE,
        prediv: PllPreDiv::DIV2,
        mul: PllMul::MUL6,
        divp: None,
        divq: Some(PllQDiv::DIV2),
        divr: Some(PllRDiv::DIV2),
    });
    config.rcc.sys = Sysclk::PLL1_R;
    config.rcc.mux.rngsel = mux::Rngsel::PLL1_Q;
    let p = embassy_stm32::init_primary(config, &SHARED);

    let reset = reset_cause();
    let fault = fault_take();
    // The "empty" flag is set by a first write through the bootloader and only looked at again at
    // power-on; cleared, a reset with BOOT0 low starts this firmware, not the bootloader.
    pac::FLASH.acr().modify(|w| w.set_empty(false));

    let mut uart_config = usart::Config::default();
    uart_config.baudrate = 115_200;
    let (Some(tx_buf), Some(rx_buf)) = (
        cortex_m::singleton!(: [u8; 512] = [0; 512]),
        cortex_m::singleton!(: [u8; 256] = [0; 256]),
    ) else {
        fault_store(FAULT_PANIC, 0)
    };
    let Ok(uart) = BufferedUart::new(p.USART1, p.PA10, p.PA9, tx_buf, rx_buf, Irqs, uart_config) else {
        fault_store(FAULT_PANIC, 0)
    };
    let (tx, rx) = uart.split();
    match link(rx) {
        Ok(token) => spawner.spawn(token),
        Err(_) => fault_store(FAULT_PANIC, 0),
    }

    let mut watchdog = IndependentWatchdog::new(p.IWDG, WATCHDOG_US);
    watchdog.unleash();

    let mut watermark = FlashWatermark::new(Flash::new_blocking(p.FLASH));
    let hello = Hello {
        version: VERSION,
        image_crc: image_crc(),
        uid: embassy_stm32::uid::uid(),
        reset,
        fault,
        personalities: 1,
        active: Personality::TowerFsk,
        tx_floor: watermark.load().unwrap_or(0),
    };
    let mut north = North {
        sender: Sender { tx, seq: 0 },
        hello,
        radio: AirRadio::new().await.ok(),
        watermark: Some(watermark),
        gateway: None,
        window: None,
        rng: Trng(Rng::new(p.RNG, Irqs)),
        last_refusal: None,
    };
    north.sender.send(&Message::Hello(hello)).await;

    let mut last_status = Instant::now();
    loop {
        watchdog.pet();
        while let Ok((seq, message)) = INCOMING.try_receive() {
            north.handle(seq, message).await;
        }
        if last_status.elapsed() >= STATUS_EVERY {
            last_status = Instant::now();
            let status = north.status();
            north.sender.send(&Message::Status(status)).await;
        }
        north.work().await;
    }
}
