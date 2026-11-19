//! The north link in the agent (`docs/firmware/north-link.md`): a task of its own owns the port,
//! takes the frames apart, and sends the agent's messages one at a time, each again when its
//! answer does not come within 200 ms. The radio service ([`super::radio`]) sees only whole
//! messages, and [`Event::Lost`] when the north bridge falls silent.
//!
//! [`simulate`] is a north bridge made up in software, for the tests.

use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use fiber_protocol::north::{Answer, Hello, Message, Node, Outcome, Personality, Reset, Status};
use fiber_protocol::{Error, FrameDecoder, MAX_WIRE};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::Instant;

/// How long a message's answer may take.
pub const ANSWER_WAIT: Duration = Duration::from_millis(200);
/// A message is sent this many times in all.
pub const TRIES: u32 = 3;
/// No frame for this long (a `Status` comes every 10 s): the north bridge has stopped.
pub const SILENT_AFTER: Duration = Duration::from_secs(30);
/// How long to wait before opening the port again.
const REOPEN: Duration = Duration::from_secs(5);

/// What the link tells the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Message(Message),
    /// The north bridge stopped answering, or the port broke: what it held is gone.
    Lost,
    /// A whole frame with another schema tag: a firmware from another build.
    OtherSchema,
}

/// The agent's end of the link.
pub struct North {
    pub to: mpsc::UnboundedSender<Message>,
    pub from: mpsc::Receiver<Event>,
}

/// The link on the serial port `port`, opened again whenever it breaks.
pub fn spawn(port: String) -> North {
    let (to, mut outgoing) = mpsc::unbounded_channel();
    let (events, from) = mpsc::channel(64);
    tokio::spawn(async move {
        use tokio_serial::SerialPortBuilderExt;
        let mut said: Option<String> = None;
        loop {
            let result = match tokio_serial::new(&port, 115_200).open_native_async() {
                Ok(stream) => {
                    said = None;
                    serve(stream, &mut outgoing, &events).await
                }
                Err(e) => Err(io::Error::other(format!("cannot open {port}: {e}"))),
            };
            if let Err(e) = result {
                let message = format!("the north link: {e}");
                if said.as_deref() != Some(&message) {
                    eprintln!("{message}");
                    said = Some(message);
                }
            }
            if events.send(Event::Lost).await.is_err() || outgoing.is_closed() {
                return;
            }
            tokio::time::sleep(REOPEN).await;
        }
    });
    North { to, from }
}

/// One connection, until the stream breaks, the north bridge falls silent, or the agent's end is
/// dropped (`Ok`). What the agent sent before it began is dropped: the north bridge's `Hello`
/// makes the agent send everything again.
pub async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    outgoing: &mut mpsc::UnboundedReceiver<Message>,
    events: &mpsc::Sender<Event>,
) -> io::Result<()> {
    while outgoing.try_recv().is_ok() {}
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut seq = 0u16;
    write(&mut writer, &Message::HelloRequest, &mut seq).await?;
    let mut decoder = FrameDecoder::<MAX_WIRE>::new();
    let mut buf = [0u8; 64];
    let mut queue: VecDeque<Message> = VecDeque::new();
    // The message waiting for its answer: its seq, itself, how often sent, until when.
    let mut in_flight: Option<(u16, Message, u32, Instant)> = None;
    let mut heard = Instant::now();
    loop {
        if in_flight.is_none()
            && let Some(message) = queue.pop_front()
        {
            let sent = write(&mut writer, &message, &mut seq).await?;
            in_flight = Some((sent, message, 1, Instant::now() + ANSWER_WAIT));
        }
        let answer_by = in_flight.map(|(_, _, _, by)| by);
        enum Step {
            Read(usize),
            Out(Option<Message>),
            NoAnswer,
            Silent,
        }
        let step = tokio::select! {
            n = reader.read(&mut buf) => Step::Read(n?),
            m = outgoing.recv() => Step::Out(m),
            () = async {
                match answer_by {
                    Some(by) => tokio::time::sleep_until(by).await,
                    None => std::future::pending().await,
                }
            } => Step::NoAnswer,
            () = tokio::time::sleep_until(heard + SILENT_AFTER) => Step::Silent,
        };
        match step {
            Step::Read(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Step::Read(n) => {
                for &byte in &buf[..n] {
                    let Some(frame) = decoder.push(byte) else { continue };
                    let event = match Message::decode(frame) {
                        Ok((_, message)) => {
                            heard = Instant::now();
                            match message {
                                Message::Answer(a) if in_flight.is_some_and(|(s, ..)| s == a.seq) => {
                                    in_flight = None;
                                }
                                // It started again: whatever was on its way is out of date.
                                Message::Hello(_) => {
                                    queue.clear();
                                    in_flight = None;
                                }
                                _ => {}
                            }
                            Event::Message(message)
                        }
                        Err(Error::SchemaMismatch { .. }) => Event::OtherSchema,
                        Err(_) => continue,
                    };
                    if events.send(event).await.is_err() {
                        return Ok(());
                    }
                }
            }
            Step::Out(Some(message)) => queue.push_back(message),
            Step::Out(None) => return Ok(()),
            Step::NoAnswer => match in_flight {
                Some((_, message, tries, _)) if tries < TRIES => {
                    let sent = write(&mut writer, &message, &mut seq).await?;
                    in_flight = Some((sent, message, tries + 1, Instant::now() + ANSWER_WAIT));
                }
                _ => return Err(io::Error::other(format!("no answer after {TRIES} tries"))),
            },
            Step::Silent => return Err(io::Error::other("the north bridge is silent")),
        }
    }
}

async fn write<W: AsyncWrite + Unpin>(writer: &mut W, message: &Message, seq: &mut u16) -> io::Result<u16> {
    let mut wire = [0u8; MAX_WIRE];
    let n = message
        .encode(*seq, &mut wire)
        .map_err(|e| io::Error::other(format!("{e:?}")))?;
    writer.write_all(&wire[..n]).await?;
    writer.flush().await?;
    let sent = *seq;
    *seq = seq.wrapping_add(1);
    Ok(sent)
}

/// A north bridge made up in software, for the tests: it says `Hello`, answers every message
/// `Done` (`Unknown` for a node it does not have), keeps the nodes it is given, lists them, sends
/// a `Status` every 10 s, and sends on what `inject` gives it (an `Uplink`, a `Paired`, ...).
/// `silent` makes it answer nothing.
pub async fn simulate<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    mut inject: mpsc::UnboundedReceiver<Message>,
    seen: mpsc::UnboundedSender<Message>,
    silent: bool,
) -> io::Result<()> {
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut seq = 0u16;
    let mut decoder = FrameDecoder::<MAX_WIRE>::new();
    let mut buf = [0u8; 64];
    let mut nodes: Vec<(u32, u32)> = Vec::new();
    let hello = Message::Hello(Hello {
        version: [0, 1, 0],
        image_crc: 0x0bad_cafe,
        uid: [7; 12],
        reset: Reset::PowerOn,
        fault: None,
        personalities: 1,
        active: Personality::TowerFsk,
        tx_floor: 2049,
    });
    if !silent {
        write(&mut writer, &hello, &mut seq).await?;
    }
    let mut status = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            n = reader.read(&mut buf) => {
                let n = n?;
                if n == 0 {
                    return Ok(());
                }
                for &byte in &buf[..n] {
                    let Some(frame) = decoder.push(byte) else { continue };
                    let Ok((their_seq, message)) = Message::decode(frame) else { continue };
                    let _ = seen.send(message);
                    if silent {
                        continue;
                    }
                    let result = match message {
                        Message::HelloRequest => {
                            write(&mut writer, &hello, &mut seq).await?;
                            continue;
                        }
                        Message::NodeAdd(m) => {
                            nodes.retain(|(a, _)| *a != m.node);
                            nodes.push((m.node, m.last_seen));
                            Outcome::Done
                        }
                        Message::NodeRemove(m) if nodes.iter().any(|(a, _)| *a == m.node) => {
                            nodes.retain(|(a, _)| *a != m.node);
                            Outcome::Done
                        }
                        Message::NodeRemove(_) => Outcome::Unknown,
                        Message::ListNodes => {
                            for &(node, last_seen) in &nodes {
                                let listed = Message::Node(Node { node, last_seen, queued: false });
                                write(&mut writer, &listed, &mut seq).await?;
                            }
                            Outcome::Done
                        }
                        _ => Outcome::Done,
                    };
                    let answer = Message::Answer(Answer { seq: their_seq, result });
                    write(&mut writer, &answer, &mut seq).await?;
                }
            }
            injected = inject.recv() => match injected {
                Some(message) => {
                    write(&mut writer, &message, &mut seq).await?;
                }
                None => return Ok(()),
            },
            _ = status.tick(), if !silent => {
                let s = Message::Status(Status { tx_floor: 2049, can_send: true, duty_left_ms: 36_000, ..Status::default() });
                write(&mut writer, &s, &mut seq).await?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fiber_protocol::north::{Band, NodeAdd, NodeRef, Settings, Uplink};

    struct Bench {
        to: mpsc::UnboundedSender<Message>,
        events: mpsc::Receiver<Event>,
        inject: mpsc::UnboundedSender<Message>,
        seen: mpsc::UnboundedReceiver<Message>,
        link: tokio::task::JoinHandle<io::Result<()>>,
    }

    fn bench(silent: bool) -> Bench {
        let (agent_side, north_side) = tokio::io::duplex(1024);
        let (inject, inject_rx) = mpsc::unbounded_channel();
        let (seen_tx, seen) = mpsc::unbounded_channel();
        tokio::spawn(simulate(north_side, inject_rx, seen_tx, silent));
        let (to, mut outgoing) = mpsc::unbounded_channel();
        let (events_tx, events) = mpsc::channel(16);
        let link = tokio::spawn(async move { serve(agent_side, &mut outgoing, &events_tx).await });
        Bench {
            to,
            events,
            inject,
            seen,
            link,
        }
    }

    async fn next_message(events: &mut mpsc::Receiver<Event>) -> Message {
        loop {
            match events.recv().await {
                Some(Event::Message(Message::Status(_))) => continue,
                Some(Event::Message(m)) => return m,
                other => panic!("{other:?}"),
            }
        }
    }

    fn settings() -> Message {
        Message::Settings(Settings {
            address: 0x2a7f_03c1,
            band: Band::Eu868,
            channel: 0,
            keyed_join: true,
            tx_floor: 0,
        })
    }

    #[tokio::test(start_paused = true)]
    async fn one_message_at_a_time_each_answered() {
        let mut b = bench(false);
        // Its Hello at start, and again for the HelloRequest the link sends.
        assert!(matches!(next_message(&mut b.events).await, Message::Hello(_)));
        assert!(matches!(next_message(&mut b.events).await, Message::Hello(_)));
        b.to.send(settings()).unwrap();
        b.to.send(Message::NodeAdd(NodeAdd {
            node: 0x1111_1111,
            key: [0x11; 16],
            last_seen: 100,
        }))
        .unwrap();
        b.to.send(Message::NodeRemove(NodeRef { node: 0x9999_9999 }))
            .unwrap();
        let answers: Vec<Outcome> = [
            next_message(&mut b.events).await,
            next_message(&mut b.events).await,
            next_message(&mut b.events).await,
        ]
        .iter()
        .map(|m| match m {
            Message::Answer(a) => a.result,
            other => panic!("{other:?}"),
        })
        .collect();
        assert_eq!(answers, [Outcome::Done, Outcome::Done, Outcome::Unknown]);
        // What the north bridge got, in order, each once.
        let mut got = Vec::new();
        while let Ok(m) = b.seen.try_recv() {
            got.push(m);
        }
        assert!(matches!(got[0], Message::HelloRequest));
        assert_eq!(got[1], settings());
        assert!(matches!(got[2], Message::NodeAdd(_)));
        assert!(matches!(got[3], Message::NodeRemove(_)));
        // What it sends on its own comes up as it comes.
        let uplink = Message::Uplink(Uplink {
            node: 0x1111_1111,
            counter: 101,
            rssi: -70,
            confirmed: true,
            data: fiber_protocol::north::Bytes::new(b"x").unwrap(),
        });
        b.inject.send(uplink).unwrap();
        assert_eq!(next_message(&mut b.events).await, uplink);
    }

    #[tokio::test(start_paused = true)]
    async fn sent_again_and_then_given_up() {
        let mut b = bench(true);
        // Once the link has begun (what was queued before is dropped).
        assert_eq!(b.seen.recv().await, Some(Message::HelloRequest));
        b.to.send(settings()).unwrap();
        // Nothing answers: the HelloRequest, then the settings three times, 200 ms apart, then
        // the link gives up.
        let result = b.link.await.unwrap();
        assert!(result.is_err());
        let mut got = Vec::new();
        while let Ok(m) = b.seen.try_recv() {
            got.push(m);
        }
        assert_eq!(got, vec![settings(); TRIES as usize]);
        assert!(b.events.try_recv().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_north_bridge_is_lost() {
        let (agent_side, _north_side) = tokio::io::duplex(1024);
        let (_to, mut outgoing) = mpsc::unbounded_channel();
        let (events, _rx) = mpsc::channel(16);
        let started = Instant::now();
        let result = serve(agent_side, &mut outgoing, &events).await;
        assert!(result.is_err());
        assert_eq!(started.elapsed(), SILENT_AFTER);
    }
}
