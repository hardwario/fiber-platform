//! The north link, there and back, its largest frames, and what it refuses.

// Tests may unwrap (`clippy.toml`), but clippy sees that only inside `#[test]` functions, and this
// file has helpers too.
#![allow(clippy::unwrap_used)]

use fiber_protocol::north::{
    Answer, Band, Bytes, Delivery, Fault, Frames, Hello, JoinRefusal, JoinRefused, KeyedJoin, MAX_DATA,
    Message, Node, NodeAdd, NodeRef, Outcome, Paired, PairingOpen, Personality, Queue, Reset, Sent,
    SetPersonality, Settings, Status, TAG, Uplink,
};
use fiber_protocol::{Error, Header, MAX_INNER, MAX_WIRE, encode, south};

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

fn every_message() -> Vec<Message> {
    let data = Bytes::new(&[0xA5; MAX_DATA]).unwrap();
    vec![
        Message::Hello(Hello {
            version: [0, 1, 0],
            image_crc: 0x1234_5678,
            uid: [9; 12],
            reset: Reset::Pin,
            fault: Some(Fault {
                code: 2,
                address: 0x0800_4000,
            }),
            personalities: 1,
            active: Personality::TowerFsk,
            tx_floor: u32::MAX - 1,
        }),
        Message::Answer(Answer {
            seq: 65535,
            result: Outcome::NoEntropy,
        }),
        Message::Status(Status {
            uptime: 86_400,
            tx_floor: 7_048,
            can_send: true,
            duty_left_ms: 36_000,
            frames: Frames {
                taken_in: 1_000_000,
                repeated: 12,
                replayed: 3,
                not_authentic: 1,
                unknown_node: 2,
                not_for_us: u32::MAX,
            },
            rx_dropped: 3,
        }),
        Message::Uplink(Uplink {
            node: 0x1111_1111,
            counter: 0xB801,
            rssi: -127,
            confirmed: true,
            data,
        }),
        Message::Delivery(Delivery {
            node: 0x1111_1111,
            result: Sent::NotDelivered,
            ack_rssi: Some(-60),
        }),
        Message::Paired(Paired {
            node: 0x4444_4444,
            key: [0x5a; 16],
            sas: 999_999,
        }),
        Message::PairingClosed,
        Message::JoinRefused(JoinRefused {
            node: 0x4444_4444,
            why: JoinRefusal::KeyedStale {
                counter: 6,
                last_seen: 6,
            },
        }),
        Message::KeyedJoin(KeyedJoin { node: 0x1111_1111 }),
        Message::Node(Node {
            node: 0x1111_1111,
            last_seen: 0xB820,
            queued: true,
        }),
        Message::HelloRequest,
        Message::SetPersonality(SetPersonality {
            personality: Personality::TowerFsk,
        }),
        Message::Settings(Settings {
            address: 0x2222_2222,
            band: Band::Eu868,
            channel: 2,
            keyed_join: true,
            tx_floor: 6_024,
        }),
        Message::NodeAdd(NodeAdd {
            node: 0x1111_1111,
            key: [0x11; 16],
            last_seen: 0xB800,
        }),
        Message::NodeRemove(NodeRef { node: 0x1111_1111 }),
        Message::ListNodes,
        Message::Queue(Queue {
            node: 0x1111_1111,
            data,
        }),
        Message::Unqueue(NodeRef { node: 0x1111_1111 }),
        Message::PairingOpen(PairingOpen { seconds: 60 }),
        Message::PairingCancel,
        Message::StatusRequest,
    ]
}

#[test]
fn every_message_there_and_back() {
    for (seq, message) in every_message().into_iter().enumerate() {
        let seq = seq as u16 * 1000;
        assert_eq!(read(wire(&message, seq)), Ok((seq, message)), "{message:?}");
    }
}

#[test]
fn the_largest_frames_fit() {
    // A node's payload at its longest, with every number at its widest: inside the frame's
    // 128 bytes, and on the wire within MAX_WIRE.
    let widest = Message::Uplink(Uplink {
        node: u32::MAX,
        counter: u32::MAX,
        rssi: i16::MIN,
        confirmed: true,
        data: Bytes::new(&[0xff; MAX_DATA]).unwrap(),
    });
    assert!(wire(&widest, u16::MAX).len() <= MAX_WIRE);
    let queue = Message::Queue(Queue {
        node: u32::MAX,
        data: Bytes::new(&[0xff; MAX_DATA]).unwrap(),
    });
    assert!(wire(&queue, u16::MAX).len() <= MAX_WIRE);
}

#[test]
fn a_payload_longer_than_a_frame_is_not_made_or_read() {
    assert!(Bytes::new(&[0; MAX_DATA + 1]).is_none());
    assert_eq!(Bytes::new(b"set").unwrap().as_slice(), b"set");
    // Written by hand: a queue message with 75 bytes in it.
    #[derive(serde::Serialize)]
    struct Long<'a> {
        node: u32,
        #[serde(with = "serde_bytes_like")]
        data: &'a [u8],
    }
    mod serde_bytes_like {
        pub fn serialize<S: serde::Serializer>(v: &&[u8], s: S) -> Result<S::Ok, S::Error> {
            s.serialize_bytes(v)
        }
    }
    let long = Long {
        node: 1,
        data: &[0; MAX_DATA + 1],
    };
    let header = Header {
        msg_type: 0x16,
        tag: TAG,
        seq: 1,
    };
    let mut out = [0u8; MAX_INNER * 2];
    let n = encode(header, &long, &mut out).unwrap();
    assert_eq!(read(out[..n].to_vec()), Err(Error::Malformed));
}

#[test]
fn the_two_links_refuse_each_other() {
    assert_ne!(TAG, south::TAG);
    let south_frame = {
        let mut out = [0u8; MAX_WIRE];
        let n = south::Message::HelloRequest.encode(1, &mut out).unwrap();
        out[..n].to_vec()
    };
    assert_eq!(read(south_frame), Err(Error::SchemaMismatch { tag: south::TAG }));
    let mut north_frame = wire(&Message::HelloRequest, 1);
    north_frame.pop();
    assert_eq!(
        south::Message::decode(&mut north_frame),
        Err(Error::SchemaMismatch { tag: TAG })
    );
}

/// The tag pinned: it moves only when a message's shape does, and then this test says so.
#[test]
fn the_tag() {
    assert_eq!(
        TAG, TAG_PINNED,
        "the north link's shapes changed: the firmware and the agent must be built again"
    );
}

const TAG_PINNED: u16 = 0xeef8;
