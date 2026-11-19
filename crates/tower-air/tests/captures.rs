//! What a TOWER node sends, as the north bridge received it (`tests/radio-beacon-2026-11-19.txt`):
//! the facts of TOWER's packet on air that the north bridge's codec (T23) must meet. Settled from
//! these captures (`docs/bench-checks.md`, check 4); the PN9 and the CRC here are the tests' own,
//! written from that description.

const CAPTURES: &str = include_str!("radio-beacon-2026-11-19.txt");

/// The sync word as it goes on air: TOWER's `SYNC4..SYNC1` (`0xDB624715`) byte-reversed.
const SYNC_ON_AIR: [u8; 4] = [0x15, 0x47, 0x62, 0xDB];

/// The frames of the beacon (the strong ones; the weaker are something else on the channel).
fn frames() -> Vec<Vec<u8>> {
    CAPTURES
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("frame rssi ")?;
            let rssi: i32 = rest.split(' ').next()?.parse().ok()?;
            let hex = rest.split(": ").nth(1)?;
            let bytes = hex
                .split(' ')
                .map(|b| u8::from_str_radix(b, 16).ok())
                .collect::<Option<Vec<u8>>>()?;
            (rssi >= -80).then_some(bytes)
        })
        .collect()
}

/// The bytes after the sync word, found at whatever bit it starts, most significant bit first.
fn after_sync(frame: &[u8]) -> Option<Vec<u8>> {
    let bits: Vec<u8> = frame
        .iter()
        .flat_map(|b| (0..8).rev().map(move |i| (b >> i) & 1))
        .collect();
    let sync: Vec<u8> = SYNC_ON_AIR
        .iter()
        .flat_map(|b| (0..8).rev().map(move |i| (b >> i) & 1))
        .collect();
    let start = bits.windows(32).position(|w| w == sync.as_slice())? + 32;
    let (bytes, _) = bits[start..].as_chunks::<8>();
    Some(
        bytes
            .iter()
            .map(|c| c.iter().fold(0, |byte, bit| (byte << 1) | bit))
            .collect(),
    )
}

/// PN9 (x^9 + x^5 + 1) from all ones, each byte filled from its least significant bit.
fn pn9(len: usize) -> Vec<u8> {
    let mut state: u16 = 0x1FF;
    (0..len)
        .map(|_| {
            let mut byte = 0u8;
            for bit in 0..8 {
                byte |= ((state & 1) as u8) << bit;
                let feedback = (state ^ (state >> 5)) & 1;
                state = (state >> 1) | (feedback << 8);
            }
            byte
        })
        .collect()
}

/// CRC-16/CCITT-FALSE: polynomial 0x1021 from 0xFFFF, not reflected, nothing XORed at the end.
fn crc16(data: &[u8]) -> u16 {
    data.iter().fold(0xFFFF, |crc, &byte| {
        (0..8).fold(crc ^ (u16::from(byte) << 8), |crc, _| {
            if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            }
        })
    })
}

#[test]
fn the_reference_pn9_and_crc() {
    assert_eq!(pn9(6), [0xFF, 0xE1, 0x1D, 0x9A, 0xED, 0x85]);
    assert_eq!(crc16(b"123456789"), 0x29B1);
}

#[test]
fn a_tower_frame_on_air() {
    let frames = frames();
    // The sync word, byte-reversed against TOWER's register order. A frame received with a bit
    // error in it does not have it (one of the 57 strong ones).
    let synced: Vec<Vec<u8>> = frames.iter().filter_map(|f| after_sync(f)).collect();
    assert!(
        synced.len() >= 50 && frames.len() - synced.len() <= 2,
        "{} of {}",
        synced.len(),
        frames.len()
    );
    let mut sequence = Vec::new();
    for whitened in &synced {
        // Whitened from the length byte on.
        let plain: Vec<u8> = whitened
            .iter()
            .zip(pn9(whitened.len()))
            .map(|(a, b)| a ^ b)
            .collect();
        // The length byte counts the payload only.
        let length = usize::from(plain[0]);
        assert_eq!(length, 16);
        let payload = &plain[1..1 + length];
        assert_eq!(payload[4..], [0xA5; 12]);
        // The CRC over the length byte and the payload, high byte first, whitened too.
        let crc = u16::from_be_bytes([plain[1 + length], plain[2 + length]]);
        assert_eq!(crc, crc16(&plain[..1 + length]));
        sequence.push(u32::from_le_bytes([
            payload[0], payload[1], payload[2], payload[3],
        ]));
    }
    // The beacon counts up by one a second; the capture missed at most a few.
    assert!(sequence.windows(2).all(|w| w[1] > w[0]), "{sequence:?}");
    let span = sequence[sequence.len() - 1] - sequence[0] + 1;
    assert!(span as usize - sequence.len() <= 3, "{sequence:?}");
}
