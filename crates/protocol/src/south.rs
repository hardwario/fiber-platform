//! The south link: the agent and the south bridge (`docs/firmware/south-link.md`).

use postcard_schema::Schema;
use serde::{Deserialize, Serialize};

use crate::fingerprint::{SOUTH_LINK, feed, fold16, hash_schema, link_basis};
use crate::{Error, Header};

/// The LED lines: the power LED's green and yellow, then each port's green and red.
pub const LED_LINES: usize = 18;
/// The 1-Wire ports.
pub const PORTS: usize = 8;

/// One LED line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum Light {
    #[default]
    Off,
    On,
    /// Lit for `on` ticks, dark for `off` ticks, starting `phase` ticks into the cycle. A tick is
    /// 20 ms.
    Blink {
        on: u8,
        off: u8,
        phase: u8,
    },
}

/// What the agent wants, all of it, every time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Desired {
    pub leds: [Light; LED_LINES],
    /// Bit 0 is port 1; 1 = on.
    pub ports: u8,
}

/// The answer to a `Desired`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Status {
    /// The `seq` of the `Desired` now applied.
    pub applied: u16,
    pub ports: u8,
    /// ADC counts (12 bits): the battery, the 12 V rail, the internal reference.
    pub vbat: u16,
    pub vin: u16,
    pub vref: u16,
    /// Seconds since the firmware started.
    pub uptime: u32,
    /// Frames dropped since start; it wraps.
    pub rx_dropped: u16,
}

/// Why the firmware last started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum Reset {
    PowerOn,
    Pin,
    Watchdog,
    Software,
    BrownOut,
    Other,
}

/// A fault the firmware caught before it reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Fault {
    /// 1: a panic; 2: a hard fault.
    pub code: u8,
    /// Where it happened; 0 when not known.
    pub address: u32,
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
    /// `FLASH_OPTR` as it reads at start.
    pub option_bytes: u32,
    /// The factory calibration of the internal reference (`VREFINT_CAL`): its ADC count at a
    /// supply of 3.0 V, so the agent can turn the counts into volts.
    pub vrefint_cal: u16,
}

/// The messages of the south link, each with its type byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    /// `0x01`, south to agent.
    Hello(Hello),
    /// `0x02`, south to agent.
    Status(Status),
    /// `0x10`, agent to south.
    Desired(Desired),
    /// `0x11`, agent to south; no payload.
    HelloRequest,
}

const HELLO: u8 = 0x01;
const STATUS: u8 = 0x02;
const DESIRED: u8 = 0x10;
const HELLO_REQUEST: u8 = 0x11;

const fn south_hash() -> u64 {
    let mut state = link_basis(SOUTH_LINK);
    state = hash_schema(feed(state, &[HELLO]), <Hello as Schema>::SCHEMA);
    state = hash_schema(feed(state, &[STATUS]), <Status as Schema>::SCHEMA);
    state = hash_schema(feed(state, &[DESIRED]), <Desired as Schema>::SCHEMA);
    hash_schema(feed(state, &[HELLO_REQUEST]), <() as Schema>::SCHEMA)
}

/// The schema tag of the south link.
pub const TAG: u16 = fold16(south_hash());

impl Message {
    /// Writes the message as one frame, `0x00` included; returns its length.
    pub fn encode(&self, seq: u16, out: &mut [u8]) -> Result<usize, Error> {
        let header = |msg_type| Header {
            msg_type,
            tag: TAG,
            seq,
        };
        match self {
            Message::Hello(m) => crate::encode(header(HELLO), m, out),
            Message::Status(m) => crate::encode(header(STATUS), m, out),
            Message::Desired(m) => crate::encode(header(DESIRED), m, out),
            Message::HelloRequest => crate::encode(header(HELLO_REQUEST), &(), out),
        }
    }

    /// Reads one frame (its COBS bytes, without the `0x00`), in place; returns its `seq` and the
    /// message.
    pub fn decode(frame: &mut [u8]) -> Result<(u16, Message), Error> {
        let (header, payload) = crate::decode(frame, TAG)?;
        let message = match header.msg_type {
            HELLO => Message::Hello(crate::payload(payload)?),
            STATUS => Message::Status(crate::payload(payload)?),
            DESIRED => Message::Desired(crate::payload(payload)?),
            HELLO_REQUEST => {
                crate::payload::<()>(payload)?;
                Message::HelloRequest
            }
            other => return Err(Error::UnknownType(other)),
        };
        Ok((header.seq, message))
    }
}
