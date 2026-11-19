//! The gateway's flow (`docs/firmware/radio.md`, "The gateway's flow"): what TOWER's `Net`
//! (`firmware/src/radio/net/mod.rs`) and the delivery part of its gateway application
//! (`firmware/apps/radio_gateway.rs`) do, at the pinned revision, written again over a
//! [`Radio`] and a [`Clock`] (T24; T27 watches the two for drift).
//!
//! Every accept or refuse is a `tower-net` kernel: the replay rule ([`ReplayLane`]), the
//! transmit counter and its reserve ([`TxCounter`]), which frame answers a confirmed one
//! ([`AckWait`]). The duty cycle is `tower-radio`'s ([`DutyGovernor`]). What is ours: the radio,
//! the clock, the watermark's storage, and that the agent, not the firmware, keeps the nodes
//! (D8): it adds each with its key and the last counter it saw from it.
//!
//! One transfer at a time, as in TOWER: the north bridge's firmware owns one [`Gateway`] in one
//! loop. Pairing a node is [`Gateway::open_window`] and [`Gateway::pair`] (T47), its keyed join
//! part of [`Gateway::recv`]; frequency hopping and bulk transfer are not here.

mod pairing;

pub use pairing::{
    Entropy, HOST_HANDSHAKE_TIME_MS, JOIN_FRAME_KEY, JoinRefusal, PAIRING_WINDOW_MS, PairedPeer, PairingHost,
};

use core::future::Future;
use core::pin::pin;
use core::task::Poll;

pub use tower_net::ack::AckMeta;
use tower_net::ack::{ACK_LEN, AckVerdict, AckWait, ack_flags, ack_meta};
use tower_net::addr::peer_addr_valid;
pub use tower_net::replay::MAX_REPS;
use tower_net::replay::{ReplayLane, ReplayVerdict};
use tower_net::txctr::TxCounter;
use tower_radio::{DutyGovernor, frame_toa_ms};

use crate::frame::{self, AesBlock, Ccm, FrameType, HDR_LEN, Header, MAX_FRAME, MAX_PAYLOAD, TAG_LEN, flags};
use crate::radio::Radio;

/// How long a confirmed frame waits for its acknowledgement (TOWER's `ACK_WINDOW`; the round
/// trip it measured is about 35 ms).
pub const ACK_WINDOW_MS: u64 = 200;
/// How long the gateway waits before it answers, so that the node has turned from sending to
/// receiving (TOWER's `ACK_TURNAROUND`).
pub const ACK_TURNAROUND_MS: u64 = 20;
/// The same before a waiting downlink, after the node's uplink (TOWER's `DOWNLINK_TURNAROUND`).
pub const DOWNLINK_TURNAROUND_MS: u64 = 20;
/// The tries of a waiting downlink: few, as the node listens for about a second after its uplink
/// (TOWER's `DOWNLINK_REPS`).
pub const DOWNLINK_REPS: u8 = 2;
/// The longest pause before a confirmed frame goes again, chosen at random below it (TOWER's
/// `MAX_BACKOFF_MS`), so that two senders that collided do not collide again.
pub const MAX_BACKOFF_MS: u32 = 100;
/// The nodes a gateway knows at once (TOWER's `MAX_PEERS`: a star of up to 16).
pub const MAX_PEERS: usize = 16;

/// The time, and waiting for a moment: the north bridge's timer in the firmware, a made-up clock
/// in the tests. A receive with a deadline is a race of the radio and [`Clock::sleep_until`].
#[allow(async_fn_in_trait)]
pub trait Clock {
    /// Milliseconds since some start; never goes back.
    fn now_ms(&self) -> u64;
    /// Returns at `at_ms`, or at once when it has passed.
    async fn sleep_until(&self, at_ms: u64);
}

/// Where the transmit counter's watermark is kept: the north bridge's own flash (T25).
pub trait Watermark {
    /// The watermark kept, if there is one.
    fn load(&mut self) -> Option<u32>;
    /// Keeps `watermark`, and reads it back: whether it is there now. On `false` the gateway
    /// sends nothing more (the counter could repeat after a reset).
    fn store(&mut self, watermark: u32) -> bool;
}

/// How a send ended. A result other than [`SendResult::Delivered`] is a delivery that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum SendResult {
    /// Acknowledged (or, not confirmed, sent).
    Delivered,
    /// Confirmed, and no acknowledgement after every try.
    NotDelivered,
    /// The duty cycle's budget had no room for it.
    DutyLimited,
    /// Not sent at all.
    Refused(Refusal),
}

/// Why a send was not tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Longer than a frame's payload (74 bytes).
    TooLong,
    /// A node the gateway has no key for.
    NotRegistered,
    /// The transmit counter is locked: its watermark could not be kept, or the agent's floor has
    /// not come yet.
    Locked,
    /// The radio failed.
    Radio,
}

/// A frame a known node sent, taken in: authenticated, new, its payload decrypted.
#[derive(Clone, Copy, Debug)]
pub struct Uplink {
    pub src: u32,
    pub counter: u32,
    /// The signal strength the radio measured, in dBm.
    pub rssi: i16,
    /// The node asked for an acknowledgement (and got one).
    pub confirmed: bool,
    len: usize,
    buf: [u8; MAX_PAYLOAD],
}

impl Uplink {
    /// The payload: one of TOWER's application messages, for the agent to read.
    pub fn data(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

/// What one received frame came to.
enum Heard {
    Uplink(Uplink),
    /// A frame not taken in.
    PassedOver,
    /// The deadline, or the radio failed.
    Nothing,
}

/// A downlink that waits for a node's next uplink.
#[derive(Clone, Copy)]
struct Waiting {
    len: usize,
    data: [u8; MAX_PAYLOAD],
}

/// A node the agent gave: its address, its key, its replay lane, and what waits for it.
#[derive(Clone, Copy)]
struct Peer {
    addr: u32,
    key: [u8; 16],
    lane: ReplayLane,
    waiting: Option<Waiting>,
}

/// The gateway over one radio.
pub struct Gateway<R: Radio, C: Clock, A: AesBlock, W: Watermark> {
    radio: R,
    clock: C,
    ccm: Ccm<A>,
    watermark: W,
    addr: u32,
    peers: [Option<Peer>; MAX_PEERS],
    txc: TxCounter,
    /// The agent has given its floor for the counter ([`Gateway::raise_tx_floor`]): until then
    /// nothing is sent, as the counter could be below one already used under a node's key.
    floor_known: bool,
    duty: DutyGovernor,
    duty_at: u64,
    rng: u32,
    last_ack: Option<AckMeta>,
    /// Keyed joins are answered ([`Gateway::set_keyed_join`]).
    keyed_join: bool,
    /// The node whose keyed join was answered last, until the agent takes it.
    keyed_joined: Option<u32>,
    /// The last request to join that was not answered, until the agent takes it.
    join_refusal: Option<(u32, JoinRefusal)>,
}

/// The output of `a`, or `None` when `b` ends first.
async fn first<A: Future, B: Future>(a: A, b: B) -> Option<A::Output> {
    let mut a = pin!(a);
    let mut b = pin!(b);
    core::future::poll_fn(|cx| {
        if let Poll::Ready(out) = a.as_mut().poll(cx) {
            return Poll::Ready(Some(out));
        }
        if b.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        Poll::Pending
    })
    .await
}

impl<R: Radio, C: Clock, A: AesBlock, W: Watermark> Gateway<R, C, A, W> {
    /// The gateway at `addr`. The counter resumes at the kept watermark (1 the first time) and
    /// reserves the next block; it stays locked until the agent gives its floor
    /// ([`raise_tx_floor`](Self::raise_tx_floor)).
    pub fn new(radio: R, clock: C, aes: A, mut watermark: W, addr: u32) -> Self {
        // No other record is kept here, so a lost watermark looks like a first start; the
        // agent's floor (it keeps the counter the firmware reports) covers that case.
        let (mut txc, reserve) = TxCounter::resume(watermark.load(), false);
        if let Some(reserve) = reserve
            && !watermark.store(reserve)
        {
            txc.lock();
        }
        let duty_at = clock.now_ms();
        Self {
            radio,
            clock,
            ccm: Ccm::new(aes),
            watermark,
            addr,
            peers: [None; MAX_PEERS],
            txc,
            floor_known: false,
            duty: DutyGovernor::eu(),
            duty_at,
            rng: addr | 1,
            last_ack: None,
            keyed_join: false,
            keyed_joined: None,
            join_refusal: None,
        }
    }

    /// The gateway's address.
    pub fn addr(&self) -> u32 {
        self.addr
    }

    /// The next counter the gateway will send under.
    pub fn tx_counter(&self) -> u32 {
        self.txc.counter()
    }

    /// Every counter sent so far is below this (the kept watermark): what the agent keeps and
    /// gives back with [`raise_tx_floor`](Self::raise_tx_floor) after a reset.
    pub fn tx_floor(&self) -> u32 {
        self.txc.floor()
    }

    /// Whether the gateway may send: the agent's floor came, and the watermark could be kept.
    pub fn can_send(&self) -> bool {
        self.floor_known && !self.txc.locked()
    }

    /// Raises the counter to at least `floor` (the agent's: the [`tx_floor`](Self::tx_floor) the
    /// firmware last reported, 0 when it has none), with the watermark kept and read back. Sending
    /// starts after the first call that succeeds. `false`: the watermark could not be kept, and
    /// nothing is sent.
    pub fn raise_tx_floor(&mut self, floor: u32) -> bool {
        let watermark = &mut self.watermark;
        let raised = self.txc.raise_to(floor, |w| watermark.store(w));
        self.floor_known |= raised;
        raised
    }

    /// Selects the channel, 0 to 2 in EU 868 (868.1, 868.3, 868.5 MHz); the nodes use the same.
    pub fn set_channel(&mut self, channel: u8) -> Result<(), R::Error> {
        self.radio.set_channel(channel)
    }

    /// Adds a node, or keys it again: its address, its key, and the last counter seen from it
    /// (0 for a node just paired). With the same key the lane only moves up; with another key it
    /// starts at `last_seen` (a new key is a new nonce space) and what waited for the node is
    /// dropped. `false` when the table is full, or `addr` is reserved or the gateway's own.
    pub fn add_peer(&mut self, addr: u32, key: &[u8; 16], last_seen: u32) -> bool {
        if !peer_addr_valid(addr, self.addr) {
            return false;
        }
        if let Some(peer) = self.peer_mut(addr) {
            if peer.key == *key {
                let _ = peer.lane.raise(last_seen);
            } else {
                peer.key = *key;
                peer.lane = ReplayLane::new(last_seen);
                peer.waiting = None;
            }
            return true;
        }
        let Some(slot) = self.peers.iter_mut().find(|p| p.is_none()) else {
            return false;
        };
        *slot = Some(Peer {
            addr,
            key: *key,
            lane: ReplayLane::new(last_seen),
            waiting: None,
        });
        true
    }

    /// Forgets a node; whether it was known.
    pub fn remove_peer(&mut self, addr: u32) -> bool {
        match self.peers.iter_mut().find(|p| p.is_some_and(|p| p.addr == addr)) {
            Some(slot) => {
                *slot = None;
                true
            }
            None => false,
        }
    }

    /// The last counter accepted from a node.
    pub fn peer_last_seen(&self, addr: u32) -> Option<u32> {
        self.peer(addr).map(|p| p.lane.last_seen())
    }

    /// Puts `data` to wait for the node's next uplink, in place of what waited before; every
    /// acknowledgement to the node says that something waits. `false`: an unknown node, or longer
    /// than a frame's payload.
    pub fn set_waiting(&mut self, addr: u32, data: &[u8]) -> bool {
        if data.len() > MAX_PAYLOAD {
            return false;
        }
        let Some(peer) = self.peer_mut(addr) else {
            return false;
        };
        let mut waiting = Waiting {
            len: data.len(),
            data: [0; MAX_PAYLOAD],
        };
        waiting.data[..data.len()].copy_from_slice(data);
        peer.waiting = Some(waiting);
        true
    }

    /// Drops what waits for a node; whether something did.
    pub fn clear_waiting(&mut self, addr: u32) -> bool {
        self.peer_mut(addr).is_some_and(|p| p.waiting.take().is_some())
    }

    /// Whether something waits for a node.
    pub fn has_waiting(&self, addr: u32) -> bool {
        self.peer(addr).is_some_and(|p| p.waiting.is_some())
    }

    /// The acknowledgement that ended the last confirmed send: the strength the node heard it at,
    /// and whether the node has something waiting for the gateway. `None` after any other send.
    pub fn last_ack(&self) -> Option<AckMeta> {
        self.last_ack
    }

    /// Receives frames until `until_ms` and returns the first new one from a known node, once it
    /// is acknowledged (when it asked). Every other frame is passed over: one for another address
    /// or of another type, from an unknown node, not authentic, a retransmission (acknowledged
    /// again, at most [`MAX_REPS`] - 1 times) or a replay. `None` at `until_ms`, or when the radio
    /// fails. TOWER's `Net::recv`, which takes one frame a call, in a loop.
    pub async fn recv(&mut self, until_ms: u64) -> Option<Uplink> {
        loop {
            match self.recv_frame(until_ms).await {
                Heard::Uplink(uplink) => return Some(uplink),
                Heard::PassedOver => {}
                Heard::Nothing => return None,
            }
        }
    }

    /// One frame, received before `until_ms`.
    async fn recv_frame(&mut self, until_ms: u64) -> Heard {
        let mut buf = [0u8; MAX_FRAME];
        let Some(Ok(rx)) = first(self.radio.receive(&mut buf), self.clock.sleep_until(until_ms)).await else {
            return Heard::Nothing;
        };
        self.take_in(&mut buf, rx.len, rx.rssi)
            .await
            .map_or(Heard::PassedOver, Heard::Uplink)
    }

    /// What a received frame is: a new uplink (acknowledged when it asked), or nothing to take in.
    async fn take_in(&mut self, buf: &mut [u8], len: usize, rssi: i16) -> Option<Uplink> {
        let frame = buf.get_mut(..len)?;
        // The clear header says who sent it, so whose key opens it.
        let (peek, _) = Header::parse(frame).ok()?;
        if peek.frame_type == FrameType::JoinReq && peek.flags & flags::KEYED != 0 && peek.dest == 0 {
            // A keyed join: answered like an acknowledgement, never an uplink.
            if let Some((node, answer)) = self.keyed_answer(frame) {
                self.sleep_ms(ACK_TURNAROUND_MS).await;
                if self.radio.send(&answer).await.is_ok() {
                    self.keyed_joined = Some(node);
                }
            }
            return None;
        }
        if peek.dest != self.addr || peek.frame_type != FrameType::Data {
            return None;
        }
        // Known nodes only: an unknown sender is dropped before any CCM work, and not answered.
        let key = self.peer(peek.src)?.key;
        let (hdr, range) = frame::open_frame(&mut self.ccm, &key, frame).ok()?;
        if hdr.frame_type != FrameType::Data {
            return None;
        }
        let confirmed = hdr.flags & flags::CONFIRMED != 0;
        let peer = self.peer_mut(hdr.src)?;
        match peer.lane.classify(hdr.counter) {
            ReplayVerdict::Fresh => {
                // The lane is not kept here: the agent keeps each node's last counter (D8), so
                // the kernel's "persist now" is not needed.
                let _ = peer.lane.accept(hdr.counter);
                if confirmed {
                    self.send_ack(hdr.src, hdr.counter, rssi).await;
                }
                let payload = frame.get(range)?;
                let mut out = Uplink {
                    src: hdr.src,
                    counter: hdr.counter,
                    rssi,
                    confirmed,
                    len: payload.len(),
                    buf: [0; MAX_PAYLOAD],
                };
                out.buf[..payload.len()].copy_from_slice(payload);
                Some(out)
            }
            ReplayVerdict::Retransmit => {
                // The node did not hear the acknowledgement: answer again, never deliver again,
                // and at most as often as a node can ask (a copied frame needs no key).
                if confirmed && peer.lane.reack() {
                    self.send_ack(hdr.src, hdr.counter, rssi).await;
                }
                None
            }
            ReplayVerdict::Replay => None,
        }
    }

    /// Sends `data` to a known node. Confirmed, it waits [`ACK_WINDOW_MS`] for the
    /// acknowledgement and sends the same bytes again, up to `reps` times (1 to [`MAX_REPS`]);
    /// not confirmed, once. One counter for the whole transfer. TOWER's `Net::send`.
    pub async fn send(&mut self, dest: u32, data: &[u8], confirmed: bool, reps: u8) -> SendResult {
        self.last_ack = None;
        if data.len() > MAX_PAYLOAD {
            return SendResult::Refused(Refusal::TooLong);
        }
        let Some(key) = self.peer(dest).map(|p| p.key) else {
            return SendResult::Refused(Refusal::NotRegistered);
        };
        // The counter is spent now, before anything is sealed or sent: a transfer dropped
        // half way must never leave it to the next one (one nonce under one key, twice, breaks
        // the CCM). The tries send the same sealed bytes.
        let Some(counter) = self.alloc_counter() else {
            return SendResult::Refused(Refusal::Locked);
        };
        let header = Header {
            frame_type: FrameType::Data,
            flags: if confirmed { flags::CONFIRMED } else { 0 },
            src: self.addr,
            dest,
            counter,
            bulk_index: None,
        };
        let mut out = [0u8; MAX_FRAME];
        let Ok(n) = frame::seal_frame(&mut self.ccm, &key, &header, data, &mut out) else {
            return SendResult::Refused(Refusal::TooLong);
        };
        let sealed = &out[..n];
        let toa = frame_toa_ms(n);
        let reps = if confirmed { reps.clamp(1, MAX_REPS) } else { 1 };
        let mut result = SendResult::NotDelivered;
        for attempt in 0..reps {
            if attempt > 0 {
                let pause = u64::from(self.backoff_ms());
                self.sleep_ms(pause).await;
            }
            if !self.duty_allows(toa) {
                result = SendResult::DutyLimited;
                break;
            }
            if self.radio.send(sealed).await.is_err() {
                result = SendResult::Refused(Refusal::Radio);
                break;
            }
            if !confirmed {
                result = SendResult::Delivered;
                break;
            }
            if self.await_ack(dest, counter, &key).await {
                result = SendResult::Delivered;
                break;
            }
        }
        result
    }

    /// After a node's uplink: what waits for it, sent [`DOWNLINK_TURNAROUND_MS`] later,
    /// confirmed, with [`DOWNLINK_REPS`] tries, and dropped once delivered. `None` when nothing
    /// waits. TOWER's gateway application, `on_uplink` and `deliver`.
    pub async fn deliver_waiting(&mut self, node: u32) -> Option<SendResult> {
        let waiting = self.peer(node)?.waiting?;
        self.sleep_ms(DOWNLINK_TURNAROUND_MS).await;
        let result = self
            .send(node, &waiting.data[..waiting.len], true, DOWNLINK_REPS)
            .await;
        if result == SendResult::Delivered
            && let Some(peer) = self.peer_mut(node)
        {
            peer.waiting = None;
        }
        Some(result)
    }

    /// Waits up to [`ACK_WINDOW_MS`] for `dest` to acknowledge `counter`, passing over every
    /// frame heard that is not that acknowledgement (another node's uplink, a stale
    /// acknowledgement). TOWER's `Net::await_ack`, the decision `tower_net::ack`'s.
    async fn await_ack(&mut self, dest: u32, counter: u32, key: &[u8; 16]) -> bool {
        let mut wait = AckWait::new(self.addr, dest, counter);
        let deadline = self.clock.now_ms().saturating_add(ACK_WINDOW_MS);
        let mut buf = [0u8; MAX_FRAME];
        loop {
            if self.clock.now_ms() >= deadline {
                return false;
            }
            let Some(Ok(rx)) = first(self.radio.receive(&mut buf), self.clock.sleep_until(deadline)).await
            else {
                return false;
            };
            let Some(frame) = buf.get_mut(..rx.len) else {
                continue;
            };
            let Ok((hdr, range)) = frame::open_frame(&mut self.ccm, key, frame) else {
                continue;
            };
            let Some(payload) = frame.get(range) else {
                continue;
            };
            if wait.offer(hdr.frame_type == FrameType::Ack, hdr.src, hdr.dest, payload)
                == AckVerdict::Resolved
            {
                self.last_ack = ack_meta(payload);
                return true;
            }
        }
    }

    /// Answers a confirmed frame: after the turnaround, an `Ack` with the gateway's own next
    /// counter, the counter it answers, the strength it was heard at, and the flag when something
    /// waits for the node. Skipped (the node sends again) when the counter is locked or the duty
    /// cycle has no room, which is checked before a counter is spent. TOWER's `Net::send_ack`.
    async fn send_ack(&mut self, dest: u32, acked: u32, rssi: i16) {
        if !self.can_send() {
            return;
        }
        self.sleep_ms(ACK_TURNAROUND_MS).await;
        let Some(peer) = self.peer(dest) else {
            return;
        };
        let key = peer.key;
        let mut payload = [0u8; ACK_LEN];
        payload[..4].copy_from_slice(&acked.to_le_bytes());
        // Clamped, not wrapped: below -128 dBm a bare cast would report a strong link.
        payload[4] = rssi.clamp(i16::from(i8::MIN), i16::from(i8::MAX)) as i8 as u8;
        payload[5] = if peer.waiting.is_some() {
            ack_flags::PENDING
        } else {
            0
        };
        if !self.duty_allows(frame_toa_ms(HDR_LEN + ACK_LEN + TAG_LEN)) {
            return;
        }
        let Some(counter) = self.alloc_counter() else {
            return;
        };
        let header = Header {
            frame_type: FrameType::Ack,
            flags: 0,
            src: self.addr,
            dest,
            counter,
            bulk_index: None,
        };
        let mut out = [0u8; MAX_FRAME];
        if let Ok(n) = frame::seal_frame(&mut self.ccm, &key, &header, &payload, &mut out) {
            let _ = self.radio.send(&out[..n]).await;
        }
    }

    /// The counter for one transfer, spent before anything is sent; `None` when sending is not
    /// allowed. Keeps the next watermark when the kernel asks, and locks when it cannot.
    fn alloc_counter(&mut self) -> Option<u32> {
        if !self.floor_known {
            return None;
        }
        let (counter, persist) = self.txc.alloc()?;
        if let Some(next) = persist {
            if self.watermark.store(next) {
                self.txc.reserve_persisted(next);
            } else {
                self.txc.lock();
            }
        }
        Some(counter)
    }

    /// Whether the duty cycle has room for `toa_ms` on air now; takes it when it has.
    fn duty_allows(&mut self, toa_ms: u32) -> bool {
        let now = self.clock.now_ms();
        let elapsed = u32::try_from(now.saturating_sub(self.duty_at)).unwrap_or(u32::MAX);
        self.duty_at = now;
        self.duty.refill_ms(elapsed);
        self.duty.try_consume(toa_ms)
    }

    async fn sleep_ms(&self, ms: u64) {
        self.clock
            .sleep_until(self.clock.now_ms().saturating_add(ms))
            .await;
    }

    /// 0 to [`MAX_BACKOFF_MS`] - 1, from xorshift32: enough to part two senders, never a secret.
    fn backoff_ms(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x % MAX_BACKOFF_MS
    }

    fn peer(&self, addr: u32) -> Option<&Peer> {
        self.peers.iter().flatten().find(|p| p.addr == addr)
    }

    fn peer_mut(&mut self, addr: u32) -> Option<&mut Peer> {
        self.peers.iter_mut().flatten().find(|p| p.addr == addr)
    }
}
