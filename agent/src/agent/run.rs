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
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::firmware::{self, Seen, Tries};
use super::link::{Event, Reader, Writer};
use super::notify::Notify;
use super::requests::{self, Answer, Handled, RecentIds};
use super::state::{FirmwareState, State};
use super::thermometers::Round;
use super::topics::{self, Identity};

/// The prefix of every topic.
pub const PREFIX: &str = "fiber/";
/// Without a `Status` for this long, the south bridge counts as silent.
pub const SILENT_AFTER: Duration = Duration::from_secs(3);
/// `power` is sent again at least this often.
pub const POWER_EVERY: Duration = Duration::from_secs(60);
/// Without a `Hello` for this long after the start, the south bridge counts as not running the
/// firmware at all.
pub const NO_FIRMWARE_AFTER: Duration = Duration::from_secs(10);
/// Frames with another schema tag, in a row, before the chip is flashed.
pub const OTHER_SCHEMA_FRAMES: u32 = 3;

/// A probe's topic is sent at least this often.
pub const THERMOMETER_EVERY: Duration = Duration::from_secs(60);

/// Why [`run`] or [`run_without_link`] returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The south bridge is to be flashed; `request` is the id of a `firmware.flash` to answer.
    Flash { request: Option<String> },
    /// The time without the link is over: try to open the port again.
    Reopen,
}

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
    /// The flash tries, and where they are kept (none in the tests).
    tries: Tries,
    tries_path: Option<std::path::PathBuf>,
    other_schema: u32,
    hello_seen: bool,
    flash: Option<Option<String>>,
    /// systemd's watchdog, fed on every tick.
    pub notify: Notify,
    /// The rounds of the thermometers, from the thread that reads them; none in most tests.
    pub rounds: Option<mpsc::Receiver<Round>>,
    /// Each probe's last topic: its reading, and when it was sent.
    thermometers_sent: std::collections::BTreeMap<String, (Option<i32>, u8, Instant)>,
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
            tries: Tries::default(),
            tries_path: None,
            other_schema: 0,
            hello_seen: false,
            flash: None,
            notify: Notify::default(),
            rounds: None,
            thermometers_sent: Default::default(),
        }
    }

    /// Keeps the flash tries in `path`, across restarts.
    pub fn keep_tries(&mut self, path: std::path::PathBuf) {
        self.tries = Tries::load(&path);
        self.tries_path = Some(path);
    }

    /// Asks for a flash of the south bridge when what was `seen` calls for one and a try is
    /// allowed; the chip counts as failed when tries are used up.
    fn consider_flash(&mut self, seen: Seen) {
        let Some(carried) = self.state.south.carried else {
            return;
        };
        if self.flash.is_some() || !firmware::needs_flash(seen, carried) {
            return;
        }
        if self.tries.allowed("south", carried, now()) {
            self.flash = Some(None);
        } else {
            self.state.south.state = FirmwareState::Failed;
        }
    }

    /// Marks the south bridge as being flashed, and says so.
    pub async fn begin_flash(&mut self) {
        self.state.south.state = FirmwareState::Flashing;
        self.publish_state(false).await;
    }

    /// The end of a flash: the try is kept, the request answered, the state said. The link
    /// starts again afterwards, and the chip's `Hello` comes with it.
    pub async fn end_flash(&mut self, result: Result<(), String>, request: Option<String>) {
        let carried = self.state.south.carried.unwrap_or(0);
        self.tries.record("south", carried, now());
        if let Some(path) = &self.tries_path
            && let Err(e) = self.tries.save(path)
        {
            eprintln!("cannot keep the flash tries in {}: {e}", path.display());
        }
        self.state.south.state = match (&result, self.tries.allowed("south", carried, now())) {
            (Ok(()), _) => FirmwareState::Running,
            (Err(_), true) => FirmwareState::Silent,
            (Err(_), false) => FirmwareState::Failed,
        };
        self.state.south.hello = None;
        self.hello_seen = false;
        self.other_schema = 0;
        self.started = Instant::now();
        if let Some(id) = request {
            let answer = match result {
                Ok(()) => Answer::ok(&id, Some(serde_json::json!({"crc": format!("0x{carried:08x}")}))),
                Err(e) => Answer::error(&id, e),
            };
            self.answer(answer).await;
        }
        self.publish_state(false).await;
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
        self.publish_thermometers(force).await;
    }

    /// A round of the thermometers: the probes that are gone get an empty retained message.
    async fn round(&mut self, round: Round) {
        for id in self.state.thermometers.take(round, self.state.ports) {
            self.thermometers_sent.remove(&id);
            self.bus
                .publish(format!("{PREFIX}thermometers/{id}"), String::new(), true)
                .await;
        }
        self.publish_state(false).await;
    }

    /// Each probe's topic, when it moved by 0.1 °C or more, gained or lost its reading, changed
    /// port, or is a minute old; all of them with `force`.
    async fn publish_thermometers(&mut self, force: bool) {
        let ts = now();
        let due: Vec<(String, Value, Option<i32>, u8)> = self
            .state
            .thermometers
            .known
            .iter()
            .filter(|(_, known)| known.shown())
            .filter(|(id, known)| {
                force
                    || self.thermometers_sent.get(*id).is_none_or(|(millis, port, at)| {
                        *port != known.port
                            || at.elapsed() >= THERMOMETER_EVERY
                            || match (millis, known.millis) {
                                (Some(a), Some(b)) => (a - b).abs() >= 100,
                                (a, b) => a.is_some() != b.is_some(),
                            }
                    })
            })
            .map(|(id, known)| {
                (
                    id.clone(),
                    topics::thermometer(known, ts),
                    known.millis,
                    known.port,
                )
            })
            .collect();
        for (id, payload, millis, port) in due {
            self.thermometers_sent
                .insert(id.clone(), (millis, port, Instant::now()));
            self.bus
                .publish(format!("{PREFIX}thermometers/{id}"), payload.to_string(), true)
                .await;
        }
    }

    async fn answer(&mut self, answer: Answer) {
        self.bus
            .publish(format!("{PREFIX}rsp/{}", answer.id), answer.payload(), false)
            .await;
    }

    /// Once a second: cycles that are due, silence, the next `Desired`.
    /// Once a second. Without the link (`writer` is `None`) the south bridge counts as silent
    /// and nothing is sent to it, and the rest goes on: the watchdog, the cycles, MQTT.
    async fn tick<W: AsyncWrite + Unpin>(&mut self, writer: Option<&mut Writer<W>>) -> io::Result<()> {
        self.notify.send("WATCHDOG=1");
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
        match writer {
            Some(writer) => {
                let last = self.last_status.unwrap_or(self.started);
                if last.elapsed() > SILENT_AFTER {
                    self.state.south_silent();
                }
                if !self.hello_seen && self.started.elapsed() > NO_FIRMWARE_AFTER {
                    self.consider_flash(Seen::Silent);
                }
                writer.send(&Message::Desired(self.state.desired())).await?;
            }
            None => self.state.south_silent(),
        }
        self.publish_state(false).await;
        Ok(())
    }

    async fn link(&mut self, event: Event) {
        match event {
            Event::Hello(hello) => {
                self.hello_seen = true;
                self.other_schema = 0;
                self.state.south_hello(hello);
                self.consider_flash(Seen::Hello {
                    image_crc: hello.image_crc,
                });
            }
            Event::Status { status, .. } => {
                self.last_status = Some(Instant::now());
                if let Some(source) = self.state.south_status(status) {
                    let payload = topics::power_event(source.name(), now()).to_string();
                    self.bus
                        .publish(format!("{PREFIX}power/event"), payload, false)
                        .await;
                }
            }
            Event::OtherSchema => {
                self.other_schema += 1;
                if self.other_schema >= OTHER_SCHEMA_FRAMES {
                    self.consider_flash(Seen::OtherSchema);
                }
            }
            Event::Dropped => {}
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
            Handled::Flash { id, .. } => match self.state.south.carried {
                None => {
                    self.answer(Answer::error(&id, "this fiber program carries no firmware image"))
                        .await
                }
                Some(carried) if !self.tries.allowed("south", carried, now()) => {
                    self.answer(Answer::error(
                        &id,
                        "flashed three times in the last hour; try again later",
                    ))
                    .await
                }
                Some(_) => self.flash = Some(Some(id)),
            },
        }
        self.publish_state(false).await;
    }
}

/// The loop on the south bridge's link: a `Desired` every second, what the chip sends, and
/// MQTT. It returns when the chip is to be flashed, or with the link's error.
pub async fn run<R, W, B>(agent: &mut Agent<B>, reader: R, writer: W) -> io::Result<Stop>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    B: Bus,
{
    serve(agent, Some((Reader::new(reader), Writer::new(writer))), None).await
}

/// The loop without the link, while the south bridge's port cannot be opened: MQTT, the
/// requests and the watchdog go on, and the chip counts as silent. It returns after `wait`, to
/// open the port again, or when the chip is to be flashed.
pub async fn run_without_link<B: Bus>(agent: &mut Agent<B>, wait: Duration) -> Stop {
    let link: Option<(Reader<tokio::io::Empty>, Writer<tokio::io::Sink>)> = None;
    // Without a link there is nothing to fail.
    serve(agent, link, Some(Instant::now() + wait))
        .await
        .unwrap_or(Stop::Reopen)
}

async fn serve<R, W, B>(
    agent: &mut Agent<B>,
    mut link: Option<(Reader<R>, Writer<W>)>,
    until: Option<Instant>,
) -> io::Result<Stop>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    B: Bus,
{
    if let Some((_, writer)) = &mut link {
        // A new link: the chip has 10 s from now to say who it is.
        agent.started = Instant::now();
        agent.hello_seen = false;
        agent.last_status = None;
        writer.send(&Message::HelloRequest).await?;
    }
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        let from_chip = async {
            match &mut link {
                Some((reader, _)) => reader.next().await,
                None => std::future::pending().await,
            }
        };
        let over = async {
            match until {
                Some(until) => tokio::time::sleep_until(until).await,
                None => std::future::pending().await,
            }
        };
        let round = async {
            match &mut agent.rounds {
                Some(rounds) => match rounds.recv().await {
                    Some(round) => round,
                    None => std::future::pending().await,
                },
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tick.tick() => agent.tick(link.as_mut().map(|(_, writer)| writer)).await?,
            round = round => agent.round(round).await,
            event = from_chip => agent.link(event?).await,
            event = agent.bus.next() => match event {
                BusEvent::Connected => agent.publish_state(true).await,
                BusEvent::Request(payload) => agent.request(&payload).await,
            },
            () = over => return Ok(Stop::Reopen),
        }
        if let Some(request) = agent.flash.take() {
            return Ok(Stop::Flash { request });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::Settings;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::mpsc;

    const CRC: u32 = crate::agent::simulator::IMAGE_CRC;

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
        tokio::spawn(crate::agent::simulator::simulate(
            bridge_side,
            silent.clone(),
            Some(seen_tx),
        ));

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

    fn agent_with(
        carried: Option<u32>,
        published: &Published,
    ) -> (Agent<FakeBus>, mpsc::UnboundedSender<BusEvent>) {
        let (bus_tx, incoming) = mpsc::unbounded_channel();
        let identity = Identity {
            version: "0".into(),
            hardware: "h".into(),
            serial: "s".into(),
            name: "n".into(),
        };
        let bus = FakeBus {
            published: published.clone(),
            incoming,
        };
        (
            Agent::new(State::new(Settings { dividers: true }, carried), bus, identity),
            bus_tx,
        )
    }

    #[tokio::test(start_paused = true)]
    async fn without_the_link_mqtt_goes_on() {
        let published = Rc::new(RefCell::new(Vec::new()));
        let (mut agent, bus) = agent_with(Some(CRC), &published);
        bus.send(BusEvent::Connected).unwrap();
        bus.send(BusEvent::Request(
            br#"{"id":"p1","op":"ports.power","params":{"port":3,"power":false}}"#.to_vec(),
        ))
        .unwrap();
        let started = Instant::now();
        assert_eq!(
            run_without_link(&mut agent, Duration::from_secs(5)).await,
            Stop::Reopen
        );
        assert!(started.elapsed() >= Duration::from_secs(5));
        assert_eq!(last(&published, "fiber/status").unwrap()["state"], "online");
        assert_eq!(
            last(&published, "fiber/firmware/south").unwrap()["state"],
            "silent"
        );
        assert_eq!(last(&published, "fiber/power").unwrap()["source"], "unknown");
        assert_eq!(last(&published, "fiber/rsp/p1").unwrap()["ok"], true);
        assert_eq!(last(&published, "fiber/ports/3").unwrap()["power"], false);

        // Not flashed on its own without a port, but when asked.
        bus.send(BusEvent::Request(
            br#"{"id":"f3","op":"firmware.flash","params":{"chip":"south"}}"#.to_vec(),
        ))
        .unwrap();
        assert_eq!(
            run_without_link(&mut agent, Duration::from_secs(60)).await,
            Stop::Flash {
                request: Some("f3".into())
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn thermometers_on_mqtt() {
        use crate::agent::thermometers::{Probe, Round};
        let published = Rc::new(RefCell::new(Vec::new()));
        let (mut agent, _bus) = agent_with(Some(CRC), &published);
        let (rounds, rx) = mpsc::channel(4);
        agent.rounds = Some(rx);
        let on_port_2 = |millis: Option<i32>| {
            let mut ports = vec![Vec::new(); 8];
            ports[1] = vec![Probe {
                id: "28-a".into(),
                millis,
            }];
            Round { ports: Some(ports) }
        };
        let topic = "fiber/thermometers/28-a";

        rounds.send(on_port_2(Some(21437))).await.unwrap();
        run_without_link(&mut agent, Duration::from_secs(1)).await;
        assert_eq!(last(&published, topic).unwrap()["temperature"], 21.44);
        assert_eq!(
            last(&published, "fiber/ports/2").unwrap()["thermometers"][0],
            "28-a"
        );

        // Less than 0.1 °C: not sent again; 0.1 °C or more: sent.
        rounds.send(on_port_2(Some(21487))).await.unwrap();
        run_without_link(&mut agent, Duration::from_secs(1)).await;
        assert_eq!(count(&published, topic), 1);
        rounds.send(on_port_2(Some(21537))).await.unwrap();
        run_without_link(&mut agent, Duration::from_secs(1)).await;
        assert_eq!(count(&published, topic), 2);
        assert_eq!(last(&published, topic).unwrap()["temperature"], 21.54);

        // A minute later it is sent again, unchanged.
        rounds.send(on_port_2(Some(21537))).await.unwrap();
        run_without_link(&mut agent, Duration::from_secs(61)).await;
        assert_eq!(count(&published, topic), 3);

        // Gone: an empty retained message.
        rounds
            .send(Round {
                ports: Some(vec![Vec::new(); 8]),
            })
            .await
            .unwrap();
        run_without_link(&mut agent, Duration::from_secs(1)).await;
        let (_, payload, retained) = published
            .borrow()
            .iter()
            .rev()
            .find(|(t, _, _)| t == topic)
            .cloned()
            .unwrap();
        assert_eq!((payload.as_str(), retained), ("", true));
        assert_eq!(
            last(&published, "fiber/ports/2").unwrap()["thermometers"],
            serde_json::json!([])
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_chip_with_another_image_is_flashed() {
        let (agent_side, bridge_side) = tokio::io::duplex(1024);
        tokio::spawn(crate::agent::simulator::simulate(
            bridge_side,
            Arc::new(AtomicBool::new(false)),
            None,
        ));
        let published = Rc::new(RefCell::new(Vec::new()));
        let (mut agent, _bus) = agent_with(Some(0x1234_5678), &published);
        let (rd, wr) = tokio::io::split(agent_side);
        assert_eq!(
            run(&mut agent, rd, wr).await.unwrap(),
            Stop::Flash { request: None }
        );

        agent.begin_flash().await;
        assert_eq!(
            last(&published, "fiber/firmware/south").unwrap()["state"],
            "flashing"
        );
        agent.end_flash(Ok(()), None).await;
        assert_eq!(
            last(&published, "fiber/firmware/south").unwrap()["state"],
            "running"
        );
        // Two more failed tries, and the chip counts as failed.
        agent.end_flash(Err("no answer".into()), None).await;
        assert_eq!(
            last(&published, "fiber/firmware/south").unwrap()["state"],
            "silent"
        );
        agent.end_flash(Err("no answer".into()), None).await;
        assert_eq!(
            last(&published, "fiber/firmware/south").unwrap()["state"],
            "failed"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_flash_asked_for() {
        let (agent_side, bridge_side) = tokio::io::duplex(1024);
        tokio::spawn(crate::agent::simulator::simulate(
            bridge_side,
            Arc::new(AtomicBool::new(false)),
            None,
        ));
        let published = Rc::new(RefCell::new(Vec::new()));
        // The chip runs the image carried: only a request makes a flash.
        let (mut agent, bus) = agent_with(Some(CRC), &published);
        let (rd, wr) = tokio::io::split(agent_side);
        bus.send(BusEvent::Request(
            br#"{"id":"f1","op":"firmware.flash","params":{"chip":"south"}}"#.to_vec(),
        ))
        .unwrap();
        assert_eq!(
            run(&mut agent, rd, wr).await.unwrap(),
            Stop::Flash {
                request: Some("f1".into())
            }
        );
        agent.end_flash(Ok(()), Some("f1".into())).await;
        let answer = last(&published, "fiber/rsp/f1").unwrap();
        assert_eq!(answer["ok"], true);
        assert_eq!(answer["data"]["crc"], format!("0x{CRC:08x}"));
    }

    #[tokio::test(start_paused = true)]
    async fn without_a_carried_image_nothing_is_flashed() {
        let (agent_side, bridge_side) = tokio::io::duplex(1024);
        tokio::spawn(crate::agent::simulator::simulate(
            bridge_side,
            Arc::new(AtomicBool::new(false)),
            None,
        ));
        let published = Rc::new(RefCell::new(Vec::new()));
        let (mut agent, bus) = agent_with(None, &published);
        let (rd, wr) = tokio::io::split(agent_side);
        bus.send(BusEvent::Request(
            br#"{"id":"f2","op":"firmware.flash","params":{"chip":"south"}}"#.to_vec(),
        ))
        .unwrap();
        let script = async {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let answer = last(&published, "fiber/rsp/f2").unwrap();
            assert_eq!(answer["error"], "this fiber program carries no firmware image");
            assert_eq!(
                last(&published, "fiber/firmware/south").unwrap()["state"],
                "running"
            );
        };
        tokio::select! {
            result = run(&mut agent, rd, wr) => panic!("the agent stopped: {result:?}"),
            () = script => {}
        }
    }
}
