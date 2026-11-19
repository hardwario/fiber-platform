//! TOWER's network frames as a real Core Module sent them (`tests/net-confirmed-2026-11-19.txt`),
//! opened by the frame codec with the example's key: the vectors of T24.

use fiber_tower_air::frame::{Ccm, FrameType, SoftAes, flags, open_frame};

const CAPTURES: &str = include_str!("net-confirmed-2026-11-19.txt");

/// `net_confirmed`'s key (TOWER's `firmware/examples/net_confirmed.rs`).
const KEY: [u8; 16] = [
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00,
];

/// Each frame received: its bytes.
fn frames() -> Vec<Vec<u8>> {
    CAPTURES
        .lines()
        .filter_map(|line| {
            let hex = line.strip_prefix("rx ")?.split(": ").nth(1)?;
            let hex = hex.split(" (").next()?;
            hex.split(' ').map(|b| u8::from_str_radix(b, 16).ok()).collect()
        })
        .collect()
}

#[test]
fn a_real_node_s_frames_open_with_its_key() {
    let frames = frames();
    assert!(frames.len() >= 30, "{} frames", frames.len());
    let mut ccm = Ccm::new(SoftAes);
    let mut counters = Vec::new();
    for frame in &frames {
        let mut buf = frame.clone();
        let (header, payload) = open_frame(&mut ccm, &KEY, &mut buf).expect("the tag under the key");
        assert_eq!(header.frame_type, FrameType::Data);
        assert_eq!(header.flags, flags::CONFIRMED);
        assert_eq!((header.src, header.dest), (0x1111_1111, 0x2222_2222));
        // The nonce is the source, the counter and nothing else, little-endian.
        let mut nonce = [0u8; 13];
        nonce[..4].copy_from_slice(&0x1111_1111u32.to_le_bytes());
        nonce[4..8].copy_from_slice(&header.counter.to_le_bytes());
        assert_eq!(header.nonce(), nonce);
        let text = &buf[payload];
        assert_eq!(text.len(), 8);
        assert_eq!(&text[..4], b"msg ");
        assert!(
            text[4..7].iter().all(u8::is_ascii_digit) && text[7] == 0,
            "{text:?}"
        );
        counters.push(header.counter);
    }
    // A message goes up to 3 times, the same frame each time; the next has the next counter.
    counters.dedup();
    assert!(counters.windows(2).all(|w| w[1] == w[0] + 1), "{counters:?}");
    assert!(counters.len() >= 10, "{counters:?}");
}

#[test]
fn under_another_key_nothing_opens() {
    let mut ccm = Ccm::new(SoftAes);
    let mut other = KEY;
    other[15] ^= 0x80;
    for frame in frames() {
        let mut buf = frame;
        assert!(open_frame(&mut ccm, &other, &mut buf).is_err());
    }
}
