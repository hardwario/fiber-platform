//! Pairing a node (`fiber_tower_air::gateway::pairing`, T47) against a joining node made up from
//! the same `tower-net` kernels a TOWER node runs (`tower_net::handshake`), on the made-up air of
//! `common`: the whole handshake, a wrong confirmation, a lost nonce, who may join, the window,
//! and the keyed join.

// Tests may unwrap (`clippy.toml`), but clippy sees that only inside `#[test]` functions, and this
// file has helpers too.
#![allow(clippy::unwrap_used)]

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::*;
use fiber_tower_air::frame::{FrameType, SoftAes, flags};
use fiber_tower_air::gateway::{
    Entropy, Gateway, HOST_HANDSHAKE_TIME_MS, JOIN_FRAME_KEY, JoinRefusal, PAIRING_WINDOW_MS,
};
use tower_net::handshake::{COMMIT_LEN, Ephemeral, NONCE_LEN, PUBLIC_LEN, Role, VERSION, commitment_matches};

/// A node that is not paired yet: it chose this address.
const JOINER: u32 = 0x4444_4444;

/// Random bytes, made up and different every time; `broken` gives none.
struct Dice {
    state: u8,
    broken: bool,
}

impl Entropy for Dice {
    fn fill(&mut self, out: &mut [u8]) -> bool {
        for b in out.iter_mut() {
            self.state = self.state.wrapping_mul(29).wrapping_add(11);
            *b = self.state;
        }
        !self.broken
    }
}

fn dice() -> Dice {
    Dice {
        state: 1,
        broken: false,
    }
}

/// What the joining node ends with: its link key and its code.
type Outcome = Rc<RefCell<Option<([u8; 16], u32)>>>;

/// The joining node's side, as TOWER's `Net::join` runs it, from the kernels: it answers the
/// gateway's `JoinResp` with its nonce 30 ms later, checks the `JoinReveal` against the
/// commitment and the gateway's tag, and confirms 1.2 s later (its shared secret on a Core
/// Module). `lose_nonce` never sends the nonce; `bad_tag` confirms with a wrong tag.
struct Joiner {
    addr: u32,
    eph: Ephemeral,
    gw_public: Option<[u8; PUBLIC_LEN]>,
    commit: Option<[u8; COMMIT_LEN]>,
    counter: u32,
    lose_nonce: bool,
    bad_tag: bool,
    outcome: Outcome,
}

impl Joiner {
    fn new(addr: u32, seed: u8) -> Self {
        Joiner {
            addr,
            eph: Ephemeral::from_parts(&[seed; 32], &[seed.wrapping_add(1); NONCE_LEN]),
            gw_public: None,
            commit: None,
            counter: 100,
            lose_nonce: false,
            bad_tag: false,
            outcome: Outcome::default(),
        }
    }

    /// Its request: `3 | E_node` to address 0, under the framing key.
    fn request(&mut self) -> Vec<u8> {
        self.counter += 1;
        let mut payload = vec![VERSION];
        payload.extend_from_slice(self.eph.public());
        frame(
            &JOIN_FRAME_KEY,
            FrameType::JoinReq,
            self.addr,
            0,
            self.counter,
            0,
            &payload,
        )
    }

    fn respond(&mut self, now: u64, sent: &[u8]) -> Vec<(u64, Vec<u8>)> {
        let mut buf = sent.to_vec();
        let Ok((h, range)) = fiber_tower_air::frame::open_frame(
            &mut fiber_tower_air::frame::Ccm::new(SoftAes),
            &JOIN_FRAME_KEY,
            &mut buf,
        ) else {
            return vec![]; // not a frame under the framing key
        };
        let p = buf[range].to_vec();
        if h.dest != self.addr || h.src != GATEWAY {
            return vec![];
        }
        match h.frame_type {
            FrameType::JoinResp => {
                self.gw_public = Some(p[1..1 + PUBLIC_LEN].try_into().unwrap());
                self.commit = Some(p[1 + PUBLIC_LEN..].try_into().unwrap());
                if self.lose_nonce {
                    return vec![];
                }
                self.counter += 1;
                let mut nonce = vec![VERSION];
                nonce.extend_from_slice(self.eph.nonce());
                vec![(
                    now + 30,
                    frame(
                        &JOIN_FRAME_KEY,
                        FrameType::JoinNonce,
                        self.addr,
                        GATEWAY,
                        self.counter,
                        0,
                        &nonce,
                    ),
                )]
            }
            FrameType::JoinReveal => {
                let n_gw: [u8; NONCE_LEN] = p[1..1 + NONCE_LEN].try_into().unwrap();
                let (e_gw, commit) = (self.gw_public.unwrap(), self.commit.unwrap());
                // The node checks the revealed nonce against the commitment, then the tag.
                assert!(commitment_matches(
                    &commit,
                    &n_gw,
                    self.eph.public(),
                    &e_gw,
                    self.addr,
                    GATEWAY
                ));
                let session = self
                    .eph
                    .session(&e_gw, &n_gw, Role::Node, self.addr, GATEWAY)
                    .unwrap();
                assert!(
                    session.verify(Role::Gateway, &p[1 + NONCE_LEN..]),
                    "the gateway's tag"
                );
                *self.outcome.borrow_mut() = Some((*session.link_key(), session.sas()));
                let mut tag = *session.tag(Role::Node);
                if self.bad_tag {
                    tag[0] ^= 1;
                }
                self.counter += 1;
                let mut confirm = vec![VERSION];
                confirm.extend_from_slice(&tag);
                vec![(
                    now + 1_200,
                    frame(
                        session.link_key(),
                        FrameType::JoinConfirm,
                        self.addr,
                        GATEWAY,
                        self.counter,
                        0,
                        &confirm,
                    ),
                )]
            }
            _ => vec![],
        }
    }
}

/// The joiner answers what the gateway sends; it is also returned, to send its requests.
fn join(world: &Shared, joiner: Joiner) -> Rc<RefCell<Joiner>> {
    let joiner = Rc::new(RefCell::new(joiner));
    let j = joiner.clone();
    world.borrow_mut().responder = Some(Box::new(move |now, sent| j.borrow_mut().respond(now, sent)));
    joiner
}

/// The types and destinations of what the gateway sent, and when.
fn sent_types(world: &Shared) -> Vec<(u64, FrameType)> {
    world
        .borrow()
        .sent
        .iter()
        .map(|(at, f)| {
            (
                *at,
                fiber_tower_air::frame::Header::parse(f).unwrap().0.frame_type,
            )
        })
        .collect()
}

#[test]
fn a_node_pairs() {
    let (mut gw, world, _) = gateway();
    let mut entropy = dice();
    let mut host = gw.open_window(&mut entropy, PAIRING_WINDOW_MS).unwrap();
    assert_eq!(host.window_end_ms(), PAIRING_WINDOW_MS);
    let joiner = join(&world, Joiner::new(JOINER, 7));
    let request = joiner.borrow_mut().request();
    world.borrow_mut().arrive(1_000, request);

    let peer = block_on(gw.pair(&mut host, &mut entropy, 20_000)).expect("paired");
    let (key, sas) = joiner.borrow().outcome.borrow().unwrap();
    assert_eq!(peer.addr, JOINER, "the node's own address");
    assert_eq!(peer.key, key, "both ends hold the same link key");
    assert_eq!(peer.sas, sas, "and show the same code");
    assert!(sas < 1_000_000);
    // The answers, each 20 ms after the frame before: the response, then the reveal after the
    // node's nonce (30 ms after the response); the confirmation came 1.2 s later.
    assert_eq!(
        sent_types(&world),
        vec![(1_020, FrameType::JoinResp), (1_070, FrameType::JoinReveal)]
    );
    assert_eq!(world.borrow().now, 2_270, "it returns with the confirmation");

    // The agent adds it under the new key, from counter 0: its uplinks are taken in.
    assert!(gw.add_peer(peer.addr, &peer.key, 0));
    let up = frame(
        &peer.key,
        FrameType::Data,
        JOINER,
        GATEWAY,
        1,
        flags::CONFIRMED,
        b"info",
    );
    world.borrow_mut().arrive(3_000, up);
    assert_eq!(block_on(gw.recv(4_000)).expect("taken in").data(), b"info");
}

#[test]
fn a_wrong_confirmation_pairs_nobody() {
    let (mut gw, world, _) = gateway();
    let mut entropy = dice();
    let mut host = gw.open_window(&mut entropy, PAIRING_WINDOW_MS).unwrap();
    let mut wrong = Joiner::new(JOINER, 7);
    wrong.bad_tag = true;
    let joiner = join(&world, wrong);
    let request = joiner.borrow_mut().request();
    world.borrow_mut().arrive(1_000, request);
    assert!(block_on(gw.pair(&mut host, &mut entropy, 10_000)).is_none());
    // The gateway waited its whole wait for a right one, and listened on to the deadline.
    assert_eq!(world.borrow().now, 10_000);
}

#[test]
fn a_lost_nonce_spends_the_key_pair() {
    let (mut gw, world, _) = gateway();
    let mut entropy = dice();
    let mut host = gw.open_window(&mut entropy, PAIRING_WINDOW_MS).unwrap();
    let mut lossy = Joiner::new(JOINER, 7);
    lossy.lose_nonce = true;
    let joiner = join(&world, lossy);
    let first = joiner.borrow_mut().request();
    world.borrow_mut().arrive(1_000, first);
    assert!(block_on(gw.pair(&mut host, &mut entropy, 3_000)).is_none());
    let first_public = joiner.borrow().gw_public.unwrap();
    // The node tries again and its nonce arrives this time: answered with a new key pair, and
    // paired.
    joiner.borrow_mut().lose_nonce = false;
    let again = joiner.borrow_mut().request();
    world.borrow_mut().arrive(4_000, again);
    let peer = block_on(gw.pair(&mut host, &mut entropy, 20_000)).expect("paired");
    assert_ne!(
        joiner.borrow().gw_public.unwrap(),
        first_public,
        "a fresh key pair"
    );
    assert_eq!(peer.key, joiner.borrow().outcome.borrow().unwrap().0);
}

#[test]
fn only_new_nodes_join() {
    let (mut gw, world, _) = gateway();
    let mut entropy = dice();
    let mut host = gw.open_window(&mut entropy, PAIRING_WINDOW_MS).unwrap();
    // A node the gateway knows, the broadcast address, the gateway's own address, another
    // version of the handshake: none is answered, and the agent learns why.
    for (at, addr, refusal) in [
        (1_000, NODE, JoinRefusal::AlreadyRegistered),
        (2_000, u32::MAX, JoinRefusal::InvalidAddress),
        (3_000, GATEWAY, JoinRefusal::InvalidAddress),
    ] {
        let request = Joiner::new(addr, 3).request();
        world.borrow_mut().arrive(at, request);
        assert!(block_on(gw.pair(&mut host, &mut entropy, at + 500)).is_none());
        assert_eq!(gw.take_join_refusal(), Some((addr, refusal)));
    }
    let old = frame(&JOIN_FRAME_KEY, FrameType::JoinReq, JOINER, 0, 1, 0, &[2; 33]);
    world.borrow_mut().arrive(4_000, old);
    assert!(block_on(gw.pair(&mut host, &mut entropy, 4_500)).is_none());
    assert_eq!(gw.take_join_refusal(), Some((JOINER, JoinRefusal::NotV3)));
    assert!(world.borrow().sent.is_empty(), "nothing answered");

    // Pairing a known node again: the agent removes it first. Then it pairs under a new key, and
    // its old key opens nothing.
    assert!(gw.remove_peer(NODE));
    let joiner = join(&world, Joiner::new(NODE, 9));
    let request = joiner.borrow_mut().request();
    world.borrow_mut().arrive(5_000, request);
    let peer = block_on(gw.pair(&mut host, &mut entropy, 20_000)).expect("paired again");
    assert!(gw.add_peer(peer.addr, &peer.key, 0));
    assert_ne!(peer.key, KEY);
    let old_key = frame(
        &KEY,
        FrameType::Data,
        NODE,
        GATEWAY,
        500,
        flags::CONFIRMED,
        b"old",
    );
    let new_key = frame(
        &peer.key,
        FrameType::Data,
        NODE,
        GATEWAY,
        1,
        flags::CONFIRMED,
        b"new",
    );
    world.borrow_mut().arrive(30_000, old_key);
    world.borrow_mut().arrive(30_100, new_key);
    assert_eq!(block_on(gw.recv(31_000)).expect("taken in").data(), b"new");
}

#[test]
fn the_window() {
    let (mut gw, world, _) = gateway();
    let mut entropy = dice();
    let mut host = gw.open_window(&mut entropy, 10_000).unwrap();
    // Too late for a whole handshake before the window ends: not answered.
    let late = 10_000 - HOST_HANDSHAKE_TIME_MS + 1;
    assert_eq!(HOST_HANDSHAKE_TIME_MS, 3_880);
    let request = Joiner::new(JOINER, 5).request();
    world.borrow_mut().arrive(late, request);
    // Past the window's end the deadline is the window's.
    assert!(block_on(gw.pair(&mut host, &mut entropy, 60_000)).is_none());
    assert_eq!(gw.take_join_refusal(), Some((JOINER, JoinRefusal::WindowClosing)));
    assert_eq!(world.borrow().now, 10_000);
    assert!(world.borrow().sent.is_empty());

    // Without random numbers there is no window.
    let mut broken = Dice {
        state: 1,
        broken: true,
    };
    assert!(gw.open_window(&mut broken, 10_000).is_none());
}

#[test]
fn nothing_answered_before_the_floor() {
    let world = Shared::default();
    let mut gw = Gateway::new(
        AirRadio(world.clone()),
        AirClock(world.clone()),
        SoftAes,
        Kept::default(),
        GATEWAY,
    );
    let mut entropy = dice();
    let mut host = gw.open_window(&mut entropy, PAIRING_WINDOW_MS).unwrap();
    let request = Joiner::new(JOINER, 5).request();
    world.borrow_mut().arrive(100, request);
    assert!(block_on(gw.pair(&mut host, &mut entropy, 5_000)).is_none());
    assert!(world.borrow().sent.is_empty());
    assert_eq!(world.borrow().now, 5_000);
}

#[test]
fn a_keyed_join() {
    let (mut gw, world, _) = gateway();
    let keyed = |counter: u32, addr: u32| {
        frame(
            &KEY,
            FrameType::JoinReq,
            NODE,
            0,
            counter,
            flags::KEYED,
            &addr.to_le_bytes(),
        )
    };
    // Switched off until the agent says.
    world.borrow_mut().arrive(0, keyed(5, NODE));
    assert!(block_on(gw.recv(1_000)).is_none());
    assert_eq!(gw.take_join_refusal(), Some((NODE, JoinRefusal::KeyedOff)));
    assert!(world.borrow().sent.is_empty());

    // On: answered 20 ms later under the node's key, with the request's counter back; never an
    // uplink.
    gw.set_keyed_join(true);
    world.borrow_mut().arrive(2_000, keyed(6, NODE));
    assert!(block_on(gw.recv(3_000)).is_none());
    assert_eq!(gw.take_keyed_join(), Some(NODE));
    let sent = world.borrow().sent.clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, 2_020);
    let (h, p) = open(&KEY, &sent[0].1);
    assert_eq!(
        (h.frame_type, h.flags, h.src, h.dest),
        (FrameType::JoinResp, flags::KEYED, GATEWAY, NODE)
    );
    assert_eq!(p, 6u32.to_le_bytes());
    assert_eq!(gw.peer_last_seen(NODE), Some(6));

    // The same request again (recorded): no second answer.
    world.borrow_mut().arrive(4_000, keyed(6, NODE));
    assert!(block_on(gw.recv(5_000)).is_none());
    assert_eq!(
        gw.take_join_refusal(),
        Some((
            NODE,
            JoinRefusal::KeyedStale {
                counter: 6,
                last_seen: 6
            }
        ))
    );
    // Another address in the payload; a node the gateway has no key for.
    world.borrow_mut().arrive(6_000, keyed(7, OTHER));
    assert!(block_on(gw.recv(7_000)).is_none());
    assert_eq!(
        gw.take_join_refusal(),
        Some((NODE, JoinRefusal::KeyedNotAuthentic))
    );
    let stranger = frame(
        &KEY,
        FrameType::JoinReq,
        OTHER,
        0,
        1,
        flags::KEYED,
        &OTHER.to_le_bytes(),
    );
    world.borrow_mut().arrive(8_000, stranger);
    assert!(block_on(gw.recv(9_000)).is_none());
    assert_eq!(gw.take_join_refusal(), Some((OTHER, JoinRefusal::KeyedUnknown)));
    assert_eq!(world.borrow().sent.len(), 1);
}
