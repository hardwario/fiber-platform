//! TOWER's network frame: the payload of a [`packet`](crate::packet), written again from TOWER's
//! `firmware/src/radio/frame.rs` at the pinned revision (T24; T27 watches the two for drift).
//! The decisions stay TOWER's: the nonce is `tower_net::nonce`'s, AES-128-CCM is
//! `tower_radio::ccm`'s, over any [`AesBlock`] ([`SoftAes`] here; the STM32WL's AES engine is the
//! other one it can have).
//!
//! On air, little-endian:
//!
//! | fmt_type | flags | src(4) | dest(4) | counter(4) | [bulk_idx(3)] | payload | tag(8) |
//!
//! The whole clear header is the CCM's associated data, the payload is encrypted, and the 8-byte
//! tag authenticates both. The 13-byte nonce is made from the header (`src | counter |
//! bulk_index | 0x0000`), never sent.

use tower_net::nonce::nonce_for;
pub use tower_radio::ccm::{AesBlock, Ccm, NONCE_LEN, TAG_LEN};

/// The frame format, `bits[7:5]` of the first byte. Another value is not a TOWER frame of this
/// layout: it is dropped before any CCM work.
pub const FORMAT: u8 = 1;
/// The header without a bulk index (fmt_type, flags, src, dest, counter).
pub const HDR_LEN: usize = 14;
/// The header with the 3-byte bulk index.
pub const HDR_LEN_BULK: usize = 17;
/// The largest payload of a frame that is not bulk: 96 (the SPIRIT1's FIFO) - 14 - 8.
pub const MAX_PAYLOAD: usize = 74;
/// The largest chunk of a bulk frame (96 - 17 - 8 = 71, capped by the protocol at 64).
pub const MAX_BULK_PAYLOAD: usize = 64;
/// The longest frame: the SPIRIT1's FIFO.
pub const MAX_FRAME: usize = 96;

// TOWER's application messages are sized against this MTU (`tower_protocol::radio`): if the two
// drift, a node could build a message the network layer refuses.
const _: () = assert!(tower_protocol::radio::MAX_RADIO_PAYLOAD == MAX_PAYLOAD);
const _: () = assert!(tower_net::nonce::NONCE_LEN == NONCE_LEN);

/// The frame types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    Data = 0,
    Ack = 1,
    BulkReq = 2,
    BulkData = 3,
    JoinReq = 4,
    JoinResp = 5,
    JoinConfirm = 6,
    /// The beacon of frequency hopping (US 915).
    Beacon = 7,
    /// Pairing: the joiner's nonce, once it holds the host's commitment.
    JoinNonce = 8,
    /// Pairing: the host shows the nonce it committed to, with its confirmation tag.
    JoinReveal = 9,
}

impl FrameType {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Data,
            1 => Self::Ack,
            2 => Self::BulkReq,
            3 => Self::BulkData,
            4 => Self::JoinReq,
            5 => Self::JoinResp,
            6 => Self::JoinConfirm,
            7 => Self::Beacon,
            8 => Self::JoinNonce,
            9 => Self::JoinReveal,
            _ => return None,
        })
    }

    /// Whether the frame type carries the 3-byte bulk index.
    pub fn has_bulk_index(self) -> bool {
        matches!(self, Self::BulkReq | Self::BulkData)
    }
}

/// The flag bits.
pub mod flags {
    /// The sender asks for an acknowledgement.
    pub const CONFIRMED: u8 = 1 << 0;
    /// A join sealed under the node's own key, not the public pairing key (JOIN_REQ, JOIN_RESP).
    pub const KEYED: u8 = 1 << 1;
    /// The last chunk of a bulk transfer.
    pub const LAST_CHUNK: u8 = 1 << 2;
    /// A data frame that announces a bulk transfer waiting.
    pub const BULK_ANNOUNCE: u8 = 1 << 3;
}

/// The clear header: the associated data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub frame_type: FrameType,
    pub flags: u8,
    pub src: u32,
    pub dest: u32,
    pub counter: u32,
    /// The 24-bit bulk index of a bulk frame; none otherwise.
    pub bulk_index: Option<u32>,
}

/// Why a frame was not made or not opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// Shorter than its header and tag, or a buffer too small.
    TooShort,
    /// Another frame format: drop it.
    BadFormat,
    /// A frame type not known: drop it.
    BadType,
    /// A payload over the frame's limit.
    PayloadTooLong,
    /// The tag does not match: forged, damaged, or another key. Drop it.
    AuthFail,
}

impl Header {
    /// The header's length on air.
    pub fn wire_len(&self) -> usize {
        if self.bulk_index.is_some() {
            HDR_LEN_BULK
        } else {
            HDR_LEN
        }
    }

    /// The longest payload this header's frame may carry.
    pub fn max_payload(&self) -> usize {
        if self.bulk_index.is_some() {
            MAX_BULK_PAYLOAD
        } else {
            MAX_PAYLOAD
        }
    }

    /// Writes the clear header into `out`; returns its length.
    pub fn encode(&self, out: &mut [u8]) -> usize {
        out[0] = (FORMAT << 5) | (self.frame_type as u8 & 0x1F);
        out[1] = self.flags;
        out[2..6].copy_from_slice(&self.src.to_le_bytes());
        out[6..10].copy_from_slice(&self.dest.to_le_bytes());
        out[10..14].copy_from_slice(&self.counter.to_le_bytes());
        if let Some(index) = self.bulk_index {
            out[14..17].copy_from_slice(&index.to_le_bytes()[..3]);
        }
        self.wire_len()
    }

    /// Reads a clear header from the start of `buf`; returns it and its length.
    pub fn parse(buf: &[u8]) -> Result<(Header, usize), FrameError> {
        if buf.len() < HDR_LEN {
            return Err(FrameError::TooShort);
        }
        if buf[0] >> 5 != FORMAT {
            return Err(FrameError::BadFormat);
        }
        let frame_type = FrameType::from_u8(buf[0] & 0x1F).ok_or(FrameError::BadType)?;
        let word = |at: usize| u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]);
        let (bulk_index, len) = if frame_type.has_bulk_index() {
            if buf.len() < HDR_LEN_BULK {
                return Err(FrameError::TooShort);
            }
            (
                Some(u32::from_le_bytes([buf[14], buf[15], buf[16], 0])),
                HDR_LEN_BULK,
            )
        } else {
            (None, HDR_LEN)
        };
        let header = Header {
            frame_type,
            flags: buf[1],
            src: word(2),
            dest: word(6),
            counter: word(10),
            bulk_index,
        };
        Ok((header, len))
    }

    /// The frame's CCM nonce: made from its header, never sent.
    pub fn nonce(&self) -> [u8; NONCE_LEN] {
        nonce_for(self.src, self.counter, self.bulk_index.unwrap_or(0))
    }
}

/// Makes a frame in `out`: the clear header, the payload encrypted, the tag. Returns its length.
pub fn seal_frame<A: AesBlock>(
    ccm: &mut Ccm<A>,
    key: &[u8; 16],
    header: &Header,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, FrameError> {
    if payload.len() > header.max_payload() {
        return Err(FrameError::PayloadTooLong);
    }
    let hlen = header.wire_len();
    let plen = payload.len();
    if out.len() < hlen + plen + TAG_LEN {
        return Err(FrameError::TooShort);
    }
    header.encode(out);
    out[hlen..hlen + plen].copy_from_slice(payload);
    let (aad, rest) = out.split_at_mut(hlen);
    let tag = ccm.seal(key, &header.nonce(), aad, &mut rest[..plen]);
    rest[plen..plen + TAG_LEN].copy_from_slice(&tag);
    Ok(hlen + plen + TAG_LEN)
}

/// Opens a frame in place: reads its header, checks its tag and decrypts its payload. Returns
/// the header and where the payload is in `buf`.
pub fn open_frame<A: AesBlock>(
    ccm: &mut Ccm<A>,
    key: &[u8; 16],
    buf: &mut [u8],
) -> Result<(Header, core::ops::Range<usize>), FrameError> {
    let (header, hlen) = Header::parse(buf)?;
    if buf.len() < hlen + TAG_LEN {
        return Err(FrameError::TooShort);
    }
    let plen = buf.len() - hlen - TAG_LEN;
    let mut tag = [0u8; TAG_LEN];
    tag.copy_from_slice(&buf[hlen + plen..]);
    let (aad, rest) = buf.split_at_mut(hlen);
    if !ccm.open(key, &header.nonce(), aad, &mut rest[..plen], &tag) {
        return Err(FrameError::AuthFail);
    }
    Ok((header, hlen..hlen + plen))
}

/// AES-128 in software (RustCrypto's `aes`), one block at a time, for the CCM.
#[derive(Clone, Copy, Debug, Default)]
pub struct SoftAes;

impl AesBlock for SoftAes {
    fn encrypt_block(&mut self, key: &[u8; 16], block: &mut [u8; 16]) {
        use aes::Aes128;
        use aes::cipher::{Array, BlockCipherEncrypt, KeyInit};
        let cipher = Aes128::new(&Array(*key));
        let mut b = Array(*block);
        cipher.encrypt_block(&mut b);
        block.copy_from_slice(&b.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 16] = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00,
    ];

    fn data(counter: u32) -> Header {
        Header {
            frame_type: FrameType::Data,
            flags: flags::CONFIRMED,
            src: 0x1111_1111,
            dest: 0x2222_2222,
            counter,
            bulk_index: None,
        }
    }

    /// RFC 3610, packet vector 1 (M = 8, L = 2, N = 13: TOWER's parameters), through the software
    /// AES: the CCM and its block cipher together.
    #[test]
    fn rfc3610_packet_vector_1() {
        let key: [u8; 16] = core::array::from_fn(|i| 0xC0 + i as u8);
        let nonce = [
            0x00, 0x00, 0x00, 0x03, 0x02, 0x01, 0x00, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5,
        ];
        let aad: [u8; 8] = core::array::from_fn(|i| i as u8);
        let mut data: [u8; 23] = core::array::from_fn(|i| 0x08 + i as u8);
        let tag = Ccm::new(SoftAes).seal(&key, &nonce, &aad, &mut data);
        assert_eq!(
            data,
            [
                0x58, 0x8C, 0x97, 0x9A, 0x61, 0xC6, 0x63, 0xD2, 0xF0, 0x66, 0xD0, 0xC2, 0xC0, 0xF9, 0x89,
                0x80, 0x6D, 0x5F, 0x6B, 0x61, 0xDA, 0xC3, 0x84
            ]
        );
        assert_eq!(tag, [0x17, 0xE8, 0xD1, 0x2C, 0xFD, 0xF9, 0x26, 0xE0]);
    }

    #[test]
    fn sealed_and_opened() {
        let mut ccm = Ccm::new(SoftAes);
        let mut frame = [0u8; MAX_FRAME];
        let n = seal_frame(&mut ccm, &KEY, &data(7), b"msg 007\0", &mut frame).unwrap();
        assert_eq!(n, HDR_LEN + 8 + TAG_LEN);
        // The header goes clear, the payload not.
        assert_eq!(&frame[..2], &[0x20, flags::CONFIRMED]);
        assert_ne!(&frame[HDR_LEN..HDR_LEN + 8], b"msg 007\0");
        let (header, payload) = open_frame(&mut ccm, &KEY, &mut frame[..n]).unwrap();
        assert_eq!(header, data(7));
        assert_eq!(&frame[payload], b"msg 007\0");
    }

    #[test]
    fn a_frame_changed_or_under_another_key_is_refused() {
        let mut ccm = Ccm::new(SoftAes);
        let mut frame = [0u8; MAX_FRAME];
        let n = seal_frame(&mut ccm, &KEY, &data(7), b"msg 007\0", &mut frame).unwrap();
        for at in [1, 12, HDR_LEN + 3, n - 1] {
            let mut changed = frame;
            changed[at] ^= 0x01;
            assert_eq!(
                open_frame(&mut ccm, &KEY, &mut changed[..n]),
                Err(FrameError::AuthFail),
                "byte {at}"
            );
        }
        let mut other = KEY;
        other[0] ^= 1;
        assert_eq!(
            open_frame(&mut ccm, &other, &mut frame[..n]).map(|_| ()),
            Err(FrameError::AuthFail)
        );
    }

    #[test]
    fn what_is_not_a_frame() {
        let mut ccm = Ccm::new(SoftAes);
        assert_eq!(
            open_frame(&mut ccm, &KEY, &mut [0x20; 10]),
            Err(FrameError::TooShort)
        );
        let mut frame = [0u8; 30];
        frame[0] = 0x40; // format 2
        assert_eq!(open_frame(&mut ccm, &KEY, &mut frame), Err(FrameError::BadFormat));
        frame[0] = 0x20 | 0x1F; // type 31
        assert_eq!(open_frame(&mut ccm, &KEY, &mut frame), Err(FrameError::BadType));
        let mut out = [0u8; MAX_FRAME];
        assert_eq!(
            seal_frame(&mut ccm, &KEY, &data(1), &[0; MAX_PAYLOAD + 1], &mut out),
            Err(FrameError::PayloadTooLong)
        );
        let bulk = Header {
            frame_type: FrameType::BulkData,
            bulk_index: Some(0x0A0B0C),
            ..data(1)
        };
        let n = seal_frame(&mut ccm, &KEY, &bulk, &[1; MAX_BULK_PAYLOAD], &mut out).unwrap();
        assert_eq!(n, HDR_LEN_BULK + MAX_BULK_PAYLOAD + TAG_LEN);
        assert_eq!(&out[14..17], &[0x0C, 0x0B, 0x0A]);
        assert_eq!(open_frame(&mut ccm, &KEY, &mut out[..n]).unwrap().0, bulk);
    }
}
