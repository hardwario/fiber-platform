//! The packet as a TOWER node's SPIRIT1 sends it, made and taken apart in software: the STM32WL's
//! packet engine cannot whiten it this way (`docs/firmware/radio.md`). Settled from captures of a
//! Core Module (`tests/radio-beacon-2026-11-19.txt`).
//!
//! On air, after the preamble and [`SYNC_WORD`]: the length of the payload (one byte), the
//! payload, and a CRC-16/CCITT-FALSE over both (high byte first), all three whitened with PN9 from
//! all ones, each byte filled from its least significant bit.

/// The sync word as it goes on air: TOWER's `SYNC4..SYNC1` (`0xDB624715`) byte-reversed, as the
/// SPIRIT1 sends `SYNC1` first.
pub const SYNC_WORD: [u8; 4] = [0x15, 0x47, 0x62, 0xDB];

/// The bytes a packet has around its payload: the length byte before it, the CRC after.
pub const OVERHEAD: usize = 3;

/// The longest payload the length byte can say.
pub const MAX_PAYLOAD: usize = 255;

/// The longest packet: [`MAX_PAYLOAD`] and the [`OVERHEAD`].
pub const MAX_PACKET: usize = MAX_PAYLOAD + OVERHEAD;

/// Why a packet was not made or not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A payload longer than [`MAX_PAYLOAD`], or a buffer too small for the packet.
    TooLong,
    /// Fewer bytes than the length byte says.
    TooShort,
    /// The CRC does not match: the packet was damaged on the way.
    Crc,
}

/// PN9 (x^9 + x^5 + 1) from all ones, one byte at a time, each filled from its least
/// significant bit.
struct Pn9(u16);

impl Pn9 {
    fn new() -> Self {
        Pn9(0x1FF)
    }

    fn next_byte(&mut self) -> u8 {
        let mut byte = 0u8;
        for bit in 0..8 {
            byte |= ((self.0 & 1) as u8) << bit;
            let feedback = (self.0 ^ (self.0 >> 5)) & 1;
            self.0 = (self.0 >> 1) | (feedback << 8);
        }
        byte
    }
}

/// Whitens `data`, or takes the whitening off: the same XOR both ways.
fn whiten(data: &mut [u8]) {
    let mut pn9 = Pn9::new();
    for byte in data {
        *byte ^= pn9.next_byte();
    }
}

/// CRC-16/CCITT-FALSE: polynomial 0x1021 from 0xFFFF, not reflected, nothing XORed at the end.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Makes the packet for `payload` in `out`, as it goes on air after the sync word; returns its
/// length (the payload's and [`OVERHEAD`]).
pub fn encode(payload: &[u8], out: &mut [u8]) -> Result<usize, Error> {
    let len = payload.len() + OVERHEAD;
    if payload.len() > MAX_PAYLOAD || out.len() < len {
        return Err(Error::TooLong);
    }
    out[0] = payload.len() as u8;
    out[1..1 + payload.len()].copy_from_slice(payload);
    let crc = crc16(&out[..1 + payload.len()]);
    out[1 + payload.len()..len].copy_from_slice(&crc.to_be_bytes());
    whiten(&mut out[..len]);
    Ok(len)
}

/// How many bytes in all the packet has whose first byte, as received, is `first`: the receiver
/// knows it one byte time after the sync word, and stops after that many.
pub fn packet_len(first: u8) -> usize {
    usize::from(first ^ Pn9::new().next_byte()) + OVERHEAD
}

/// Takes the whitening off `packet` (the bytes after the sync word, as received; more may follow)
/// and checks it; returns its payload.
pub fn decode(packet: &mut [u8]) -> Result<&[u8], Error> {
    let first = *packet.first().ok_or(Error::TooShort)?;
    let len = packet_len(first);
    if packet.len() < len {
        return Err(Error::TooShort);
    }
    let packet = &mut packet[..len];
    whiten(packet);
    let (body, crc) = packet.split_at(len - 2);
    if crc16(body) != u16::from_be_bytes([crc[0], crc[1]]) {
        return Err(Error::Crc);
    }
    Ok(&packet[1..len - 2])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `radio_beacon`'s frame number 142, as the north bridge received it after the sync word
    /// (the first of `tests/radio-beacon-2026-11-19.txt`).
    const CAPTURED: [u8; 19] = [
        0xef, 0x6f, 0x1d, 0x9a, 0xed, 0x20, 0x96, 0x81, 0x4f, 0xdf, 0x77, 0x9c, 0xd5, 0x32, 0xf2, 0xaf, 0xf1,
        0xb3, 0x44,
    ];

    fn beacon(seq: u32) -> [u8; 16] {
        let mut payload = [0xA5; 16];
        payload[..4].copy_from_slice(&seq.to_le_bytes());
        payload
    }

    #[test]
    fn the_known_answers() {
        let mut pn9 = Pn9::new();
        let first: [u8; 6] = core::array::from_fn(|_| pn9.next_byte());
        assert_eq!(first, [0xFF, 0xE1, 0x1D, 0x9A, 0xED, 0x85]);
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn a_packet_made_as_a_tower_node_makes_it() {
        let mut out = [0u8; MAX_PACKET];
        let len = encode(&beacon(142), &mut out).unwrap();
        assert_eq!(&out[..len], &CAPTURED);
    }

    #[test]
    fn a_captured_packet_taken_apart() {
        assert_eq!(packet_len(CAPTURED[0]), 19);
        // With bytes after it, as a receiver of a fixed length delivers them.
        let mut received = [0u8; 32];
        received[..19].copy_from_slice(&CAPTURED);
        received[19..].fill(0x5A);
        assert_eq!(decode(&mut received).unwrap(), &beacon(142));
    }

    #[test]
    fn damage_is_refused() {
        let mut damaged = CAPTURED;
        damaged[7] ^= 0x10;
        assert_eq!(decode(&mut damaged), Err(Error::Crc));
        let mut short = CAPTURED;
        assert_eq!(decode(&mut short[..18]), Err(Error::TooShort));
        assert_eq!(decode(&mut []), Err(Error::TooShort));
        let mut out = [0u8; 18];
        assert_eq!(encode(&beacon(1), &mut out), Err(Error::TooLong));
        assert_eq!(encode(&[0; 256], &mut [0; MAX_PACKET + 1]), Err(Error::TooLong));
    }

    #[test]
    fn there_and_back() {
        for len in [0, 1, 74, MAX_PAYLOAD] {
            let payload: [u8; MAX_PAYLOAD] = core::array::from_fn(|i| (i * 7) as u8);
            let mut out = [0u8; MAX_PACKET];
            let n = encode(&payload[..len], &mut out).unwrap();
            assert_eq!(n, len + OVERHEAD);
            assert_eq!(decode(&mut out[..n]).unwrap(), &payload[..len]);
        }
    }
}
