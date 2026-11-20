//! The made-up air the gateway's tests run on (`gateway.rs`, `pairing.rs`): a radio and a clock
//! sharing one time that moves only when the gateway waits, so the turnarounds and the windows
//! run in exact milliseconds, and the frames as a TOWER node seals them.

// Each test file uses its own part of this module.
#![allow(dead_code)]
// Tests may unwrap (`clippy.toml`), but clippy sees that only inside `#[test]` functions.
#![allow(clippy::unwrap_used)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use fiber_tower_air::frame::{Ccm, FrameType, Header, SoftAes, open_frame, seal_frame};
use fiber_tower_air::gateway::{Clock, Gateway, Watermark};
use fiber_tower_air::radio::{Radio, Received};

pub const GATEWAY: u32 = 0x2222_2222;
pub const NODE: u32 = 0x1111_1111;
pub const OTHER: u32 = 0x3333_3333;
/// `net_confirmed`'s key, as in the captures.
pub const KEY: [u8; 16] = [
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00,
];
pub const NEW_KEY: [u8; 16] = [0x5a; 16];
/// The strength every made-up frame arrives at.
pub const RSSI: i16 = -70;

/// What the node does with a frame the gateway sent: frames to send back, each with when.
pub type Responder = Box<dyn FnMut(u64, &[u8]) -> Vec<(u64, Vec<u8>)>>;

/// The air and the time.
#[derive(Default)]
pub struct World {
    pub now: u64,
    /// Frames on their way to the gateway, by the time they arrive.
    pub incoming: VecDeque<(u64, Vec<u8>)>,
    /// What the gateway sent, and when.
    pub sent: Vec<(u64, Vec<u8>)>,
    pub responder: Option<Responder>,
}

pub type Shared = Rc<RefCell<World>>;

impl World {
    pub fn arrive(&mut self, at: u64, frame: Vec<u8>) {
        let i = self
            .incoming
            .iter()
            .position(|(t, _)| *t > at)
            .unwrap_or(self.incoming.len());
        self.incoming.insert(i, (at, frame));
    }
}

pub struct AirRadio(pub Shared);

impl Radio for AirRadio {
    type Error = ();

    fn set_channel(&mut self, _channel: u8) -> Result<(), ()> {
        Ok(())
    }

    async fn send(&mut self, frame: &[u8]) -> Result<(), ()> {
        let mut w = self.0.borrow_mut();
        let now = w.now;
        w.sent.push((now, frame.to_vec()));
        if let Some(mut respond) = w.responder.take() {
            for (at, reply) in respond(now, frame) {
                w.arrive(at, reply);
            }
            w.responder = Some(respond);
        }
        Ok(())
    }

    /// The next frame that begins before `until_ms`, taken when it has arrived whole: a frame is
    /// on air for its airtime before the time it arrives at. The time moves to its end, or to
    /// `until_ms` when none begins before it.
    async fn receive(&mut self, buf: &mut [u8], until_ms: u64) -> Result<Option<Received>, ()> {
        let mut w = self.0.borrow_mut();
        loop {
            match w.incoming.front() {
                // Gone by before the radio listened: a frame on air while nobody receives is lost.
                Some((at, frame)) if at.saturating_sub(airtime(frame)) < w.now && *at < w.now => {
                    w.incoming.pop_front();
                }
                Some((at, frame)) if at.saturating_sub(airtime(frame)) < until_ms => {
                    let at = *at;
                    let (_, frame) = w.incoming.pop_front().unwrap();
                    w.now = w.now.max(at);
                    buf[..frame.len()].copy_from_slice(&frame);
                    return Ok(Some(Received {
                        len: frame.len(),
                        rssi: RSSI,
                    }));
                }
                _ => {
                    w.now = w.now.max(until_ms);
                    return Ok(None);
                }
            }
        }
    }
}

/// How long `frame` is on air, as TOWER's duty cycle counts it.
pub fn airtime(frame: &[u8]) -> u64 {
    u64::from(tower_radio::frame_toa_ms(frame.len()))
}

pub struct AirClock(pub Shared);

impl Clock for AirClock {
    fn now_ms(&self) -> u64 {
        self.0.borrow().now
    }

    /// The time moves on; a frame on air meanwhile is lost, as nobody receives it.
    async fn sleep_until(&self, at_ms: u64) {
        let mut w = self.0.borrow_mut();
        w.now = w.now.max(at_ms);
    }
}

/// The watermark in memory; `fails` makes every write fail.
#[derive(Clone, Default)]
pub struct Kept {
    pub value: Rc<RefCell<Option<u32>>>,
    pub fails: bool,
}

impl Watermark for Kept {
    fn load(&mut self) -> Option<u32> {
        *self.value.borrow()
    }
    fn store(&mut self, watermark: u32) -> bool {
        if self.fails {
            return false;
        }
        *self.value.borrow_mut() = Some(watermark);
        true
    }
}

pub type Gw = Gateway<AirRadio, AirClock, SoftAes, Kept>;

/// Runs a future to its end: the made-up radio and clock never need a real wake-up.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..1_000_000 {
        if let Poll::Ready(out) = future.as_mut().poll(&mut cx) {
            return out;
        }
    }
    panic!("the future never ended");
}

/// A gateway that knows the node and may send.
pub fn gateway() -> (Gw, Shared, Kept) {
    let world = Shared::default();
    let kept = Kept::default();
    let mut gw = Gateway::new(
        AirRadio(world.clone()),
        AirClock(world.clone()),
        SoftAes,
        kept.clone(),
        GATEWAY,
    );
    assert!(gw.raise_tx_floor(0));
    assert!(gw.add_peer(NODE, &KEY, 0));
    (gw, world, kept)
}

/// A frame as a TOWER node seals it.
pub fn frame(
    key: &[u8; 16],
    frame_type: FrameType,
    src: u32,
    dest: u32,
    counter: u32,
    flags: u8,
    payload: &[u8],
) -> Vec<u8> {
    let header = Header {
        frame_type,
        flags,
        src,
        dest,
        counter,
        bulk_index: None,
    };
    let mut out = [0u8; 96];
    let n = seal_frame(&mut Ccm::new(SoftAes), key, &header, payload, &mut out).unwrap();
    out[..n].to_vec()
}

/// A frame the gateway sent, opened with `key`: its header and payload.
pub fn open(key: &[u8; 16], sent: &[u8]) -> (Header, Vec<u8>) {
    let mut buf = sent.to_vec();
    let (header, range) = open_frame(&mut Ccm::new(SoftAes), key, &mut buf).unwrap();
    (header, buf[range].to_vec())
}
