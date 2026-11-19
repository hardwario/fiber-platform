//! The gateway's flow (`fiber_tower_air::gateway`, T24) against a radio and a clock made up in
//! software, sharing one time that moves only when the gateway waits: so the turnarounds and the
//! acknowledgement windows run in exact milliseconds. The nodes' frames are sealed by the same
//! codec, as a TOWER node seals them; at the end, the frames a real Core Module sent.

// Tests may unwrap (`clippy.toml`), but clippy sees that only inside `#[test]` functions, and this
// file has helpers too.
#![allow(clippy::unwrap_used)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use fiber_tower_air::frame::{Ccm, FrameType, Header, SoftAes, flags, open_frame, seal_frame};
use fiber_tower_air::gateway::{Clock, Gateway, MAX_REPS, Refusal, SendResult, Watermark};
use fiber_tower_air::radio::{Radio, Received};

const GATEWAY: u32 = 0x2222_2222;
const NODE: u32 = 0x1111_1111;
const OTHER: u32 = 0x3333_3333;
/// `net_confirmed`'s key, as in the captures.
const KEY: [u8; 16] = [
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00,
];
const NEW_KEY: [u8; 16] = [0x5a; 16];
/// The strength every made-up frame arrives at.
const RSSI: i16 = -70;

/// What the node does with a frame the gateway sent: frames to send back, each with when.
type Responder = Box<dyn FnMut(u64, &[u8]) -> Vec<(u64, Vec<u8>)>>;

/// The air and the time.
#[derive(Default)]
struct World {
    now: u64,
    /// Frames on their way to the gateway, by the time they arrive.
    incoming: VecDeque<(u64, Vec<u8>)>,
    /// What the gateway sent, and when.
    sent: Vec<(u64, Vec<u8>)>,
    responder: Option<Responder>,
}

type Shared = Rc<RefCell<World>>;

impl World {
    fn arrive(&mut self, at: u64, frame: Vec<u8>) {
        let i = self
            .incoming
            .iter()
            .position(|(t, _)| *t > at)
            .unwrap_or(self.incoming.len());
        self.incoming.insert(i, (at, frame));
    }
}

struct AirRadio(Shared);

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

    async fn receive(&mut self, buf: &mut [u8]) -> Result<Received, ()> {
        poll_fn(|_| {
            let mut w = self.0.borrow_mut();
            match w.incoming.front() {
                Some((at, _)) if *at <= w.now => {
                    let (_, frame) = w.incoming.pop_front().unwrap();
                    buf[..frame.len()].copy_from_slice(&frame);
                    Poll::Ready(Ok(Received {
                        len: frame.len(),
                        rssi: RSSI,
                    }))
                }
                _ => Poll::Pending,
            }
        })
        .await
    }
}

struct AirClock(Shared);

impl Clock for AirClock {
    fn now_ms(&self) -> u64 {
        self.0.borrow().now
    }

    async fn sleep_until(&self, at_ms: u64) {
        poll_fn(|_| {
            let mut w = self.0.borrow_mut();
            if w.now >= at_ms {
                return Poll::Ready(());
            }
            // A frame arrives first: the time moves to it, and a receiver racing this sleep takes
            // it on the next poll. Nobody receiving, the frame waits in the air.
            match w.incoming.front() {
                Some((at, _)) if *at > w.now && *at < at_ms => {
                    w.now = *at;
                    Poll::Pending
                }
                _ => {
                    w.now = at_ms;
                    Poll::Ready(())
                }
            }
        })
        .await
    }
}

/// The watermark in memory; `fails` makes every write fail.
#[derive(Clone, Default)]
struct Kept {
    value: Rc<RefCell<Option<u32>>>,
    fails: bool,
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

type Gw = Gateway<AirRadio, AirClock, SoftAes, Kept>;

/// Runs a future to its end: the made-up radio and clock never need a real wake-up.
fn block_on<F: Future>(future: F) -> F::Output {
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
fn gateway() -> (Gw, Shared, Kept) {
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
fn frame(
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

fn uplink(counter: u32, payload: &[u8]) -> Vec<u8> {
    frame(
        &KEY,
        FrameType::Data,
        NODE,
        GATEWAY,
        counter,
        flags::CONFIRMED,
        payload,
    )
}

/// A frame the gateway sent, opened with `key`: its header and payload.
fn open(key: &[u8; 16], sent: &[u8]) -> (Header, Vec<u8>) {
    let mut buf = sent.to_vec();
    let (header, range) = open_frame(&mut Ccm::new(SoftAes), key, &mut buf).unwrap();
    (header, buf[range].to_vec())
}

/// The acknowledgements the gateway sent: (when, the counter each answers, its flags).
fn acks(world: &Shared) -> Vec<(u64, u32, u8)> {
    world
        .borrow()
        .sent
        .iter()
        .filter_map(|(at, f)| {
            let (h, p) = open(&KEY, f);
            (h.frame_type == FrameType::Ack)
                .then(|| (*at, u32::from_le_bytes([p[0], p[1], p[2], p[3]]), p[5]))
        })
        .collect()
}

/// A node that acknowledges every data frame sent to it, 35 ms later (TOWER's measured round
/// trip), under its next counter.
fn acknowledging_node() -> Responder {
    let mut counter = 1000;
    Box::new(move |now, sent| {
        let (h, _) = open(&KEY, sent);
        if h.frame_type != FrameType::Data || h.flags & flags::CONFIRMED == 0 {
            return vec![];
        }
        counter += 1;
        let mut payload = [0u8; 6];
        payload[..4].copy_from_slice(&h.counter.to_le_bytes());
        payload[4] = -60i8 as u8;
        vec![(
            now + 35,
            frame(&KEY, FrameType::Ack, NODE, GATEWAY, counter, 0, &payload),
        )]
    })
}

#[test]
fn a_new_frame_is_taken_in_and_acknowledged() {
    let (mut gw, world, _) = gateway();
    world.borrow_mut().arrive(100, uplink(5, b"hello"));
    let up = block_on(gw.recv(1_000)).expect("taken in");
    assert_eq!((up.src, up.counter, up.rssi, up.confirmed), (NODE, 5, RSSI, true));
    assert_eq!(up.data(), b"hello");
    assert_eq!(gw.peer_last_seen(NODE), Some(5));

    // The acknowledgement: 20 ms after the frame, under the gateway's own first counter, the
    // counter it answers, the strength it heard, no flag.
    let sent = world.borrow().sent.clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, 120);
    let (h, p) = open(&KEY, &sent[0].1);
    assert_eq!(
        (h.frame_type, h.src, h.dest, h.flags),
        (FrameType::Ack, GATEWAY, NODE, 0)
    );
    assert_eq!(h.counter, 1);
    assert_eq!(p, [5, 0, 0, 0, RSSI as i8 as u8, 0]);

    // Not confirmed: taken in, not answered.
    let quiet = frame(&KEY, FrameType::Data, NODE, GATEWAY, 6, 0, b"hi");
    world.borrow_mut().arrive(500, quiet);
    let up = block_on(gw.recv(1_000)).expect("taken in");
    assert!(!up.confirmed);
    assert_eq!(world.borrow().sent.len(), 1);
    // Nothing more: the deadline.
    assert!(block_on(gw.recv(2_000)).is_none());
    assert_eq!(world.borrow().now, 2_000);
}

#[test]
fn a_lost_acknowledgement() {
    let (mut gw, world, _) = gateway();
    world.borrow_mut().arrive(0, uplink(7, b"once"));
    assert!(block_on(gw.recv(1_000)).is_some());
    // The node heard no acknowledgement and sends the same frame again, and again: answered
    // each time, delivered never again, and answered at most as often as a node can ask.
    for i in 1..=MAX_REPS as u64 + 2 {
        world.borrow_mut().arrive(300 * i, uplink(7, b"once"));
    }
    assert!(block_on(gw.recv(10_000)).is_none());
    let answered: Vec<u32> = acks(&world).iter().map(|a| a.1).collect();
    assert_eq!(
        answered,
        vec![7; MAX_REPS as usize],
        "the first and {} again",
        MAX_REPS - 1
    );
    // Each with a counter of its own.
    let counters: Vec<u32> = world
        .borrow()
        .sent
        .iter()
        .map(|(_, f)| open(&KEY, f).0.counter)
        .collect();
    assert_eq!(counters, (1..=MAX_REPS as u32).collect::<Vec<_>>());
}

#[test]
fn a_replay_is_dropped() {
    let (mut gw, world, _) = gateway();
    world.borrow_mut().arrive(0, uplink(9, b"new"));
    assert!(block_on(gw.recv(1_000)).is_some());
    // An earlier counter, authentic, and counter 0: dropped, not answered.
    world.borrow_mut().arrive(1_000, uplink(8, b"old"));
    world.borrow_mut().arrive(1_100, uplink(0, b"zero"));
    assert!(block_on(gw.recv(5_000)).is_none());
    assert_eq!(acks(&world).len(), 1);
    assert_eq!(gw.peer_last_seen(NODE), Some(9));
    // The next counter is new again.
    world.borrow_mut().arrive(6_000, uplink(10, b"next"));
    assert_eq!(block_on(gw.recv(7_000)).expect("taken in").data(), b"next");
}

#[test]
fn only_known_nodes_and_authentic_frames() {
    let (mut gw, world, _) = gateway();
    let mut w = world.borrow_mut();
    // A node the gateway has no key for; a known node under another key; a frame for another
    // gateway; an acknowledgement, not data; a damaged tag.
    w.arrive(
        0,
        frame(&KEY, FrameType::Data, OTHER, GATEWAY, 1, flags::CONFIRMED, b"who"),
    );
    w.arrive(
        100,
        frame(
            &NEW_KEY,
            FrameType::Data,
            NODE,
            GATEWAY,
            2,
            flags::CONFIRMED,
            b"forged",
        ),
    );
    w.arrive(
        200,
        frame(
            &KEY,
            FrameType::Data,
            NODE,
            OTHER,
            3,
            flags::CONFIRMED,
            b"not ours",
        ),
    );
    w.arrive(300, frame(&KEY, FrameType::Ack, NODE, GATEWAY, 4, 0, &[0; 6]));
    let mut damaged = uplink(5, b"damaged");
    *damaged.last_mut().unwrap() ^= 1;
    w.arrive(400, damaged);
    drop(w);
    assert!(block_on(gw.recv(1_000)).is_none());
    assert!(world.borrow().sent.is_empty(), "nothing answered");
    assert_eq!(gw.peer_last_seen(NODE), Some(0), "the lane did not move");
}

#[test]
fn a_node_keyed_again() {
    let (mut gw, world, _) = gateway();
    world.borrow_mut().arrive(0, uplink(50, b"before"));
    assert!(block_on(gw.recv(1_000)).is_some());
    // The same key again, with the agent's last-seen: the lane only moves up.
    assert!(gw.add_peer(NODE, &KEY, 40));
    assert_eq!(gw.peer_last_seen(NODE), Some(50));
    assert!(gw.add_peer(NODE, &KEY, 60));
    assert_eq!(gw.peer_last_seen(NODE), Some(60));
    // Paired again: a new key, its counter from 1. The old key opens nothing now.
    assert!(gw.set_waiting(NODE, b"old"));
    assert!(gw.add_peer(NODE, &NEW_KEY, 0));
    assert!(!gw.has_waiting(NODE), "what waited under the old key is dropped");
    world.borrow_mut().arrive(2_000, uplink(70, b"old key"));
    world.borrow_mut().arrive(
        2_100,
        frame(&NEW_KEY, FrameType::Data, NODE, GATEWAY, 1, 0, b"new key"),
    );
    assert_eq!(block_on(gw.recv(3_000)).expect("taken in").data(), b"new key");
    // Reserved and own addresses are refused; the table holds 16.
    assert!(!gw.add_peer(0, &KEY, 0));
    assert!(!gw.add_peer(u32::MAX, &KEY, 0));
    assert!(!gw.add_peer(GATEWAY, &KEY, 0));
    for i in 1..16 {
        assert!(gw.add_peer(0x4000_0000 + i, &KEY, 0));
    }
    assert!(!gw.add_peer(0x5000_0000, &KEY, 0), "full");
    assert!(gw.remove_peer(0x4000_0001));
    assert!(gw.add_peer(0x5000_0000, &KEY, 0));
}

#[test]
fn a_downlink_waits_for_the_node() {
    let (mut gw, world, _) = gateway();
    assert!(gw.set_waiting(NODE, b"set 21.5"));
    assert!(!gw.set_waiting(OTHER, b"x"), "an unknown node");
    assert!(!gw.set_waiting(NODE, &[0; 75]), "longer than a frame");
    world.borrow_mut().responder = Some(acknowledging_node());
    world.borrow_mut().arrive(1_000, uplink(3, b"report"));
    let up = block_on(gw.recv(5_000)).expect("taken in");
    // The acknowledgement says that something waits.
    assert_eq!(acks(&world), vec![(1_020, 3, 1)]);
    // 20 ms after the uplink handled, the downlink, confirmed; the node answers it.
    let result = block_on(gw.deliver_waiting(up.src));
    assert_eq!(result, Some(SendResult::Delivered));
    assert_eq!(gw.last_ack().map(|a| a.rssi), Some(-60));
    let sent = world.borrow().sent.clone();
    let (at, downlink) = &sent[1];
    assert_eq!(*at, 1_040);
    let (h, p) = open(&KEY, downlink);
    assert_eq!(
        (h.frame_type, h.flags, h.dest, h.counter),
        (FrameType::Data, flags::CONFIRMED, NODE, 2)
    );
    assert_eq!(p, b"set 21.5");
    // Delivered: nothing waits, and the next acknowledgement has no flag.
    assert!(!gw.has_waiting(NODE));
    assert_eq!(block_on(gw.deliver_waiting(NODE)), None);
    world.borrow_mut().arrive(3_000, uplink(4, b"report"));
    assert!(block_on(gw.recv(5_000)).is_some());
    assert_eq!(acks(&world).last(), Some(&(3_020, 4, 0)));
}

#[test]
fn a_downlink_not_heard_stays() {
    let (mut gw, world, _) = gateway();
    assert!(gw.set_waiting(NODE, b"set"));
    // Nobody answers: 2 tries, each waiting 200 ms, then it stays for the next uplink.
    assert_eq!(block_on(gw.deliver_waiting(NODE)), Some(SendResult::NotDelivered));
    assert!(gw.has_waiting(NODE));
    let sent = world.borrow().sent.clone();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].1, sent[1].1, "the same bytes again");
    let gap = sent[1].0 - sent[0].0;
    assert!(
        (200..300).contains(&gap),
        "the window and a pause below 100 ms: {gap}"
    );
    assert!(gw.clear_waiting(NODE));
    assert!(!gw.clear_waiting(NODE));
}

#[test]
fn sending_to_a_node_that_listens() {
    let (mut gw, world, _) = gateway();
    // Unknown, too long: not tried.
    assert_eq!(
        block_on(gw.send(OTHER, b"x", true, 3)),
        SendResult::Refused(Refusal::NotRegistered)
    );
    assert_eq!(
        block_on(gw.send(NODE, &[0; 75], true, 3)),
        SendResult::Refused(Refusal::TooLong)
    );
    // Not confirmed: once.
    assert_eq!(block_on(gw.send(NODE, b"time", false, 5)), SendResult::Delivered);
    assert_eq!(world.borrow().sent.len(), 1);
    assert_eq!(gw.last_ack(), None);
    // Confirmed, nobody answers: the tries asked (at most MAX_REPS), one counter for all.
    assert_eq!(block_on(gw.send(NODE, b"set", true, 3)), SendResult::NotDelivered);
    assert_eq!(
        block_on(gw.send(NODE, b"set", true, 50)),
        SendResult::NotDelivered
    );
    let counters: Vec<u32> = world
        .borrow()
        .sent
        .iter()
        .map(|(_, f)| open(&KEY, f).0.counter)
        .collect();
    let mut want = vec![1, 2, 2, 2];
    want.extend(vec![3; MAX_REPS as usize]);
    assert_eq!(counters, want);
    // An acknowledgement for another counter, or from another node, does not end the wait.
    world.borrow_mut().responder = Some(Box::new(|now, sent| {
        let (h, _) = open(&KEY, sent);
        let ack = |counter: u32, src: u32| {
            let mut p = [0u8; 6];
            p[..4].copy_from_slice(&counter.to_le_bytes());
            frame(&KEY, FrameType::Ack, src, GATEWAY, 2000 + counter, 0, &p)
        };
        vec![
            (now + 10, ack(h.counter - 1, NODE)),
            (now + 20, ack(h.counter, OTHER)),
            (now + 30, ack(h.counter, NODE)),
        ]
    }));
    assert_eq!(block_on(gw.send(NODE, b"set", true, 3)), SendResult::Delivered);
}

#[test]
fn the_counter_and_its_watermark() {
    let world = Shared::default();
    let kept = Kept::default();
    let mut gw = Gateway::new(
        AirRadio(world.clone()),
        AirClock(world.clone()),
        SoftAes,
        kept.clone(),
        GATEWAY,
    );
    // The first start: from 1, a block of 1024 reserved; nothing sent before the agent's floor.
    assert_eq!(*kept.value.borrow(), Some(1025));
    assert_eq!(gw.tx_counter(), 1);
    assert!(gw.add_peer(NODE, &KEY, 0));
    assert!(!gw.can_send());
    assert_eq!(
        block_on(gw.send(NODE, b"x", false, 1)),
        SendResult::Refused(Refusal::Locked)
    );
    // A confirmed uplink is still taken in, but not answered.
    world.borrow_mut().arrive(0, uplink(1, b"up"));
    assert!(block_on(gw.recv(1_000)).is_some());
    assert!(world.borrow().sent.is_empty());
    // The agent's floor: above the watermark, so a new one is kept.
    assert!(gw.raise_tx_floor(5_000));
    assert!(gw.can_send());
    assert_eq!(gw.tx_counter(), 5_000);
    assert_eq!(*kept.value.borrow(), Some(6_024));
    assert_eq!(gw.tx_floor(), 6_024);
    assert_eq!(block_on(gw.send(NODE, b"x", false, 1)), SendResult::Delivered);
    assert_eq!(open(&KEY, &world.borrow().sent[0].1).0.counter, 5_000);

    // After a reset the counter resumes at the kept watermark.
    let gw = Gateway::new(
        AirRadio(world.clone()),
        AirClock(world.clone()),
        SoftAes,
        kept.clone(),
        GATEWAY,
    );
    assert_eq!(gw.tx_counter(), 6_024);
    assert_eq!(*kept.value.borrow(), Some(7_048));

    // A watermark that cannot be kept: nothing is ever sent.
    let broken = Kept {
        fails: true,
        ..Kept::default()
    };
    let mut gw = Gateway::new(
        AirRadio(world.clone()),
        AirClock(world.clone()),
        SoftAes,
        broken,
        GATEWAY,
    );
    assert!(gw.add_peer(NODE, &KEY, 0));
    gw.raise_tx_floor(0);
    assert!(!gw.can_send());
    assert_eq!(
        block_on(gw.send(NODE, b"x", false, 1)),
        SendResult::Refused(Refusal::Locked)
    );
}

#[test]
fn the_duty_cycle() {
    let (mut gw, world, _) = gateway();
    // EU 868: 36 s of air, refilled at 1 %. A frame of 3 bytes takes 25 bytes on air with the
    // header and the tag, 15 ms with the packet around it.
    let mut sent = 0;
    while block_on(gw.send(NODE, b"abc", false, 1)) == SendResult::Delivered {
        sent += 1;
    }
    assert_eq!(sent, 36_000 / 15);
    assert_eq!(block_on(gw.send(NODE, b"abc", false, 1)), SendResult::DutyLimited);
    // 15 s later: 150 ms earned, ten frames.
    let now = world.borrow().now;
    world.borrow_mut().now = now + 15_000;
    let mut more = 0;
    while block_on(gw.send(NODE, b"abc", false, 1)) == SendResult::Delivered {
        more += 1;
    }
    assert_eq!(more, 10);
}

/// The frames a real Core Module sent (`net-confirmed-2026-11-19.txt`), through the gateway:
/// each message taken in once, each of its copies answered.
#[test]
fn a_real_node_s_frames() {
    let frames: Vec<Vec<u8>> = include_str!("net-confirmed-2026-11-19.txt")
        .lines()
        .filter_map(|line| {
            let hex = line.strip_prefix("rx ")?.split(": ").nth(1)?.split(" (").next()?;
            hex.split(' ').map(|b| u8::from_str_radix(b, 16).ok()).collect()
        })
        .collect();
    assert!(frames.len() >= 30);
    let (mut gw, world, _) = gateway();
    // The counters of that run start at 0xB801: the agent would know the last one before.
    assert!(gw.add_peer(NODE, &KEY, 0xB800));
    for (i, f) in frames.iter().enumerate() {
        world.borrow_mut().arrive(700 * i as u64, f.clone());
    }
    let mut taken = Vec::new();
    while let Some(up) = block_on(gw.recv(700 * frames.len() as u64 + 1_000)) {
        taken.push((up.counter, up.data().to_vec()));
    }
    // Every message once, in order, `msg NNN\0`.
    let counters: Vec<u32> = taken.iter().map(|t| t.0).collect();
    assert!(counters.windows(2).all(|w| w[1] == w[0] + 1), "{counters:?}");
    assert!(taken.iter().all(|t| t.1.starts_with(b"msg ") && t.1.len() == 8));
    // Every frame answered: the first copy, and each copy after it.
    assert_eq!(acks(&world).len(), frames.len());
}
