//! What a TOWER node sends, as the north bridge received it (`tests/radio-beacon-2026-11-19.txt`),
//! taken apart by the packet codec: every capture must pass it (T23). The captures settled the
//! air format (`docs/bench-checks.md`, check 4).

use fiber_tower_air::packet::{SYNC_WORD, decode, packet_len};

const CAPTURES: &str = include_str!("radio-beacon-2026-11-19.txt");

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

/// The bytes after the sync word, found at whatever bit it starts (the captures were taken
/// without a sync word, from the preamble on), most significant bit first.
fn after_sync(frame: &[u8]) -> Option<Vec<u8>> {
    let bits: Vec<u8> = frame
        .iter()
        .flat_map(|b| (0..8).rev().map(move |i| (b >> i) & 1))
        .collect();
    let sync: Vec<u8> = SYNC_WORD
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

#[test]
fn every_capture_passes_the_codec() {
    let frames = frames();
    // A frame received with a bit error in its sync word has none (one of the 57 strong ones).
    let synced: Vec<Vec<u8>> = frames.iter().filter_map(|f| after_sync(f)).collect();
    assert!(
        synced.len() >= 50 && frames.len() - synced.len() <= 2,
        "{} of {}",
        synced.len(),
        frames.len()
    );
    let mut sequence = Vec::new();
    for mut packet in synced {
        // The length byte says how many bytes follow: radio_beacon's 16, and the CRC.
        assert_eq!(packet_len(packet[0]), 19);
        let payload = decode(&mut packet).expect("the CRC and the length");
        assert_eq!(payload[4..], [0xA5; 12]);
        sequence.push(u32::from_le_bytes([
            payload[0], payload[1], payload[2], payload[3],
        ]));
    }
    // The beacon counts up by one a second; the capture missed at most a few.
    assert!(sequence.windows(2).all(|w| w[1] > w[0]), "{sequence:?}");
    let span = sequence[sequence.len() - 1] - sequence[0] + 1;
    assert!(span as usize - sequence.len() <= 3, "{sequence:?}");
}
