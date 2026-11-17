//! The south bridge's firmware (STM32C031C6): the status lights, the power of the 1-Wire ports
//! and two voltages, on the agent's word (docs/firmware/south.md).
//!
//! The logic is `fiber-south-core`, the messages `fiber-protocol`; this file is the board: the
//! lines, the serial link, the ADC, the watchdog, the faults kept across a reset, and the one
//! change it ever makes to the option bytes.
//!
//! Not run on a chip yet (TASKS.md, T16).
#![no_std]
#![no_main]

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU16, Ordering};

use cortex_m_rt::{ExceptionFrame, exception};
use embassy_executor::Spawner;
use embassy_stm32::adc::{Adc, Resolution, SampleTime};
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::usart::{self, BufferedUart, BufferedUartRx, BufferedUartTx};
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_stm32::{bind_interrupts, pac, peripherals};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Ticker};
use embedded_io_async::{Read, Write};
use fiber_protocol::south::{Desired, Fault, Hello, Message, Reset, Status};
use fiber_protocol::{FrameDecoder, MAX_WIRE, crc};
use fiber_south_core::{Engine, TICKS_PER_SECOND};

bind_interrupts!(struct Irqs {
    USART1 => usart::BufferedInterruptHandler<peripherals::USART1>;
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

// ---- Faults kept across a reset ----------------------------------------------------------------

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

/// A panic records its code without its place: the places of every panic in the HAL would cost
/// 3.2 KB of the 32 KB (docs/bench-checks.md, check 5).
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

/// Why the chip last started, from RCC_CSR2; the flags are cleared after.
fn reset_cause() -> Reset {
    let csr = pac::RCC.csr2().read();
    let cause = if csr.iwdgrstf() || csr.wwdgrstf() {
        Reset::Watchdog
    } else if csr.sftrstf() {
        Reset::Software
    } else if csr.pwrrstf() {
        Reset::PowerOn
    } else if csr.pinrstf() {
        Reset::Pin
    } else {
        Reset::Other
    };
    pac::RCC.csr2().modify(|w| w.set_rmvf(true));
    cause
}

/// The one change this firmware ever makes to the option bytes: where `nBOOT_SEL` is 1 (the
/// factory setting), BOOT0 is read from an option bit, and once the flash is written the CM4's
/// BOOT0 line no longer reaches the bootloader. So the bit is cleared, once, and the options
/// reloaded, which resets the chip. Nothing else is touched, and a chip with readout protection
/// is left alone (docs/hardware.md, "Flashing and recovery").
fn boot0_from_the_pin() {
    let flash = pac::FLASH;
    let optr = flash.optr().read();
    if !optr.n_boot_sel() || optr.0 & 0xFF != 0xAA {
        return;
    }
    while flash.sr().read().bsy() {}
    if flash.cr().read().lock() {
        flash.keyr().write_value(0x4567_0123);
        flash.keyr().write_value(0xCDEF_89AB);
    }
    if flash.cr().read().optlock() {
        flash.optkeyr().write_value(0x0819_2A3B);
        flash.optkeyr().write_value(0x4C5D_6E7F);
    }
    flash.optr().modify(|w| w.set_n_boot_sel(false));
    flash.cr().modify(|w| w.set_optstrt(true));
    while flash.sr().read().bsy() {}
    // Loads the options, and resets the chip.
    flash.cr().modify(|w| w.set_obl_launch(true));
}

// ---- The link ----------------------------------------------------------------------------------

enum Incoming {
    Desired { seq: u16, desired: Desired },
    HelloRequest,
}

static INCOMING: Channel<CriticalSectionRawMutex, Incoming, 2> = Channel::new();
/// Frames dropped since start. Only the link task writes it.
static DROPPED: AtomicU16 = AtomicU16::new(0);

#[embassy_executor::task]
async fn link(mut rx: BufferedUartRx<'static>) {
    let mut decoder = FrameDecoder::<MAX_WIRE>::new();
    let mut buf = [0u8; 16];
    loop {
        let Ok(n) = rx.read(&mut buf).await else { continue };
        for &byte in &buf[..n] {
            let Some(frame) = decoder.push(byte) else { continue };
            let incoming = match Message::decode(frame) {
                Ok((seq, Message::Desired(desired))) => Some(Incoming::Desired { seq, desired }),
                Ok((_, Message::HelloRequest)) => Some(Incoming::HelloRequest),
                _ => None,
            };
            match incoming {
                Some(incoming) => INCOMING.send(incoming).await,
                None => DROPPED.store(DROPPED.load(Ordering::Relaxed).wrapping_add(1), Ordering::Relaxed),
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

// ---- The board ---------------------------------------------------------------------------------

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    config.rcc.ls = embassy_stm32::rcc::LsConfig::default_lse();
    let p = embassy_stm32::init(config);

    let reset = reset_cause();
    let fault = fault_take();
    boot0_from_the_pin();
    // The "empty" flag is set by a first write through the bootloader and only looked at again at
    // power-on; cleared, a reset with BOOT0 low starts this firmware, not the bootloader.
    pac::FLASH.acr().modify(|w| w.set_empty(false));
    let hello = Hello {
        version: VERSION,
        image_crc: image_crc(),
        uid: embassy_stm32::uid::uid(),
        reset,
        fault,
        option_bytes: pac::FLASH.optr().read().0,
    };

    // The 18 LED lines (low = lit): the power LED's green and yellow, then each port's green and
    // red (docs/hardware.md, "South bridge pins").
    let mut leds = [
        Output::new(p.PA0, Level::High, Speed::Low),
        Output::new(p.PA1, Level::High, Speed::Low),
        Output::new(p.PA2, Level::High, Speed::Low),
        Output::new(p.PA3, Level::High, Speed::Low),
        Output::new(p.PA4, Level::High, Speed::Low),
        Output::new(p.PA5, Level::High, Speed::Low),
        Output::new(p.PA6, Level::High, Speed::Low),
        Output::new(p.PA7, Level::High, Speed::Low),
        Output::new(p.PB0, Level::High, Speed::Low),
        Output::new(p.PB1, Level::High, Speed::Low),
        Output::new(p.PB2, Level::High, Speed::Low),
        Output::new(p.PB10, Level::High, Speed::Low),
        Output::new(p.PB11, Level::High, Speed::Low),
        Output::new(p.PB12, Level::High, Speed::Low),
        Output::new(p.PB13, Level::High, Speed::Low),
        Output::new(p.PB14, Level::High, Speed::Low),
        Output::new(p.PB15, Level::High, Speed::Low),
        Output::new(p.PA8, Level::High, Speed::Low),
    ];
    // The power switches of ports 1 to 8 (high = on; 100 k pull-downs keep them off until then).
    let mut ports = [
        Output::new(p.PB9, Level::Low, Speed::Low),
        Output::new(p.PB8, Level::Low, Speed::Low),
        Output::new(p.PB7, Level::Low, Speed::Low),
        Output::new(p.PB6, Level::Low, Speed::Low),
        Output::new(p.PB5, Level::Low, Speed::Low),
        Output::new(p.PB4, Level::Low, Speed::Low),
        Output::new(p.PB3, Level::Low, Speed::Low),
        Output::new(p.PD3, Level::Low, Speed::Low),
    ];

    let mut uart_config = usart::Config::default();
    uart_config.baudrate = 115_200;
    // The serial port's two buffers, handed over once, for good.
    let (Some(tx_buf), Some(rx_buf)) = (
        cortex_m::singleton!(: [u8; 128] = [0; 128]),
        cortex_m::singleton!(: [u8; 128] = [0; 128]),
    ) else {
        fault_store(FAULT_PANIC, 0)
    };
    let Ok(uart) = BufferedUart::new(p.USART1, p.PA10, p.PA9, tx_buf, rx_buf, Irqs, uart_config) else {
        fault_store(FAULT_PANIC, 0)
    };
    let (tx, rx) = uart.split();
    let mut sender = Sender { tx, seq: 0 };
    match link(rx) {
        Ok(token) => spawner.spawn(token),
        Err(_) => fault_store(FAULT_PANIC, 0),
    }

    // The battery on PA11 (68 k / 68 k), the 12 V rail on PA12 (330 k / 68 k), the reference:
    // raw counts, turned into volts by the agent. The longest sample time, for the dividers'
    // high resistance (about 34 k and 56 k).
    let mut adc = Adc::new(p.ADC1, Resolution::BITS12);
    let mut vbat = p.PA11;
    let mut vin = p.PA12;
    let mut vref = adc.enable_vrefint();

    let mut watchdog = IndependentWatchdog::new(p.IWDG, 2_000_000);
    watchdog.unleash();

    sender.send(&Message::Hello(hello)).await;

    let mut engine = Engine::new();
    let mut ticker = Ticker::every(Duration::from_hz(u64::from(TICKS_PER_SECOND)));
    loop {
        ticker.next().await;
        watchdog.pet();

        while let Ok(incoming) = INCOMING.try_receive() {
            match incoming {
                Incoming::Desired { seq, desired } => {
                    engine.desired(desired);
                    let status = Status {
                        applied: seq,
                        ports: engine.ports(),
                        vbat: adc.blocking_read(&mut vbat, SampleTime::CYCLES640_5),
                        vin: adc.blocking_read(&mut vin, SampleTime::CYCLES640_5),
                        vref: adc.blocking_read(&mut vref, SampleTime::CYCLES640_5),
                        uptime: engine.uptime(),
                        rx_dropped: DROPPED.load(Ordering::Relaxed),
                    };
                    sender.send(&Message::Status(status)).await;
                }
                Incoming::HelloRequest => sender.send(&Message::Hello(hello)).await,
            }
        }

        let out = engine.tick();
        for (line, lit) in leds.iter_mut().zip(out.leds) {
            line.set_level(if lit { Level::Low } else { Level::High });
        }
        for (i, port) in ports.iter_mut().enumerate() {
            port.set_level(if out.ports & (1 << i) != 0 {
                Level::High
            } else {
                Level::Low
            });
        }
    }
}
