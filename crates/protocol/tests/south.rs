//! The south link, there and back, and every way a frame is refused.

// Tests may unwrap (`clippy.toml`), but clippy sees that only inside `#[test]` functions, and this
// file has helpers too.
#![allow(clippy::unwrap_used)]

use fiber_protocol::south::{Desired, Fault, Hello, LED_LINES, Light, Message, Reset, Status, TAG};
use fiber_protocol::{Error, FrameDecoder, Header, MAX_WIRE, crc, encode};

fn hello() -> Hello {
    Hello {
        version: [0, 2, 1],
        image_crc: 0x6f1c_2a90,
        uid: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        reset: Reset::Watchdog,
        fault: Some(Fault {
            code: 1,
            address: 0x0800_1f3c,
        }),
        option_bytes: 0xdeff_e1aa,
    }
}

fn desired() -> Desired {
    let mut leds = [Light::Off; LED_LINES];
    leds[0] = Light::On;
    leds[3] = Light::Blink {
        on: 5,
        off: 45,
        phase: 0,
    };
    Desired {
        leds,
        ports: 0b1111_0111,
    }
}

fn status() -> Status {
    Status {
        applied: 41,
        ports: 0xff,
        vbat: 2560,
        vin: 2481,
        vref: 1500,
        uptime: 3600,
        rx_dropped: 2,
    }
}

fn wire(message: &Message, seq: u16) -> Vec<u8> {
    let mut out = [0u8; MAX_WIRE];
    let n = message.encode(seq, &mut out).unwrap();
    out[..n].to_vec()
}

/// The frame's bytes without its `0x00`, as the decoder hands them over.
fn read(mut bytes: Vec<u8>) -> Result<(u16, Message), Error> {
    assert_eq!(bytes.pop(), Some(0));
    Message::decode(&mut bytes)
}

#[test]
fn every_message_there_and_back() {
    for (seq, message) in [
        (0, Message::Hello(hello())),
        (1, Message::Status(status())),
        (65535, Message::Desired(desired())),
        (7, Message::HelloRequest),
    ] {
        assert_eq!(read(wire(&message, seq)), Ok((seq, message)));
    }
}

#[test]
fn the_largest_messages_fit() {
    let leds = [Light::Blink {
        on: 255,
        off: 255,
        phase: 255,
    }; LED_LINES];
    let biggest = Message::Desired(Desired { leds, ports: 0xff });
    assert!(wire(&biggest, 1).len() <= MAX_WIRE);
    let mut hello = hello();
    hello.image_crc = u32::MAX;
    hello.option_bytes = u32::MAX;
    hello.fault = Some(Fault {
        code: 255,
        address: u32::MAX,
    });
    assert!(wire(&Message::Hello(hello), 1).len() <= MAX_WIRE);
}

/// The inner frame of a message: header, payload, CRC, without the COBS.
fn inner(message: &Message) -> Vec<u8> {
    let mut bytes = wire(message, 9);
    bytes.pop();
    let n = cobs::decode_in_place(&mut bytes).unwrap();
    bytes.truncate(n);
    bytes
}

/// An inner frame back on the wire, its CRC worked out again when asked.
fn reframe(mut inner: Vec<u8>, fix_crc: bool) -> Vec<u8> {
    if fix_crc {
        let n = inner.len() - 4;
        let crc = crc::crc32(&inner[..n]);
        inner[n..].copy_from_slice(&crc.to_le_bytes());
    }
    let mut out = vec![0u8; cobs::max_encoding_length(inner.len()) + 1];
    let n = cobs::encode(&inner, &mut out);
    out.truncate(n);
    out.push(0);
    out
}

#[test]
fn a_damaged_frame_is_refused() {
    let mut bytes = inner(&Message::Status(status()));
    bytes[7] ^= 0x10;
    assert_eq!(read(reframe(bytes, false)), Err(Error::BadCrc));
}

#[test]
fn a_frame_of_another_build_is_refused() {
    let mut out = [0u8; MAX_WIRE];
    let n = encode(
        Header {
            msg_type: 0x10,
            tag: TAG ^ 0x0101,
            seq: 3,
        },
        &desired(),
        &mut out,
    )
    .unwrap();
    assert_eq!(
        read(out[..n].to_vec()),
        Err(Error::SchemaMismatch { tag: TAG ^ 0x0101 })
    );
}

#[test]
fn an_unknown_type_is_refused() {
    let mut bytes = inner(&Message::HelloRequest);
    bytes[0] = 0x7f;
    assert_eq!(read(reframe(bytes, true)), Err(Error::UnknownType(0x7f)));
}

#[test]
fn bytes_left_over_are_refused() {
    let mut bytes = inner(&Message::Status(status()));
    let crc_at = bytes.len() - 4;
    bytes.insert(crc_at, 0x33);
    assert_eq!(read(reframe(bytes, true)), Err(Error::Malformed));
    // And a payload cut short.
    let mut bytes = inner(&Message::Hello(hello()));
    let crc_at = bytes.len() - 4;
    bytes.remove(crc_at - 1);
    assert_eq!(read(reframe(bytes, true)), Err(Error::Malformed));
}

#[test]
fn too_short_is_refused() {
    assert_eq!(Message::decode(&mut [0x02, 0x01]), Err(Error::Framing));
}

#[test]
fn bytes_as_they_come() {
    let mut stream = vec![0x55, 0xaa, 0x00, 0x00]; // noise, then an empty frame
    stream.extend(wire(&Message::Status(status()), 1));
    stream.extend([0x01; 300]); // a frame far too long
    stream.push(0);
    stream.extend(wire(&Message::Hello(hello()), 2));
    let mut decoder = FrameDecoder::<MAX_WIRE>::new();
    let mut got = Vec::new();
    for byte in stream {
        if let Some(frame) = decoder.push(byte) {
            got.push(Message::decode(frame));
        }
    }
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!(got[0], Err(Error::Framing)); // the noise
    assert_eq!(got[1], Ok((1, Message::Status(status()))));
    assert_eq!(got[2], Ok((2, Message::Hello(hello()))));
}

/// A few frames kept as bytes (seq 0x0102). A change to the messages changes them, and the tag
/// in them: then this test is changed with the firmware and the agent, in the same commit.
#[test]
fn frames_kept_as_bytes() {
    let hex =
        |message: &Message| -> String { wire(message, 0x0102).iter().map(|b| format!("{b:02x}")).collect() };
    assert_eq!(TAG, 0x90d6);
    assert_eq!(
        hex(&Message::Hello(hello())),
        "0601d690020124020190d5f0f8060102030405060708090a0b0c020101bcbe8040aac3fff70d87491c3c00"
    );
    assert_eq!(
        hex(&Message::Status(status())),
        "1502d690020129ff8014b113dc0b901c02c678db7800"
    );
    assert_eq!(
        hex(&Message::Desired(desired())),
        "0710d690020101010402052d010101010101010101010101010106f78c154a6800"
    );
    assert_eq!(hex(&Message::HelloRequest), "0a11d69002018e45bf2000");
}
