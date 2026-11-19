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
use tower_hmi::input::{ButtonEvent, Kind};

use super::buttons;
use super::buzzer::Pattern;
use super::display::Picture;
use super::firmware::{self, Seen, Tries};
use super::link::{Event, Reader, Writer};
use super::notify::Notify;
use super::power::Source;
use super::requests::{self, Answer, Handled, RecentIds};
use super::screen::{self, Ask, Facts, Screen};
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

/// The longest an apply runs (`TimeoutStartSec=` of `fiber-platform-apply.service`).
const APPLY_AT_MOST: Duration = Duration::from_secs(2 * 3600);

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
    /// A retained topic the broker kept, as `thermometers/{id}`: maybe of a probe that went
    /// while the agent was not running.
    Retained(String),
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
    /// Thermometers the broker kept that no round has found yet.
    stale: std::collections::BTreeSet<String>,
    /// The buttons' gestures, from the thread that watches them; none without the lines.
    pub gestures: Option<mpsc::Receiver<ButtonEvent>>,
    /// The buzzer's patterns; none without its line.
    pub buzzer: Option<mpsc::UnboundedSender<Pattern>>,
    /// The backlight's duty cycle, 0 to 100 %; none without it.
    pub backlight: Option<mpsc::UnboundedSender<u8>>,
    backlight_sent: Option<u8>,
    /// The front panel's pages and menu, and the picture they make for the LCD.
    pub screen: Screen,
    pub picture: Picture,
    /// The LCD's thread, and the picture last sent to it; none without the LCD.
    pub lcd: Option<std::sync::mpsc::Sender<super::lcd::Rows>>,
    lcd_sent: Option<super::lcd::Rows>,
    /// The press that is down has sent a client's message away or stopped a client's pattern:
    /// nothing else of it acts.
    spent: bool,
    /// When the pattern a client asked for ends: a press before then stops it (D26).
    sounding_until: Option<Instant>,
    /// Where the pages find what is not in the state; none in the tests.
    pub sources: Option<Sources>,
}

/// The files the pages read (the apply's results and progress, and the addresses), and the one
/// the menu writes to ask for a restart.
pub struct Sources {
    pub results: std::path::PathBuf,
    pub status: std::path::PathBuf,
    pub fib_trie: std::path::PathBuf,
    pub restart: std::path::PathBuf,
    /// The local time zone, from `/etc/localtime`.
    pub zone: super::zone::Zone,
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
            stale: Default::default(),
            gestures: None,
            buzzer: None,
            backlight: None,
            backlight_sent: None,
            screen: Screen::default(),
            picture: Picture::new(),
            lcd: None,
            lcd_sent: None,
            spent: false,
            sounding_until: None,
            sources: None,
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
            // A voltage moved by more than 0.05 V, or came or went (the battery is `null` when no
            // cell is fitted).
            let moved = |k: &str| match (before[k].as_f64(), power[k].as_f64()) {
                (Some(a), Some(b)) => (a - b).abs() > 0.05,
                (a, b) => a.is_some() != b.is_some(),
            };
            before["source"] != power["source"] || moved("input") || moved("battery")
        });
        if force || stale || moved {
            self.power_sent = Some(Instant::now());
            self.retained("power", power, true).await;
        }
        self.publish_thermometers(force).await;
        self.retained("display", topics::display(&self.state.display, ts), force)
            .await;
    }

    /// The backlight's duty cycle, sent when it changed.
    fn light(&mut self) {
        let duty = self.state.display.duty(Instant::now());
        if self.backlight_sent != Some(duty)
            && let Some(backlight) = &self.backlight
            && backlight.send(duty).is_ok()
        {
            self.backlight_sent = Some(duty);
        }
    }

    /// Asks for a restart: `fiber-restart.path` restarts the FIBER as root (D23).
    fn restart(&self) {
        let Some(sources) = &self.sources else {
            return;
        };
        match std::fs::write(&sources.restart, "restart\n") {
            Ok(()) => eprintln!("a restart asked for from the front panel"),
            Err(e) => eprintln!("cannot ask for a restart ({}): {e}", sources.restart.display()),
        }
    }

    /// What the pages show now.
    fn facts(&mut self) -> Facts {
        let read = |path: &std::path::Path| std::fs::read_to_string(path).unwrap_or_default();
        let mut facts = Facts {
            name: self.identity.name.clone(),
            ts: now(),
            source: match self.state.power.source {
                Source::Poe => "PoE",
                Source::Battery => "Battery",
                Source::Unknown => "Power not known",
            }
            .into(),
            volts: self.state.power.volts.map(|v| (v.input, v.cell())),
            thermometers: {
                let mut probes: Vec<_> = self
                    .state
                    .thermometers
                    .known
                    .iter()
                    .map(|(id, known)| (known.port, id.clone(), known.millis))
                    .collect();
                probes.sort();
                probes
            },
            message: self.state.display.message.clone(),
            versions: vec![
                ("fiber", self.identity.version.clone()),
                (
                    "south",
                    self.state.south.hello.map_or("not known".into(), |h| {
                        format!("{}.{}.{}", h.version[0], h.version[1], h.version[2])
                    }),
                ),
                ("tower", env!("FIBER_TOWER_REV").chars().take(7).collect()),
            ],
            ..Facts::default()
        };
        if let Some(sources) = &mut self.sources {
            (facts.utc_offset, facts.zone) = sources.zone.at(facts.ts);
            facts.addresses = screen::local_addresses(&read(&sources.fib_trie));
            if let Ok(results) = crate::platform::results::Results::read(&sources.results) {
                facts.components = results
                    .components
                    .into_iter()
                    .filter(|(_, r)| r.wanted == crate::platform::results::Wanted::On)
                    .map(|(name, r)| (name, format!("{:?}", r.result).to_lowercase()))
                    .collect();
            }
            // An apply that was killed leaves `running` behind; the unit gives one 2 hours at most.
            let fresh = std::fs::metadata(&sources.status)
                .and_then(|m| m.modified())
                .is_ok_and(|at| at.elapsed().is_ok_and(|age| age < APPLY_AT_MOST));
            if let Ok(status) = serde_json::from_str::<crate::platform::apply::Status>(&read(&sources.status))
                && status.state == crate::platform::apply::ApplyState::Running
                && fresh
            {
                facts.apply = Some((status.component.unwrap_or_default(), status.step, status.steps));
            }
        }
        facts
    }

    /// Draws the picture again, and sends it to the LCD when it changed.
    fn draw(&mut self) {
        let facts = self.facts();
        self.picture = screen::draw(&mut self.screen, &facts);
        let rows = super::lcd::rows(&self.picture);
        if let Some(lcd) = &self.lcd
            && self.lcd_sent != Some(rows)
        {
            if lcd.send(rows).is_ok() {
                self.lcd_sent = Some(rows);
            } else {
                // The thread has ended: nothing shows pictures any more.
                self.lcd = None;
            }
        }
    }

    /// A gesture of the buttons: on MQTT, the backlight lit, and the press acts at once, also on
    /// a dimmed display (D27). A press that sends a client's message away or stops a client's
    /// pattern does nothing else: its click, hold or repeats act on nothing (D26).
    async fn gesture(&mut self, event: ButtonEvent) {
        let at = Instant::now();
        self.state.display.lit_at = at;
        self.light();
        let mut sent_away = false;
        if event.kind == Kind::Press {
            let sounding = self.sounding_until.take().is_some_and(|until| until > at);
            sent_away = self.state.display.message.is_some();
            if sent_away {
                self.state.display.message = None;
                self.state.display.message_until = None;
            }
            self.spent = sent_away || sounding;
            if let Some(buzzer) = &self.buzzer {
                if sounding {
                    let _ = buzzer.send(Pattern::Silence);
                }
                // A click as the button goes down, whatever else the press does.
                let _ = buzzer.send(Pattern::Click);
            }
        }
        if !self.spent && self.screen.press(event) == Some(Ask::Restart) {
            self.restart();
        }
        self.draw();
        if sent_away {
            self.publish_state(false).await;
        }
        if let Some(gesture) = buttons::gesture(event.kind) {
            let payload = topics::button_event(event.buttons, gesture, now());
            self.bus
                .publish(format!("{PREFIX}buttons/event"), payload.to_string(), false)
                .await;
        }
    }

    /// A round of the thermometers: the probes that are gone get an empty retained message.
    async fn round(&mut self, round: Round) {
        for id in self.state.thermometers.take(round, self.state.ports) {
            self.thermometers_sent.remove(&id);
            self.clear_thermometer(&id).await;
        }
        self.clear_stale().await;
        self.publish_state(false).await;
    }

    async fn clear_thermometer(&mut self, id: &str) {
        self.bus
            .publish(format!("{PREFIX}thermometers/{id}"), String::new(), true)
            .await;
    }

    /// A retained topic from the broker: a probe the agent does not know is stale, once a round
    /// has said which probes there are.
    async fn retained_seen(&mut self, topic: String) {
        if let Some(id) = topic.strip_prefix("thermometers/")
            && !self.state.thermometers.known.contains_key(id)
        {
            self.stale.insert(id.to_owned());
        }
        self.clear_stale().await;
    }

    /// Clears the stale thermometers: those the broker kept and no round has found.
    async fn clear_stale(&mut self) {
        if self.state.thermometers.buses.is_none() {
            return;
        }
        let stale: Vec<String> = std::mem::take(&mut self.stale)
            .into_iter()
            .filter(|id| !self.state.thermometers.known.contains_key(id))
            .collect();
        for id in stale {
            self.clear_thermometer(&id).await;
        }
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
        self.state.display.expire(Instant::now());
        self.light();
        self.draw();
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
            Handled::Beep { id, pattern } => {
                let answer = match &self.buzzer {
                    Some(buzzer) if buzzer.send(pattern).is_ok() => {
                        self.sounding_until = Some(Instant::now() + pattern.length());
                        Answer::ok(&id, None)
                    }
                    _ => Answer::error(&id, "the buzzer is not there"),
                };
                self.answer(answer).await;
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
        // A message lights the backlight, a new brightness shows: at once, not at the next tick.
        self.light();
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
        let gesture = async {
            match &mut agent.gestures {
                Some(gestures) => match gestures.recv().await {
                    Some(gesture) => gesture,
                    None => std::future::pending().await,
                },
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tick.tick() => agent.tick(link.as_mut().map(|(_, writer)| writer)).await?,
            round = round => agent.round(round).await,
            gesture = gesture => agent.gesture(gesture).await,
            event = from_chip => agent.link(event?).await,
            event = agent.bus.next() => match event {
                BusEvent::Connected => agent.publish_state(true).await,
                BusEvent::Request(payload) => agent.request(&payload).await,
                BusEvent::Retained(topic) => agent.retained_seen(topic).await,
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
    async fn a_probe_that_went_while_the_agent_was_away() {
        use crate::agent::thermometers::{Probe, Round};
        let published = Rc::new(RefCell::new(Vec::new()));
        let (mut agent, bus) = agent_with(Some(CRC), &published);
        let (rounds, rx) = mpsc::channel(4);
        agent.rounds = Some(rx);
        // The broker kept two; before the first round nothing is cleared.
        bus.send(BusEvent::Retained("thermometers/28-old".into()))
            .unwrap();
        bus.send(BusEvent::Retained("thermometers/28-a".into())).unwrap();
        run_without_link(&mut agent, Duration::from_secs(1)).await;
        assert!(last(&published, "fiber/thermometers/28-old").is_none());
        let mut ports = vec![Vec::new(); 8];
        ports[0] = vec![Probe {
            id: "28-a".into(),
            millis: Some(20000),
        }];
        rounds.send(Round { ports: Some(ports) }).await.unwrap();
        run_without_link(&mut agent, Duration::from_secs(1)).await;
        let cleared = |id: &str| {
            published.borrow().iter().any(|(t, payload, retained)| {
                t == &format!("fiber/thermometers/{id}") && payload.is_empty() && *retained
            })
        };
        assert!(cleared("28-old"));
        assert!(!cleared("28-a"));
        assert_eq!(
            last(&published, "fiber/thermometers/28-a").unwrap()["temperature"],
            20.0
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_front_panel() {
        use crate::agent::buttons::{DOWN, ENTER};
        let published = Rc::new(RefCell::new(Vec::new()));
        let (mut agent, bus) = agent_with(Some(CRC), &published);

        // Without the buzzer's line: the request says so.
        bus.send(BusEvent::Request(br#"{"id":"b0","op":"buzzer.beep"}"#.to_vec()))
            .unwrap();
        run_without_link(&mut agent, Duration::from_secs(1)).await;
        assert_eq!(
            last(&published, "fiber/rsp/b0").unwrap()["error"],
            "the buzzer is not there"
        );

        let (gestures, rx) = mpsc::channel(4);
        agent.gestures = Some(rx);
        let (buzzer, mut beeps) = mpsc::unbounded_channel();
        agent.buzzer = Some(buzzer);
        let (backlight, mut duty) = mpsc::unbounded_channel();
        agent.backlight = Some(backlight);
        bus.send(BusEvent::Request(
            br#"{"id":"b1","op":"buzzer.beep","params":{"pattern":"double"}}"#.to_vec(),
        ))
        .unwrap();
        bus.send(BusEvent::Request(
            br#"{"id":"m1","op":"display.message","params":{"text":"hello","seconds":5}}"#.to_vec(),
        ))
        .unwrap();
        run_without_link(&mut agent, Duration::from_secs(2)).await;
        assert_eq!(last(&published, "fiber/rsp/b1").unwrap()["ok"], true);
        assert_eq!(beeps.try_recv().unwrap(), Pattern::Double);
        assert_eq!(duty.try_recv().unwrap(), 80);
        assert!(duty.try_recv().is_err());
        assert_eq!(last(&published, "fiber/display").unwrap()["message"], "hello");

        // The message goes after its 5 s; a minute without a press dims the backlight.
        run_without_link(&mut agent, Duration::from_secs(60)).await;
        assert_eq!(last(&published, "fiber/display").unwrap()["message"], Value::Null);
        assert_eq!(duty.try_recv().unwrap(), 20);

        // A click on the dimmed display: on MQTT, the backlight lit again, and it acts at once:
        // the menu opens (D27).
        gestures
            .send(ButtonEvent {
                buttons: ENTER,
                kind: Kind::Click,
            })
            .await
            .unwrap();
        run_without_link(&mut agent, Duration::from_secs(1)).await;
        let event = last(&published, "fiber/buttons/event").unwrap();
        assert_eq!(event["buttons"], serde_json::json!(["enter"]));
        assert_eq!(event["gesture"], "click");
        assert_eq!(duty.try_recv().unwrap(), 80);
        assert!(matches!(agent.screen.view, crate::agent::screen::View::Menu(_)));
        assert_eq!(
            crate::agent::display::as_text(&agent.picture),
            std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/agent/pictures/menu.txt"
            ))
            .unwrap()
        );

        // Dimmed again: a press of down lights it and moves the menu at once, as the button goes
        // down; it clicks, the click on the release does not.
        run_without_link(&mut agent, Duration::from_secs(61)).await;
        assert_eq!(duty.try_recv().unwrap(), 20);
        let menu = crate::agent::display::as_text(&agent.picture);
        let press = |buttons, kind| ButtonEvent { buttons, kind };
        gestures.send(press(DOWN, Kind::Press)).await.unwrap();
        gestures.send(press(DOWN, Kind::Click)).await.unwrap();
        run_without_link(&mut agent, Duration::from_secs(1)).await;
        assert_eq!(beeps.try_recv().unwrap(), Pattern::Click);
        assert!(beeps.try_recv().is_err());
        assert_eq!(duty.try_recv().unwrap(), 80);
        assert!(
            crate::agent::display::as_text(&agent.picture) != menu,
            "the press moved nothing"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_press_sends_a_message_away() {
        use crate::agent::buttons::DOWN;
        use crate::agent::screen::View;
        let published = Rc::new(RefCell::new(Vec::new()));
        let (mut agent, bus) = agent_with(Some(CRC), &published);
        let (gestures, rx) = mpsc::channel(4);
        agent.gestures = Some(rx);
        let (buzzer, mut beeps) = mpsc::unbounded_channel();
        agent.buzzer = Some(buzzer);
        let (backlight, mut duty) = mpsc::unbounded_channel();
        agent.backlight = Some(backlight);
        let press = |buttons, kind| ButtonEvent { buttons, kind };
        let request = |json: &str| BusEvent::Request(json.as_bytes().to_vec());
        // The backlight's last duty cycle sent, if any since the last look.
        let latest = |duty: &mut mpsc::UnboundedReceiver<u8>| {
            let mut latest = None;
            while let Ok(d) = duty.try_recv() {
                latest = Some(d);
            }
            latest
        };

        // An alarm as the example flow asks for it: a message for 10 minutes, and the pattern.
        bus.send(request(
            r#"{"id":"m1","op":"display.message","params":{"text":"Port 5: 30.8 C\nabove 30 C","seconds":600}}"#,
        ))
        .unwrap();
        bus.send(request(
            r#"{"id":"b1","op":"buzzer.beep","params":{"pattern":"alarm"}}"#,
        ))
        .unwrap();
        run_without_link(&mut agent, Duration::from_millis(500)).await;
        assert_eq!(beeps.try_recv().unwrap(), Pattern::Alarm);
        assert_eq!(
            last(&published, "fiber/display").unwrap()["message"],
            "Port 5: 30.8 C\nabove 30 C"
        );

        // A press of down while it sounds: the message goes, the alarm stops, the page stays,
        // and its click (on the release) acts on nothing.
        gestures.send(press(DOWN, Kind::Press)).await.unwrap();
        gestures.send(press(DOWN, Kind::Click)).await.unwrap();
        run_without_link(&mut agent, Duration::from_millis(100)).await;
        assert_eq!(beeps.try_recv().unwrap(), Pattern::Silence);
        assert_eq!(beeps.try_recv().unwrap(), Pattern::Click);
        assert!(beeps.try_recv().is_err());
        assert_eq!(last(&published, "fiber/display").unwrap()["message"], Value::Null);
        assert!(matches!(agent.screen.view, View::Page(0)));

        // The next press acts as ever.
        gestures.send(press(DOWN, Kind::Press)).await.unwrap();
        run_without_link(&mut agent, Duration::from_millis(100)).await;
        assert_eq!(beeps.try_recv().unwrap(), Pattern::Click);
        assert!(matches!(agent.screen.view, View::Page(1)));

        // A pattern that has ended is not stopped: the press moves the page.
        bus.send(request(
            r#"{"id":"b2","op":"buzzer.beep","params":{"pattern":"alarm"}}"#,
        ))
        .unwrap();
        run_without_link(&mut agent, Duration::from_secs(2)).await;
        assert_eq!(beeps.try_recv().unwrap(), Pattern::Alarm);
        gestures.send(press(DOWN, Kind::Press)).await.unwrap();
        run_without_link(&mut agent, Duration::from_millis(100)).await;
        assert_eq!(beeps.try_recv().unwrap(), Pattern::Click);
        assert!(matches!(agent.screen.view, View::Page(2)));

        // A message lights a dimmed backlight at once (D27).
        run_without_link(&mut agent, Duration::from_secs(61)).await;
        assert_eq!(latest(&mut duty), Some(20));
        bus.send(request(
            r#"{"id":"m2","op":"display.message","params":{"text":"hello"}}"#,
        ))
        .unwrap();
        run_without_link(&mut agent, Duration::from_millis(100)).await;
        assert_eq!(latest(&mut duty), Some(80));

        // One press sends it away, also on a dimmed display, and does nothing else.
        run_without_link(&mut agent, Duration::from_secs(61)).await;
        assert_eq!(latest(&mut duty), Some(20));
        gestures.send(press(DOWN, Kind::Press)).await.unwrap();
        run_without_link(&mut agent, Duration::from_millis(100)).await;
        assert_eq!(beeps.try_recv().unwrap(), Pattern::Click);
        assert_eq!(latest(&mut duty), Some(80));
        assert_eq!(last(&published, "fiber/display").unwrap()["message"], Value::Null);
        assert!(matches!(agent.screen.view, View::Page(2)));
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
