//! Pairing a node (`docs/firmware/radio.md`, "Pairing a node"): the gateway's side of TOWER's
//! `firmware/src/radio/net/pairing.rs` at the pinned revision, written again over the
//! [`Gateway`] (T47; T27 watches the two for drift).
//!
//! ```text
//! node     JoinReq      3 | E_node              to address 0   under JOIN_FRAME_KEY
//! gateway  JoinResp     3 | E_gw | C_gw         to the node    under JOIN_FRAME_KEY
//! node     JoinNonce    3 | N_node              to the gateway under JOIN_FRAME_KEY
//! gateway  JoinReveal   3 | N_gw | tag_gw       to the node    under JOIN_FRAME_KEY
//! node     JoinConfirm  3 | tag_node            to the gateway under the new link key
//! ```
//!
//! The commitment, the key schedule, the tags and the six-digit code are
//! `tower_net::handshake`'s; who may be answered is `tower_net::pairing`'s. Here: the window, the
//! frames, the waits. Also the keyed join of a node whose key the gateway holds, answered inside
//! [`Gateway::recv`].

use tower_net::handshake::{
    CONFIRM_LEN, ENTROPY_LEN, Ephemeral, NONCE_LEN, NONCE_MSG_LEN, PUBLIC_LEN, REQ_LEN, RESP_LEN, REVEAL_LEN,
    Role, VERSION,
};
use tower_net::pairing::{handshake_fits, join_request_admissible, keyed_req_admit, keyed_req_valid};
use tower_radio::frame_toa_ms;
use zeroize::Zeroize;

use super::{ACK_TURNAROUND_MS, Clock, Gateway, Watermark};
use crate::frame::{self, AesBlock, FrameType, HDR_LEN, Header, MAX_FRAME, TAG_LEN, flags};
use crate::radio::Radio;

/// The public framing key of the four pairing frames before the link key (TOWER's
/// `JOIN_FRAME_KEY`, its spelling kept by TOWER on purpose). **Not a secret:** it gives those
/// frames a CCM format, so a damaged one is dropped; secrecy comes from the key agreement.
pub const JOIN_FRAME_KEY: [u8; 16] = *b"TOWER-JOIN-v2\0\0\0";
/// The window the agent opens when it names none (TOWER's `PAIRING_WINDOW`).
pub const PAIRING_WINDOW_MS: u64 = 60_000;
/// The radio's part of every wait for the node's next frame (TOWER's `JOIN_STEP_WINDOW`).
pub const JOIN_STEP_WINDOW_MS: u64 = 300;
/// The node's shared secret before its confirmation: about 1.2 s on a Core Module's Cortex-M0+
/// (TOWER measured 1185 ms), with margin (TOWER's `PEER_KEX_TIME`).
pub const PEER_KEX_TIME_MS: u64 = 1_500;
/// How long the gateway listens for a request at a time, between checks of the window.
const PAIR_RX_SLICE_MS: u64 = 500;
/// TOWER's bound on sending one frame, kept in the handshake's length so that the two agree.
const TX_TIMEOUT_MS: u64 = 120;
/// The gateway's whole handshake, from an answered request to the end of its wait for the
/// confirmation (TOWER's `HOST_HANDSHAKE_TIME`, about 3.9 s): a request is answered only while
/// this much of the window is left.
pub const HOST_HANDSHAKE_TIME_MS: u64 =
    2 * (ACK_TURNAROUND_MS + TX_TIMEOUT_MS) + 2 * (JOIN_STEP_WINDOW_MS + PEER_KEX_TIME_MS);
/// A keyed `JoinReq` or `JoinResp` on air: the header, 4 bytes, the tag.
const KEYED_FRAME_LEN: usize = HDR_LEN + 4 + TAG_LEN;

/// Random bytes for the key pairs: the STM32WL's true random number generator. `false` when it
/// cannot give them: pairing is then refused, never keyed from anything else.
pub trait Entropy {
    fn fill(&mut self, out: &mut [u8]) -> bool;
}

/// A node that paired: its own address, its new link key, and the six-digit code to compare
/// (0 to 999 999). The agent adds it ([`Gateway::add_peer`], counter 0) and keeps it.
#[derive(Clone, Copy)]
pub struct PairedPeer {
    pub addr: u32,
    pub key: [u8; 16],
    pub sas: u32,
}

/// Why a node's request to join was not answered, for the agent to tell (TOWER logs it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinRefusal {
    /// Not a request of this handshake (another version of TOWER's pairing).
    NotV3,
    /// A node the gateway knows: it is removed first to pair again.
    AlreadyRegistered,
    /// A reserved address, or the gateway's own.
    InvalidAddress,
    /// Less than a whole handshake is left of the window.
    WindowClosing,
    /// The node's public key is a low-order point: the shared secret would be known to anyone.
    LowOrderKey,
    /// A keyed join, while keyed joins are switched off.
    KeyedOff,
    /// A keyed join from a node the gateway has no key for.
    KeyedUnknown,
    /// A keyed join that does not open under the node's key, or does not carry its address.
    KeyedNotAuthentic,
    /// A keyed join whose counter is not new: replayed, or a node that started its counter
    /// again under the same key.
    KeyedStale { counter: u32, last_seen: u32 },
    /// The answer could not be sent: the counter is locked, or the duty cycle has no room.
    CannotSend,
}

/// An open pairing window: when it closes, and the key pair for the next request it answers
/// (made ahead, so an answer pays only for the shared secret). Each key pair answers one request
/// and is replaced, whether its handshake ends or not. Dropped, its secret is wiped.
pub struct PairingHost {
    eph: Option<Ephemeral>,
    window_end_ms: u64,
}

impl PairingHost {
    /// When the window closes.
    pub fn window_end_ms(&self) -> u64 {
        self.window_end_ms
    }

    /// A key pair ready; `false` when the generator cannot give one.
    fn ready(&mut self, entropy: &mut impl Entropy) -> bool {
        if self.eph.is_none() {
            self.eph = draw(entropy);
        }
        self.eph.is_some()
    }
}

/// A key pair and nonce from [`ENTROPY_LEN`] random bytes, which are wiped after.
fn draw(entropy: &mut impl Entropy) -> Option<Ephemeral> {
    let mut bytes = [0u8; ENTROPY_LEN];
    let eph = entropy.fill(&mut bytes).then(|| Ephemeral::from_entropy(&bytes));
    bytes.zeroize();
    eph
}

impl<R: Radio, C: Clock, A: AesBlock, W: Watermark> Gateway<R, C, A, W> {
    /// Opens a pairing window of `window_ms` from now: its end is fixed first, then its first key
    /// pair is drawn. `None` when the generator cannot give one.
    pub fn open_window(&self, entropy: &mut impl Entropy, window_ms: u64) -> Option<PairingHost> {
        let window_end_ms = self.clock.now_ms().saturating_add(window_ms);
        Some(PairingHost {
            eph: Some(draw(entropy)?),
            window_end_ms,
        })
    }

    /// Switches answering keyed joins on or off (TOWER's gateway setting `keyed-join`, on by
    /// default there; off here until the agent says).
    pub fn set_keyed_join(&mut self, on: bool) {
        self.keyed_join = on;
    }

    /// The node whose keyed join was answered since the last call.
    pub fn take_keyed_join(&mut self) -> Option<u32> {
        self.keyed_joined.take()
    }

    /// The last request to join that was not answered, and who sent it, since the last call.
    pub fn take_join_refusal(&mut self) -> Option<(u32, JoinRefusal)> {
        self.join_refusal.take()
    }

    /// Listens for nodes that want to join, until `until_ms` or the window's end, whichever is
    /// first, and pairs the first one whose handshake ends with a right confirmation. A request
    /// answered before then runs its handshake to the end (at most [`HOST_HANDSHAKE_TIME_MS`],
    /// never past the window). `None`: nobody paired in that time. TOWER's `Net::open_pairing`.
    pub async fn pair(
        &mut self,
        host: &mut PairingHost,
        entropy: &mut impl Entropy,
        until_ms: u64,
    ) -> Option<PairedPeer> {
        let deadline = until_ms.min(host.window_end_ms);
        if !self.can_send() {
            // Every answer takes a counter: nothing can be answered.
            self.clock.sleep_until(deadline).await;
            return None;
        }
        let mut req = [0u8; REQ_LEN];
        while self.clock.now_ms() < deadline {
            if !host.ready(entropy) {
                self.clock.sleep_until(deadline).await;
                return None;
            }
            let listen_end = self.clock.now_ms().saturating_add(PAIR_RX_SLICE_MS).min(deadline);
            let Some((hdr, len)) = self.rx_under(&JOIN_FRAME_KEY, listen_end, &mut req).await else {
                continue;
            };
            if hdr.frame_type != FrameType::JoinReq {
                continue;
            }
            let node = hdr.src;
            let registered = self.peer(node).is_some();
            let left = host.window_end_ms.saturating_sub(self.clock.now_ms());
            let refusal = if len != REQ_LEN || req[0] != VERSION {
                Some(JoinRefusal::NotV3)
            } else if !join_request_admissible(node, self.addr, registered) {
                Some(if registered {
                    JoinRefusal::AlreadyRegistered
                } else {
                    JoinRefusal::InvalidAddress
                })
            } else if !handshake_fits(left, HOST_HANDSHAKE_TIME_MS) {
                Some(JoinRefusal::WindowClosing)
            } else {
                None
            };
            if let Some(why) = refusal {
                self.join_refusal = Some((node, why));
                continue;
            }

            // This key pair answers this request only. Taken out, its bytes would stay in the
            // window's slot, which the moved value's wiping cannot reach: wipe the slot.
            let Some(eph) = host.eph.take() else {
                continue;
            };
            host.eph.zeroize();
            let mut e_node = [0u8; PUBLIC_LEN];
            e_node.copy_from_slice(&req[1..REQ_LEN]);
            let mut resp = [0u8; RESP_LEN];
            resp[0] = VERSION;
            resp[1..1 + PUBLIC_LEN].copy_from_slice(eph.public());
            resp[1 + PUBLIC_LEN..].copy_from_slice(&eph.commitment(&e_node, node, self.addr));
            if !self
                .send_join(FrameType::JoinResp, node, &JOIN_FRAME_KEY, &resp)
                .await
            {
                continue;
            }

            // The node's nonce, from that node, to us; another frame does not end the wait.
            let nonce_end = self.clock.now_ms().saturating_add(JOIN_STEP_WINDOW_MS);
            let mut n_node = None;
            while self.clock.now_ms() < nonce_end {
                let mut nbuf = [0u8; NONCE_MSG_LEN];
                if let Some((h, n)) = self.rx_under(&JOIN_FRAME_KEY, nonce_end, &mut nbuf).await
                    && h.frame_type == FrameType::JoinNonce
                    && h.src == node
                    && h.dest == self.addr
                    && n == NONCE_MSG_LEN
                    && nbuf[0] == VERSION
                {
                    n_node = Some(nbuf);
                    break;
                }
            }
            let Some(n_node) = n_node else {
                continue; // lost: this key pair is spent, the window stays open
            };
            let mut nonce = [0u8; NONCE_LEN];
            nonce.copy_from_slice(&n_node[1..]);
            let Some(session) = eph.session(&e_node, &nonce, Role::Gateway, node, self.addr) else {
                self.join_refusal = Some((node, JoinRefusal::LowOrderKey));
                continue;
            };
            let mut reveal = [0u8; REVEAL_LEN];
            reveal[0] = VERSION;
            reveal[1..1 + NONCE_LEN].copy_from_slice(eph.nonce());
            reveal[1 + NONCE_LEN..].copy_from_slice(session.tag(Role::Gateway));
            drop(eph); // the secret is wiped before anything waits
            if !self
                .send_join(FrameType::JoinReveal, node, &JOIN_FRAME_KEY, &reveal)
                .await
            {
                continue;
            }

            // The confirmation, under the new key, after the node's own shared secret: the whole
            // wait, as another frame or one that does not open must not end it.
            let confirm_end = self
                .clock
                .now_ms()
                .saturating_add(JOIN_STEP_WINDOW_MS + PEER_KEX_TIME_MS);
            while self.clock.now_ms() < confirm_end {
                let mut cbuf = [0u8; CONFIRM_LEN];
                if let Some((h, n)) = self.rx_under(session.link_key(), confirm_end, &mut cbuf).await
                    && h.frame_type == FrameType::JoinConfirm
                    && h.src == node
                    && h.dest == self.addr
                    && n == CONFIRM_LEN
                    && cbuf[0] == VERSION
                    && session.verify(Role::Node, &cbuf[1..])
                {
                    return Some(PairedPeer {
                        addr: node,
                        key: *session.link_key(),
                        sas: session.sas(),
                    });
                }
            }
        }
        None
    }

    /// A keyed join, answered when it may be: the request opens under the node's key, carries the
    /// node's address, and its counter is new on the node's lane. Returns the node and the sealed
    /// answer; the turnaround and the sending are the caller's. The duty cycle is checked before
    /// the counter is spent. TOWER's `Net::keyed_answer`.
    pub(super) fn keyed_answer(&mut self, frame: &mut [u8]) -> Option<(u32, [u8; KEYED_FRAME_LEN])> {
        let src = Header::parse(frame).map(|(h, _)| h.src).unwrap_or(0);
        match self.keyed_decision(frame) {
            Ok(answer) => Some(answer),
            Err(why) => {
                self.join_refusal = Some((src, why));
                None
            }
        }
    }

    fn keyed_decision(&mut self, frame: &mut [u8]) -> Result<(u32, [u8; KEYED_FRAME_LEN]), JoinRefusal> {
        if !self.keyed_join {
            return Err(JoinRefusal::KeyedOff);
        }
        if !self.can_send() {
            return Err(JoinRefusal::CannotSend);
        }
        let (peek, _) = Header::parse(frame).map_err(|_| JoinRefusal::KeyedNotAuthentic)?;
        let key = self.peer(peek.src).ok_or(JoinRefusal::KeyedUnknown)?.key;
        let (hdr, range) =
            frame::open_frame(&mut self.ccm, &key, frame).map_err(|_| JoinRefusal::KeyedNotAuthentic)?;
        let payload = frame.get(range).ok_or(JoinRefusal::KeyedNotAuthentic)?;
        if !keyed_req_valid(payload, hdr.src) {
            return Err(JoinRefusal::KeyedNotAuthentic);
        }
        let peer = self.peer_mut(hdr.src).ok_or(JoinRefusal::KeyedUnknown)?;
        if !keyed_req_admit(true, peer.lane.classify(hdr.counter)) {
            return Err(JoinRefusal::KeyedStale {
                counter: hdr.counter,
                last_seen: peer.lane.last_seen(),
            });
        }
        // Not kept here (the agent keeps the lanes, D8).
        let _ = peer.lane.accept(hdr.counter);
        if !self.duty_allows(frame_toa_ms(KEYED_FRAME_LEN)) {
            return Err(JoinRefusal::CannotSend);
        }
        let counter = self.alloc_counter().ok_or(JoinRefusal::CannotSend)?;
        let header = Header {
            frame_type: FrameType::JoinResp,
            flags: flags::KEYED,
            src: self.addr,
            dest: hdr.src,
            counter,
            bulk_index: None,
        };
        let mut answer = [0u8; KEYED_FRAME_LEN];
        frame::seal_frame(
            &mut self.ccm,
            &key,
            &header,
            &hdr.counter.to_le_bytes(),
            &mut answer,
        )
        .map_err(|_| JoinRefusal::CannotSend)?;
        Ok((hdr.src, answer))
    }

    /// One pairing frame to `node`, under `key`: its counter spent first, then the turnaround,
    /// the duty cycle, and the radio. `false`: nothing went out.
    async fn send_join(&mut self, frame_type: FrameType, node: u32, key: &[u8; 16], payload: &[u8]) -> bool {
        let Some(counter) = self.alloc_counter() else {
            return false;
        };
        let header = Header {
            frame_type,
            flags: 0,
            src: self.addr,
            dest: node,
            counter,
            bulk_index: None,
        };
        let mut out = [0u8; MAX_FRAME];
        let Ok(n) = frame::seal_frame(&mut self.ccm, key, &header, payload, &mut out) else {
            return false;
        };
        self.sleep_ms(ACK_TURNAROUND_MS).await;
        if !self.duty_allows(frame_toa_ms(n)) {
            return false;
        }
        self.radio.send(&out[..n]).await.is_ok()
    }

    /// One frame received before `until_ms` that opens under `key`: its header, and its payload's
    /// length in `out`.
    async fn rx_under(&mut self, key: &[u8; 16], until_ms: u64, out: &mut [u8]) -> Option<(Header, usize)> {
        let mut buf = [0u8; MAX_FRAME];
        let rx = super::first(self.radio.receive(&mut buf), self.clock.sleep_until(until_ms))
            .await?
            .ok()?;
        let frame = buf.get_mut(..rx.len)?;
        let (hdr, range) = frame::open_frame(&mut self.ccm, key, frame).ok()?;
        let payload = frame.get(range)?;
        out.get_mut(..payload.len())?.copy_from_slice(payload);
        Some((hdr, payload.len()))
    }
}
