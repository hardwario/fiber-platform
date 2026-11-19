//! The links between the agent and the FIBER's two microcontrollers (`docs/firmware/`): the south
//! link ([`south`]) and the north link ([`north`]), in the same frame, each with its own tag.
//!
//! The agent and the firmware build this one source, so they cannot disagree on a message. A
//! peer built from another source is refused by the schema tag every frame carries.
//!
//! ```text
//! wire:   COBS( inner ) 0x00
//! inner:  type(1) | tag(2, LE) | seq(2, LE) | payload (postcard) | crc32(4, LE)
//! ```
//!
//! A receiver checks the CRC first, then the tag, then the type, then that the payload is one
//! message with nothing left over. So a damaged frame is always [`Error::BadCrc`], and
//! [`Error::SchemaMismatch`] always means a whole frame from another build.
#![no_std]

pub mod crc;
pub mod fingerprint;
pub mod north;
pub mod south;

use serde::{Deserialize, Serialize};

/// The most bytes inside the COBS of one frame.
pub const MAX_INNER: usize = 128;
/// The most bytes of one frame on the wire: COBS adds one byte in 254, and one ahead; then the
/// `0x00`.
pub const MAX_WIRE: usize = MAX_INNER + MAX_INNER / 254 + 2;

/// The header: type, tag, seq.
const HEADER: usize = 5;
/// The CRC at the end.
const TRAILER: usize = 4;

/// Why a frame was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not COBS, or too short to hold a header and a CRC.
    Framing,
    /// The CRC does not match: the frame was damaged on the way.
    BadCrc,
    /// A whole frame, from a peer built from another source.
    SchemaMismatch { tag: u16 },
    /// A type this link does not have.
    UnknownType(u8),
    /// The payload is not one message of its type, or has bytes left over.
    Malformed,
    /// The message does not fit into [`MAX_INNER`], or the buffer is too small.
    TooLong,
}

/// The header of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub msg_type: u8,
    pub tag: u16,
    pub seq: u16,
}

/// Writes one frame, `0x00` included, into `out`; returns its length.
pub fn encode<T: Serialize>(header: Header, payload: &T, out: &mut [u8]) -> Result<usize, Error> {
    let mut inner = [0u8; MAX_INNER];
    inner[0] = header.msg_type;
    inner[1..3].copy_from_slice(&header.tag.to_le_bytes());
    inner[3..5].copy_from_slice(&header.seq.to_le_bytes());
    let body =
        postcard::to_slice(payload, &mut inner[HEADER..MAX_INNER - TRAILER]).map_err(|_| Error::TooLong)?;
    let len = HEADER + body.len();
    let crc = crc::crc32(&inner[..len]);
    inner[len..len + TRAILER].copy_from_slice(&crc.to_le_bytes());
    let inner = &inner[..len + TRAILER];
    if out.len() < cobs::max_encoding_length(inner.len()) + 1 {
        return Err(Error::TooLong);
    }
    let n = cobs::encode(inner, out);
    out[n] = 0;
    Ok(n + 1)
}

/// Decodes one frame in place (its COBS bytes, without the `0x00`) and checks it, CRC first,
/// then the tag. Returns the header and the payload.
pub fn decode(frame: &mut [u8], tag: u16) -> Result<(Header, &[u8]), Error> {
    let n = cobs::decode_in_place(frame).map_err(|_| Error::Framing)?;
    if n < HEADER + TRAILER {
        return Err(Error::Framing);
    }
    let (body, crc) = frame[..n].split_at(n - TRAILER);
    if crc::crc32(body).to_le_bytes() != crc {
        return Err(Error::BadCrc);
    }
    let header = Header {
        msg_type: body[0],
        tag: u16::from_le_bytes([body[1], body[2]]),
        seq: u16::from_le_bytes([body[3], body[4]]),
    };
    if header.tag != tag {
        return Err(Error::SchemaMismatch { tag: header.tag });
    }
    Ok((header, &body[HEADER..]))
}

/// A payload as one message of type `T`, with nothing left over.
pub fn payload<'a, T: Deserialize<'a>>(payload: &'a [u8]) -> Result<T, Error> {
    match postcard::take_from_bytes(payload) {
        Ok((msg, [])) => Ok(msg),
        _ => Err(Error::Malformed),
    }
}

/// Collects bytes as they come off the serial port into frames.
///
/// A frame longer than `N` is dropped whole, and the decoder starts again at the next `0x00`;
/// so is any byte noise between frames, which ends up as a frame that does not decode.
pub struct FrameDecoder<const N: usize> {
    buf: [u8; N],
    len: usize,
    overflow: bool,
}

impl<const N: usize> Default for FrameDecoder<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> FrameDecoder<N> {
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
            overflow: false,
        }
    }

    /// Takes one byte. At the end of a frame, returns its COBS bytes (without the `0x00`), for
    /// [`decode`]; an empty frame, and one that did not fit, are skipped.
    pub fn push(&mut self, byte: u8) -> Option<&mut [u8]> {
        if byte == 0 {
            let len = core::mem::replace(&mut self.len, 0);
            let overflow = core::mem::replace(&mut self.overflow, false);
            return (!overflow && len > 0).then(|| &mut self.buf[..len]);
        }
        if self.len < N {
            self.buf[self.len] = byte;
            self.len += 1;
        } else {
            self.overflow = true;
        }
        None
    }
}
