//! The agent's loop: the state in the middle, the south link and MQTT around it.
//!
//! Every second it sends the south bridge a `Desired`; what comes back goes into the state; what
//! changed goes out on MQTT; requests from MQTT change the state and are answered. MQTT is behind
//! [`Bus`], so the board keeps working while the broker is down, and the tests drive the loop
//! without one.

use std::future::Future;
use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fiber_protocol::south::Message;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;

use super::link::{Event, Reader, Writer};
use super::requests::{self, Answer, Handled, RecentIds};
use super::state::State;
use super::topics::{self, Identity};

/// The prefix of every topic.
pub const PREFIX: &str = "fiber/";
/// Without a `Status` for this long, the south bridge counts as silent.
pub const SILENT_AFTER: Duration = Duration::from_secs(3);
/// `power` is sent again at least this often.
pub const POWER_EVERY: Duration = Duration::from_secs(60);

/// What comes from the broker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BusEvent {
    /// (Re)connected: every retained topic is sent again.
    Connected,
    /// A request on `fiber/cmd`.
    Request(Vec<u8>),
}

/// MQTT, as the loop sees it.
pub trait Bus {
    /// Publishes, QoS 1. A publish while the broker is down is dropped by the adapter.
    fn publish(&mut self, topic: String, payload: String, retain: bool) -> impl Future<Output = ()>;
    fn next(&mut self) -> impl Future<Output = BusEvent>;
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The agent: its state, its bus, and what it has published.
pub struct Agent<B: Bus> {
    pub state: State,
    pub bus: B,
    identity: Identity,
    ids: RecentIds,
    /// Ports switched off by a cycle: on again at the deadline, then the answer.
    cycles: Vec<(Instant, u8, String)>,
    last_status: Option<Instant>,
    started: Instant,
    /// The payloads last sent (without `ts`), to send only what changed.
    sent: std::collections::BTreeMap<String, Value>,
    power_sent: Option<Instant>,
}

/// A payload without its `ts`, to compare.
fn without_ts(mut value: Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        map.remove("ts");
    }
    value
}

impl<B: Bus> Agent<B> {
    pub fn new(state: State, bus: B, identity: Identity) -> Self {
        Agent {
            state,
            bus,
            identity,
            ids: RecentIds::default(),
            cycles: Vec::new(),
            last_status: None,
            started: Instant::now(),
            sent: Default::default(),
            power_sent: None,
        }
    }

    /// Sends a retained topic when its payload changed, or always with `force`.
    async fn retained(&mut self, topic: &str, payload: Value, force: bool) {
        let compare = without_ts(payload.clone());
        if !force && self.sent.get(topic) == Some(&compare) {
            return;
        }
        self.sent.insert(topic.to_owned(), compare);
        self.bus
            .publish(format!("{PREFIX}{topic}"), payload.to_string(), true)
            .await;
    }

    /// Every retained topic, as after a reconnect (`force`), or what changed.
    async fn publish_state(&mut self, force: bool) {
        let ts = now();
        if force {
            self.retained("status", topics::status(&self.identity, ts), true)
                .await;
        }
        for port in 1..=8u8 {
            self.retained(
                &format!("ports/{port}"),
                topics::port(&self.state, port, ts),
                force,
            )
            .await;
        }
        self.retained("firmware/south", topics::firmware(&self.state.south, ts), force)
            .await;
        let stale = self.power_sent.is_none_or(|at| at.elapsed() >= POWER_EVERY);
        let power = topics::power(&self.state, ts);
        let moved = self.sent.get("power").is_none_or(|before| {
            let v = |p: &Value, k: &str| p[k].as_f64();
            before["source"] != power["source"]
                || match (
                    v(before, "input"),
                    v(&power, "input"),
                    v(before, "battery"),
                    v(&power, "battery"),
                ) {
                    (Some(a), Some(b), Some(c), Some(d)) => (a - b).abs() > 0.05 || (c - d).abs() > 0.05,
                    _ => before["input"] != power["input"],
                }
        });
        if force || stale || moved {
            self.power_sent = Some(Instant::now());
            self.retained("power", power, true).await;
        }
    }

    async fn answer(&mut self, answer: Answer) {
        self.bus
            .publish(format!("{PREFIX}rsp/{}", answer.id), answer.payload(), false)
            .await;
    }

    /// Once a second: cycles that are due, silence, the next `Desired`.
    async fn tick<W: AsyncWrite + Unpin>(&mut self, writer: &mut Writer<W>) -> io::Result<()> {
        let now = Instant::now();
        let due: Vec<_> = self
            .cycles
            .iter()
            .filter(|(at, _, _)| *at <= now)
            .cloned()
            .collect();
        self.cycles.retain(|(at, _, _)| *at > now);
        for (_, mask, id) in due {
            self.state.ports |= mask;
            let data = requests::ports_data(&self.state);
            self.answer(Answer::ok(&id, Some(data))).await;
        }
        let last = self.last_status.unwrap_or(self.started);
        if last.elapsed() > SILENT_AFTER {
            self.state.south_silent();
        }
        writer.send(&Message::Desired(self.state.desired())).await?;
        self.publish_state(false).await;
        Ok(())
    }

    async fn link(&mut self, event: Event) {
        match event {
            Event::Hello(hello) => self.state.south_hello(hello),
            Event::Status { status, .. } => {
                self.last_status = Some(Instant::now());
                if let Some(source) = self.state.south_status(status) {
                    let payload = topics::power_event(source.name(), now()).to_string();
                    self.bus
                        .publish(format!("{PREFIX}power/event"), payload, false)
                        .await;
                }
            }
            // A firmware from another build, or noise: flashing it comes with the flasher's
            // adapters (T17).
            Event::OtherSchema | Event::Dropped => {}
        }
        self.publish_state(false).await;
    }

    async fn request(&mut self, payload: &[u8]) {
        let Ok(request) = requests::parse(payload) else {
            return;
        };
        if !self.ids.is_new(&request.id) {
            return;
        }
        match requests::handle(&mut self.state, &request) {
            Handled::Answer(answer) => self.answer(answer).await,
            Handled::Cycle { id, mask, seconds } => {
                self.cycles
                    .push((Instant::now() + Duration::from_secs(seconds), mask, id));
            }
            Handled::Flash { id, .. } => {
                self.answer(Answer::error(
                    &id,
                    "flashing through the agent is not written yet",
                ))
                .await
            }
        }
        self.publish_state(false).await;
    }
}

/// Runs the agent on a south link until the link ends or breaks.
pub async fn run<R, W, B>(agent: &mut Agent<B>, reader: R, writer: W) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    B: Bus,
{
    let mut reader = Reader::new(reader);
    let mut writer = Writer::new(writer);
    writer.send(&Message::HelloRequest).await?;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = tick.tick() => agent.tick(&mut writer).await?,
            event = reader.next() => agent.link(event?).await,
            event = agent.bus.next() => match event {
                BusEvent::Connected => agent.publish_state(true).await,
                BusEvent::Request(payload) => agent.request(&payload).await,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::Settings;
    use fiber_protocol::south::{Desired, Hello, Reset, Status};
    use fiber_protocol::{FrameDecoder, MAX_WIRE};
    use fiber_south_core::Engine;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
    use tokio::sync::mpsc;

    const CRC: u32 = 0x6f1c_2a90;

    /// The south bridge, simulated: `fiber-south-core` on the other end of a pipe, with counts
    /// that make 12 V on the input and 4.1 V on the battery at a supply of 3.3 V.
    async fn south_bridge(
        stream: DuplexStream,
        silent: Arc<AtomicBool>,
        seen: mpsc::UnboundedSender<Desired>,
    ) {
        let (mut rd, mut wr) = tokio::io::split(stream);
        let mut decoder = FrameDecoder::<MAX_WIRE>::new();
        let mut engine = Engine::new();
        let mut seq = 0u16;
        let mut buf = [0u8; 64];
        let hello = Hello {
            version: [0, 1, 0],
            image_crc: CRC,
            uid: [7; 12],
            reset: Reset::PowerOn,
            fault: None,
            option_bytes: 0xDEFF_E1AA,
            vrefint_cal: 1655,
        };
        loop {
            let Ok(n) = rd.read(&mut buf).await else { return };
            if n == 0 {
                return;
            }
            for &byte in &buf[..n] {
                let Some(frame) = decoder.push(byte) else { continue };
                if silent.load(Ordering::Relaxed) {
                    continue;
                }
                let answer = match Message::decode(frame) {
                    Ok((_, Message::HelloRequest)) => Message::Hello(hello),
                    Ok((applied, Message::Desired(desired))) => {
                        engine.desired(desired);
                        let _ = seen.send(desired);
                        Message::Status(Status {
                            applied,
                            ports: engine.ports(),
                            vbat: 2544,
                            vin: 2557,
                            vref: 1505,
                            uptime: 1,
                            rx_dropped: 0,
                        })
                    }
                    _ => continue,
                };
                let mut wire = [0u8; MAX_WIRE];
                let n = answer.encode(seq, &mut wire).unwrap();
                seq = seq.wrapping_add(1);
                wr.write_all(&wire[..n]).await.unwrap();
            }
        }
    }

    /// What was published: topic, payload, retained.
    type Published = Rc<RefCell<Vec<(String, String, bool)>>>;

    /// MQTT, faked: what was published, and what the test sends.
    struct FakeBus {
        published: Published,
        incoming: mpsc::UnboundedReceiver<BusEvent>,
    }

    impl Bus for FakeBus {
        async fn publish(&mut self, topic: String, payload: String, retain: bool) {
            self.published.borrow_mut().push((topic, payload, retain));
        }

        async fn next(&mut self) -> BusEvent {
            match self.incoming.recv().await {
                Some(event) => event,
                None => std::future::pending().await,
            }
        }
    }

    /// The last payload on a topic, as JSON.
    fn last(published: &Published, topic: &str) -> Option<Value> {
        let all = published.borrow();
        all.iter()
            .rev()
            .find(|(t, _, _)| t == topic)
            .map(|(_, p, _)| serde_json::from_str(p).unwrap())
    }

    fn count(published: &Published, topic: &str) -> usize {
        published.borrow().iter().filter(|(t, _, _)| t == topic).count()
    }

    #[tokio::test(start_paused = true)]
    async fn the_agent_with_a_south_bridge() {
        let (agent_side, bridge_side) = tokio::io::duplex(1024);
        let silent = Arc::new(AtomicBool::new(false));
        let (seen_tx, mut seen) = mpsc::unbounded_channel();
        tokio::spawn(south_bridge(bridge_side, silent.clone(), seen_tx));

        let published = Rc::new(RefCell::new(Vec::new()));
        let (bus_tx, incoming) = mpsc::unbounded_channel();
        let identity = Identity {
            version: "0.1.0".into(),
            hardware: "FIBER-R2 2.1".into(),
            serial: "1".into(),
            name: "fiber-test".into(),
        };
        let bus = FakeBus {
            published: published.clone(),
            incoming,
        };
        let mut agent = Agent::new(State::new(Settings { dividers: true }, Some(CRC)), bus, identity);
        let (rd, wr) = tokio::io::split(agent_side);

        let script = async {
            bus_tx.send(BusEvent::Connected).unwrap();
            tokio::time::sleep(Duration::from_millis(2500)).await;
            assert_eq!(last(&published, "fiber/status").unwrap()["state"], "online");
            let firmware = last(&published, "fiber/firmware/south").unwrap();
            assert_eq!(firmware["state"], "running");
            assert_eq!(firmware["version"], "0.1.0");
            assert_eq!(firmware["crc"], firmware["carried"]);
            let power = last(&published, "fiber/power").unwrap();
            assert_eq!(power["source"], "poe");
            assert!((power["input"].as_f64().unwrap() - 12.05).abs() < 0.05, "{power}");
            assert_eq!(last(&published, "fiber/ports/3").unwrap()["power"], true);

            // A request, delivered twice: answered once, and the south bridge gets the ports.
            let request = br#"{"id":"r1","op":"ports.power","params":{"port":3,"power":false}}"#.to_vec();
            bus_tx.send(BusEvent::Request(request.clone())).unwrap();
            bus_tx.send(BusEvent::Request(request)).unwrap();
            tokio::time::sleep(Duration::from_millis(1500)).await;
            assert_eq!(count(&published, "fiber/rsp/r1"), 1);
            assert_eq!(last(&published, "fiber/rsp/r1").unwrap()["ok"], true);
            assert_eq!(last(&published, "fiber/ports/3").unwrap()["power"], false);
            let mut ports = 0xFF;
            while let Ok(desired) = seen.try_recv() {
                ports = desired.ports;
            }
            assert_eq!(ports, 0b1111_1011);

            // A cycle: off now, on again after 2 s, and then the answer.
            let cycle =
                br#"{"id":"r2","op":"ports.power","params":{"port":1,"power":"cycle","seconds":2}}"#.to_vec();
            bus_tx.send(BusEvent::Request(cycle)).unwrap();
            tokio::time::sleep(Duration::from_millis(1100)).await;
            assert_eq!(last(&published, "fiber/ports/1").unwrap()["power"], false);
            assert_eq!(count(&published, "fiber/rsp/r2"), 0);
            tokio::time::sleep(Duration::from_millis(2000)).await;
            assert_eq!(
                last(&published, "fiber/rsp/r2").unwrap()["data"]["ports"][0],
                true
            );
            assert_eq!(last(&published, "fiber/ports/1").unwrap()["power"], true);

            // A reconnect sends every retained topic again.
            let before = count(&published, "fiber/status");
            bus_tx.send(BusEvent::Connected).unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(count(&published, "fiber/status"), before + 1);

            // The south bridge goes quiet: silent, and the power not known.
            silent.store(true, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_secs(5)).await;
            assert_eq!(
                last(&published, "fiber/firmware/south").unwrap()["state"],
                "silent"
            );
            assert_eq!(last(&published, "fiber/power").unwrap()["source"], "unknown");
        };
        tokio::select! {
            result = run(&mut agent, rd, wr) => panic!("the agent stopped: {result:?}"),
            () = script => {}
        }
    }
}
