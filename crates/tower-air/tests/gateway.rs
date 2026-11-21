//! The gateway's flow (`fiber_tower_air::gateway`, T24) against a radio and a clock made up in
//! software, sharing one time that moves only when the gateway waits: so the turnarounds and the
//! acknowledgement windows run in exact milliseconds. The nodes' frames are sealed by the same
//! codec, as a TOWER node seals them; at the end, the frames a real Core Module sent.

// Tests may unwrap (`clippy.toml`), but clippy sees that only inside `#[test]` functions, and this
// file has helpers too.
#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use fiber_tower_air::frame::{FrameType, SoftAes, flags};
use fiber_tower_air::gateway::{Frames, Gateway, MAX_REPS, Refusal, SendResult};

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
fn a_frame_across_the_deadline_is_received_whole() {
    // Begun before a receive's deadline and ended after it: taken in. Cut off at the deadline it
    // would be lost, and the node tries again 200 ms later at the earliest (on a FIBER, one click
    // of 20 needed a second try while the receive was cut off).
    let (mut gw, world, _) = gateway();
    let first = uplink(5, b"hello");
    let toa = airtime(&first);
    assert!(toa > 10, "on air for {toa} ms");
    world.borrow_mut().arrive(100 + toa / 2, first);
    assert_eq!(block_on(gw.recv(100)).expect("taken in whole").counter, 5);
    assert_eq!(acks(&world).len(), 1);
    // One that begins after the deadline is the next receive's.
    let deadline = world.borrow().now + 50;
    let next = uplink(6, b"later");
    let at = deadline + 10 + airtime(&next);
    world.borrow_mut().arrive(at, next);
    assert!(block_on(gw.recv(deadline)).is_none());
    assert_eq!(world.borrow().now, deadline);
    assert_eq!(block_on(gw.recv(deadline + 1_000)).expect("taken in").counter, 6);
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
    // Counted as repeated, every copy, answered or not.
    let f = gw.frames();
    assert_eq!((f.taken_in, f.repeated), (1, MAX_REPS as u32 + 2));
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
    let f = gw.frames();
    assert_eq!((f.taken_in, f.replayed, f.repeated), (2, 2, 0));
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
    // Each by why: the unknown node; the other key and the damaged tag; the other gateway's and
    // the acknowledgement.
    assert_eq!(
        gw.frames(),
        Frames {
            unknown_node: 1,
            not_authentic: 2,
            not_for_us: 2,
            ..Frames::default()
        }
    );
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
    // Counted: each message taken in once, each copy after it as repeated.
    assert_eq!(
        gw.frames(),
        Frames {
            taken_in: taken.len() as u32,
            repeated: (frames.len() - taken.len()) as u32,
            ..Frames::default()
        }
    );
    assert_eq!(
        gw.peers().collect::<Vec<_>>(),
        vec![(NODE, *counters.last().unwrap(), false)]
    );
    assert!(gw.duty_left_ms() < 36_000);
}
