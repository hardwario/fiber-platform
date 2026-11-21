//! The north link: the agent and the north bridge, the radio (`docs/firmware/north-link.md`).

use core::fmt;

use postcard_schema::Schema;
use postcard_schema::schema::{DataModelType, NamedType};
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};

use crate::fingerprint::{NORTH_LINK, feed, fold16, hash_schema, link_basis};
pub use crate::south::{Fault, Reset};
use crate::{Error, Header};

/// The longest payload of a node's frame (TOWER's `MAX_PAYLOAD`).
pub const MAX_DATA: usize = 74;

/// A node's payload: up to [`MAX_DATA`] bytes, written as postcard writes a byte string (its
/// length, then the bytes). A longer one is not read.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Bytes {
    len: u8,
    buf: [u8; MAX_DATA],
}

impl Bytes {
    /// `None` when `data` is longer than [`MAX_DATA`].
    pub fn new(data: &[u8]) -> Option<Self> {
        let len = u8::try_from(data.len())
            .ok()
            .filter(|&n| usize::from(n) <= MAX_DATA)?;
        let mut buf = [0u8; MAX_DATA];
        buf[..data.len()].copy_from_slice(data);
        Some(Bytes { len, buf })
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf[..usize::from(self.len)]
    }
}

impl Default for Bytes {
    fn default() -> Self {
        Bytes {
            len: 0,
            buf: [0; MAX_DATA],
        }
    }
}

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bytes({:02x?})", self.as_slice())
    }
}

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(self.as_slice())
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Up;
        impl Visitor<'_> for Up {
            type Value = Bytes;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "at most {MAX_DATA} bytes")
            }
            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Bytes, E> {
                Bytes::new(v).ok_or_else(|| E::invalid_length(v.len(), &self))
            }
        }
        deserializer.deserialize_bytes(Up)
    }
}

impl Schema for Bytes {
    const SCHEMA: &'static NamedType = &NamedType {
        name: "Bytes",
        ty: &DataModelType::ByteArray,
    };
}

/// What the radio speaks (D4): one at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum Personality {
    /// TOWER's GFSK in EU 868, as its nodes speak it.
    TowerFsk,
}

/// The band (TOWER's EU 868 only, for now).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum Band {
    Eu868,
}

/// Who the firmware is, and how it came to be running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Hello {
    pub version: [u8; 3],
    /// The CRC-32/IEEE of its own image in flash.
    pub image_crc: u32,
    /// The chip's unique id.
    pub uid: [u8; 12],
    pub reset: Reset,
    pub fault: Option<Fault>,
    /// The personalities it has, one bit each: bit 0 `TowerFsk`.
    pub personalities: u8,
    pub active: Personality,
    /// Every transmit counter used so far is below this.
    pub tx_floor: u32,
}

/// How a message from the agent went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum Outcome {
    Done,
    /// A node the north bridge does not have.
    Unknown,
    /// 16 nodes already.
    Full,
    /// A reserved address, a payload too long, a setting out of range.
    Invalid,
    /// No `Settings` yet, or a pairing window already open.
    NotNow,
    /// The watermark cannot be kept: nothing is sent.
    Locked,
    /// No random numbers for pairing.
    NoEntropy,
}

/// The answer to a message from the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Answer {
    /// The `seq` of the message it answers.
    pub seq: u16,
    pub result: Outcome,
}

/// The radio's numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Status {
    /// Seconds since the firmware started.
    pub uptime: u32,
    /// As in `Hello`.
    pub tx_floor: u32,
    /// The counter has its floor and its watermark is kept.
    pub can_send: bool,
    /// What is left of the duty cycle's budget, in ms.
    pub duty_left_ms: u32,
    /// The frames heard since start, by what became of them.
    pub frames: Frames,
    /// Link frames dropped since start; it wraps.
    pub rx_dropped: u16,
}

/// The frames the north bridge heard since it started, by what became of them; each wraps
/// (`docs/firmware/north-link.md`, `Status`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Frames {
    /// Taken in and passed on as an `Uplink`.
    pub taken_in: u32,
    /// A copy of a frame already taken in: the node did not hear the acknowledgement.
    pub repeated: u32,
    /// An older counter than the node's last.
    pub replayed: u32,
    /// From a known node, its tag not checking under the node's key.
    pub not_authentic: u32,
    /// To this gateway, from a node it does not have.
    pub unknown_node: u32,
    /// To another address, not data, or not a frame of this layout.
    pub not_for_us: u32,
}

/// A node's new frame, taken in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Uplink {
    pub node: u32,
    pub counter: u32,
    /// dBm.
    pub rssi: i16,
    /// It asked for an acknowledgement, and got one.
    pub confirmed: bool,
    pub data: Bytes,
}

/// How a queued message ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum Sent {
    Delivered,
    /// No acknowledgement after its tries: it stays queued.
    NotDelivered,
    /// Not sent: it stays queued.
    DutyLimited,
    /// Not sent at all: the counter is locked.
    Refused,
}

/// A queued message went out, or did not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Delivery {
    pub node: u32,
    pub result: Sent,
    /// The strength the node heard it at, from its acknowledgement.
    pub ack_rssi: Option<i8>,
}

/// A node paired: the north bridge has added it, from counter 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Paired {
    /// The address it chose.
    pub node: u32,
    /// Its new link key.
    pub key: [u8; 16],
    /// The six-digit code, 0 to 999 999.
    pub sas: u32,
}

/// Why a node's request to join was not answered (`docs/firmware/radio.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum JoinRefusal {
    NotV3,
    AlreadyRegistered,
    InvalidAddress,
    WindowClosing,
    LowOrderKey,
    KeyedOff,
    KeyedUnknown,
    KeyedNotAuthentic,
    KeyedStale { counter: u32, last_seen: u32 },
    CannotSend,
}

/// A node asked to join and was not answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct JoinRefused {
    pub node: u32,
    pub why: JoinRefusal,
}

/// A known node joined under its key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct KeyedJoin {
    pub node: u32,
}

/// One node, in answer to `ListNodes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Node {
    pub node: u32,
    /// The last counter taken in from it.
    pub last_seen: u32,
    /// A message waits for it.
    pub queued: bool,
}

/// The radio's personality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct SetPersonality {
    pub personality: Personality,
}

/// The gateway's settings; the radio starts with the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Settings {
    /// The gateway's address (not 0, not `0xFFFFFFFF`).
    pub address: u32,
    pub band: Band,
    /// 0 to 2: 868.1, 868.3, 868.5 MHz.
    pub channel: u8,
    /// Answer the keyed joins of known nodes.
    pub keyed_join: bool,
    /// The agent's floor for the transmit counter.
    pub tx_floor: u32,
}

/// A node, with its key and the last counter the agent has from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct NodeAdd {
    pub node: u32,
    pub key: [u8; 16],
    pub last_seen: u32,
}

/// A node, by its address (`NodeRemove`, `Unqueue`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct NodeRef {
    pub node: u32,
}

/// A message to wait for a node's next uplink.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Queue {
    pub node: u32,
    pub data: Bytes,
}

/// A pairing window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct PairingOpen {
    /// 10 to 600.
    pub seconds: u16,
}

/// The messages of the north link, each with its type byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    /// `0x01`, north to agent.
    Hello(Hello),
    /// `0x02`, north to agent.
    Answer(Answer),
    /// `0x03`, north to agent.
    Status(Status),
    /// `0x04`, north to agent.
    Uplink(Uplink),
    /// `0x05`, north to agent.
    Delivery(Delivery),
    /// `0x06`, north to agent.
    Paired(Paired),
    /// `0x07`, north to agent; no payload.
    PairingClosed,
    /// `0x08`, north to agent.
    JoinRefused(JoinRefused),
    /// `0x09`, north to agent.
    KeyedJoin(KeyedJoin),
    /// `0x0A`, north to agent.
    Node(Node),
    /// `0x10`, agent to north; no payload.
    HelloRequest,
    /// `0x11`, agent to north.
    SetPersonality(SetPersonality),
    /// `0x12`, agent to north.
    Settings(Settings),
    /// `0x13`, agent to north.
    NodeAdd(NodeAdd),
    /// `0x14`, agent to north.
    NodeRemove(NodeRef),
    /// `0x15`, agent to north; no payload.
    ListNodes,
    /// `0x16`, agent to north.
    Queue(Queue),
    /// `0x17`, agent to north.
    Unqueue(NodeRef),
    /// `0x18`, agent to north.
    PairingOpen(PairingOpen),
    /// `0x19`, agent to north; no payload.
    PairingCancel,
    /// `0x1A`, agent to north; no payload.
    StatusRequest,
}

const HELLO: u8 = 0x01;
const ANSWER: u8 = 0x02;
const STATUS: u8 = 0x03;
const UPLINK: u8 = 0x04;
const DELIVERY: u8 = 0x05;
const PAIRED: u8 = 0x06;
const PAIRING_CLOSED: u8 = 0x07;
const JOIN_REFUSED: u8 = 0x08;
const KEYED_JOIN: u8 = 0x09;
const NODE: u8 = 0x0a;
const HELLO_REQUEST: u8 = 0x10;
const SET_PERSONALITY: u8 = 0x11;
const SETTINGS: u8 = 0x12;
const NODE_ADD: u8 = 0x13;
const NODE_REMOVE: u8 = 0x14;
const LIST_NODES: u8 = 0x15;
const QUEUE: u8 = 0x16;
const UNQUEUE: u8 = 0x17;
const PAIRING_OPEN: u8 = 0x18;
const PAIRING_CANCEL: u8 = 0x19;
const STATUS_REQUEST: u8 = 0x1a;

const fn north_hash() -> u64 {
    let unit = <() as Schema>::SCHEMA;
    let mut s = link_basis(NORTH_LINK);
    s = hash_schema(feed(s, &[HELLO]), <Hello as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[ANSWER]), <Answer as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[STATUS]), <Status as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[UPLINK]), <Uplink as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[DELIVERY]), <Delivery as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[PAIRED]), <Paired as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[PAIRING_CLOSED]), unit);
    s = hash_schema(feed(s, &[JOIN_REFUSED]), <JoinRefused as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[KEYED_JOIN]), <KeyedJoin as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[NODE]), <Node as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[HELLO_REQUEST]), unit);
    s = hash_schema(feed(s, &[SET_PERSONALITY]), <SetPersonality as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[SETTINGS]), <Settings as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[NODE_ADD]), <NodeAdd as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[NODE_REMOVE]), <NodeRef as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[LIST_NODES]), unit);
    s = hash_schema(feed(s, &[QUEUE]), <Queue as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[UNQUEUE]), <NodeRef as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[PAIRING_OPEN]), <PairingOpen as Schema>::SCHEMA);
    s = hash_schema(feed(s, &[PAIRING_CANCEL]), unit);
    hash_schema(feed(s, &[STATUS_REQUEST]), unit)
}

/// The schema tag of the north link.
pub const TAG: u16 = fold16(north_hash());

impl Message {
    /// Writes the message as one frame, `0x00` included; returns its length.
    pub fn encode(&self, seq: u16, out: &mut [u8]) -> Result<usize, Error> {
        let h = |msg_type| Header {
            msg_type,
            tag: TAG,
            seq,
        };
        match self {
            Message::Hello(m) => crate::encode(h(HELLO), m, out),
            Message::Answer(m) => crate::encode(h(ANSWER), m, out),
            Message::Status(m) => crate::encode(h(STATUS), m, out),
            Message::Uplink(m) => crate::encode(h(UPLINK), m, out),
            Message::Delivery(m) => crate::encode(h(DELIVERY), m, out),
            Message::Paired(m) => crate::encode(h(PAIRED), m, out),
            Message::PairingClosed => crate::encode(h(PAIRING_CLOSED), &(), out),
            Message::JoinRefused(m) => crate::encode(h(JOIN_REFUSED), m, out),
            Message::KeyedJoin(m) => crate::encode(h(KEYED_JOIN), m, out),
            Message::Node(m) => crate::encode(h(NODE), m, out),
            Message::HelloRequest => crate::encode(h(HELLO_REQUEST), &(), out),
            Message::SetPersonality(m) => crate::encode(h(SET_PERSONALITY), m, out),
            Message::Settings(m) => crate::encode(h(SETTINGS), m, out),
            Message::NodeAdd(m) => crate::encode(h(NODE_ADD), m, out),
            Message::NodeRemove(m) => crate::encode(h(NODE_REMOVE), m, out),
            Message::ListNodes => crate::encode(h(LIST_NODES), &(), out),
            Message::Queue(m) => crate::encode(h(QUEUE), m, out),
            Message::Unqueue(m) => crate::encode(h(UNQUEUE), m, out),
            Message::PairingOpen(m) => crate::encode(h(PAIRING_OPEN), m, out),
            Message::PairingCancel => crate::encode(h(PAIRING_CANCEL), &(), out),
            Message::StatusRequest => crate::encode(h(STATUS_REQUEST), &(), out),
        }
    }

    /// Reads one frame (its COBS bytes, without the `0x00`), in place; returns its `seq` and the
    /// message.
    pub fn decode(frame: &mut [u8]) -> Result<(u16, Message), Error> {
        let (header, p) = crate::decode(frame, TAG)?;
        let unit = |p| crate::payload::<()>(p);
        let message = match header.msg_type {
            HELLO => Message::Hello(crate::payload(p)?),
            ANSWER => Message::Answer(crate::payload(p)?),
            STATUS => Message::Status(crate::payload(p)?),
            UPLINK => Message::Uplink(crate::payload(p)?),
            DELIVERY => Message::Delivery(crate::payload(p)?),
            PAIRED => Message::Paired(crate::payload(p)?),
            PAIRING_CLOSED => unit(p).map(|()| Message::PairingClosed)?,
            JOIN_REFUSED => Message::JoinRefused(crate::payload(p)?),
            KEYED_JOIN => Message::KeyedJoin(crate::payload(p)?),
            NODE => Message::Node(crate::payload(p)?),
            HELLO_REQUEST => unit(p).map(|()| Message::HelloRequest)?,
            SET_PERSONALITY => Message::SetPersonality(crate::payload(p)?),
            SETTINGS => Message::Settings(crate::payload(p)?),
            NODE_ADD => Message::NodeAdd(crate::payload(p)?),
            NODE_REMOVE => Message::NodeRemove(crate::payload(p)?),
            LIST_NODES => unit(p).map(|()| Message::ListNodes)?,
            QUEUE => Message::Queue(crate::payload(p)?),
            UNQUEUE => Message::Unqueue(crate::payload(p)?),
            PAIRING_OPEN => Message::PairingOpen(crate::payload(p)?),
            PAIRING_CANCEL => unit(p).map(|()| Message::PairingCancel)?,
            STATUS_REQUEST => unit(p).map(|()| Message::StatusRequest)?,
            other => return Err(Error::UnknownType(other)),
        };
        Ok((header.seq, message))
    }
}
