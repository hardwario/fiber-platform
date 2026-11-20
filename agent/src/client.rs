//! The command line as a client of the agent's MQTT API (`docs/cli.md`, "The board"): each
//! command reads the agent's retained topics or sends it a request, so whatever it does, a
//! program of your own can do too.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde_json::{Value, json};

use crate::agent::topics::SCHEMA;

/// The agent does not answer: its `status` is `offline`, or not there.
pub const EXIT_NO_AGENT: u8 = 4;
/// The agent speaks another schema of the API.
pub const EXIT_SCHEMA: u8 = 125;

/// Where the broker is, and the login for a FIBER's listener for the network.
#[derive(Clone, Debug)]
pub struct Broker {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
}

impl Broker {
    /// From `--broker HOST[:PORT]` (the FIBER itself without it) and `--user`; the password is in
    /// `FIBER_MQTT_PASSWORD`.
    pub fn new(broker: Option<&str>, user: Option<String>) -> Result<Broker> {
        let (host, port) = match broker {
            None => ("127.0.0.1".to_owned(), 1883),
            Some(text) => match text.rsplit_once(':') {
                Some((host, port)) => (
                    host.to_owned(),
                    port.parse()
                        .map_err(|_| anyhow!("--broker: `{port}` is not a port"))?,
                ),
                None => (text.to_owned(), if user.is_some() { 1884 } else { 1883 }),
            },
        };
        Ok(Broker { host, port, user })
    }

    fn connect(&self) -> (AsyncClient, rumqttc::EventLoop) {
        let mut options =
            MqttOptions::new(format!("fiber-cli-{}", std::process::id()), &self.host, self.port);
        options.set_keep_alive(Duration::from_secs(15));
        if let Some(user) = &self.user {
            options.set_credentials(user, std::env::var("FIBER_MQTT_PASSWORD").unwrap_or_default());
        }
        AsyncClient::new(options, 16)
    }
}

/// The retained topics under `fiber/`, as the broker has them (without the prefix).
type Retained = BTreeMap<String, Value>;

async fn retained(broker: &Broker) -> Result<Retained> {
    let (client, mut events) = broker.connect();
    client.subscribe("fiber/#", QoS::AtLeastOnce).await?;
    let mut topics = Retained::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
    loop {
        let event = tokio::time::timeout_at(deadline, events.poll()).await;
        match event {
            Err(_) => break,
            Ok(Err(e)) => bail!("cannot reach the broker at {}:{}: {e}", broker.host, broker.port),
            Ok(Ok(Event::Incoming(Packet::Publish(p)))) if p.retain => {
                if let (Some(topic), Ok(value)) =
                    (p.topic.strip_prefix("fiber/"), serde_json::from_slice(&p.payload))
                {
                    topics.insert(topic.to_owned(), value);
                }
            }
            Ok(Ok(_)) => {}
        }
    }
    let _ = client.disconnect().await;
    Ok(topics)
}

/// Whether the agent is there and speaks this schema; an exit status when not.
fn check_agent(topics: &Retained) -> Option<ExitCode> {
    match topics.get("status") {
        Some(status) if status["schema"] != SCHEMA => {
            eprintln!(
                "error: the agent speaks schema {} of the API, and this program schema {SCHEMA}",
                status["schema"]
            );
            Some(ExitCode::from(EXIT_SCHEMA))
        }
        Some(status) if status["state"] == "online" => None,
        _ => {
            eprintln!("error: the agent does not answer (its status is offline, or not there)");
            Some(ExitCode::from(EXIT_NO_AGENT))
        }
    }
}

/// A new id for every request: the process, a count within it, and the time. The agent drops
/// an id it has seen among the last 64, so two requests of one command must never share one.
fn request_id() -> String {
    static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let count = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("cli-{}-{count}-{nanos}", std::process::id())
}

/// Sends a request and waits for its answer.
async fn request(broker: &Broker, op: &str, params: Value, wait: Duration) -> Result<Value> {
    let (client, mut events) = broker.connect();
    let id = request_id();
    client
        .subscribe(format!("fiber/rsp/{id}"), QoS::AtLeastOnce)
        .await?;
    let mut sent = false;
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let event = tokio::time::timeout_at(deadline, events.poll()).await;
        match event {
            Err(_) => bail!("the agent did not answer within {} s", wait.as_secs()),
            Ok(Err(e)) => bail!("cannot reach the broker at {}:{}: {e}", broker.host, broker.port),
            Ok(Ok(Event::Incoming(Packet::SubAck(_)))) if !sent => {
                let body = json!({"id": id, "op": op, "params": params}).to_string();
                client.publish("fiber/cmd", QoS::AtLeastOnce, false, body).await?;
                sent = true;
            }
            Ok(Ok(Event::Incoming(Packet::Publish(p)))) if p.topic == format!("fiber/rsp/{id}") => {
                let _ = client.disconnect().await;
                let answer: Value = serde_json::from_slice(&p.payload)?;
                if answer["ok"] == true {
                    return Ok(answer.get("data").cloned().unwrap_or(Value::Null));
                }
                bail!("{}", answer["error"].as_str().unwrap_or("the agent refused it"));
            }
            Ok(Ok(_)) => {}
        }
    }
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}

/// `Ports       1 2 3 4 5 6 7 8 on`, or which are off.
pub fn ports_line(on: &[bool]) -> String {
    let list = |state: bool| -> Vec<String> {
        on.iter()
            .enumerate()
            .filter(|(_, p)| **p == state)
            .map(|(i, _)| (i + 1).to_string())
            .collect()
    };
    let (up, down) = (list(true), list(false));
    match (up.is_empty(), down.is_empty()) {
        (_, true) => format!("{} on", up.join(" ")),
        (true, _) => format!("{} off", down.join(" ")),
        _ => format!("{} on, {} off", up.join(" "), down.join(" ")),
    }
}

fn power_line(power: Option<&Value>) -> String {
    let Some(power) = power else {
        return "not known".into();
    };
    let source = match power["source"].as_str() {
        Some("poe") => "PoE",
        Some("battery") => "battery",
        _ => "not known",
    };
    match (power["input"].as_f64(), power["battery"].as_f64()) {
        (Some(input), Some(battery)) => format!("{source}, input {input:.2} V, battery {battery:.2} V"),
        (Some(input), None) => format!("{source}, input {input:.2} V, no battery"),
        _ => source.to_owned(),
    }
}

fn firmware_line(firmware: Option<&Value>) -> String {
    let Some(f) = firmware else {
        return "not known".into();
    };
    let state = f["state"].as_str().unwrap_or("?");
    match (f["version"].as_str(), f["crc"].as_str()) {
        (Some(version), Some(crc)) => {
            let same = if f["carried"] == f["crc"] {
                "the one carried"
            } else {
                "not the one carried"
            };
            format!("{state} {version}, image {crc} ({same})")
        }
        _ => state.to_owned(),
    }
}

/// What `fiber status` prints.
pub fn status_text(topics: &Retained) -> String {
    let status = &topics["status"];
    let ports: Vec<bool> = (1..=8)
        .map(|p| {
            topics
                .get(&format!("ports/{p}"))
                .is_some_and(|v| v["power"] == true)
        })
        .collect();
    format!(
        "FIBER       {}, {}, serial {}\nAgent       online, {}, schema {}\nPower       {}\nPorts       {}\nSouth       {}\n",
        status["name"].as_str().unwrap_or("?"),
        status["hardware"].as_str().unwrap_or("?"),
        status["serial"].as_str().unwrap_or("?"),
        status["version"].as_str().unwrap_or("?"),
        status["schema"],
        power_line(topics.get("power")),
        ports_line(&ports),
        firmware_line(topics.get("firmware/south")),
    )
}

/// Reads the retained topics and prints with `print`, after checking the agent.
fn show(broker: &Broker, print: impl Fn(&Retained) -> String) -> Result<ExitCode> {
    let topics = runtime()?.block_on(retained(broker))?;
    if let Some(code) = check_agent(&topics) {
        return Ok(code);
    }
    print!("{}", print(&topics));
    Ok(ExitCode::SUCCESS)
}

/// `fiber status`.
pub fn status(broker: &Broker) -> Result<ExitCode> {
    show(broker, status_text)
}

/// `fiber power [--follow]`.
pub fn power(broker: &Broker, follow: bool) -> Result<ExitCode> {
    let code = show(broker, |topics| {
        format!("Power       {}\n", power_line(topics.get("power")))
    })?;
    if !follow || code != ExitCode::SUCCESS {
        return Ok(code);
    }
    runtime()?.block_on(watch(
        broker,
        &["fiber/power", "fiber/power/event"],
        |topic, payload| {
            let value: Value = serde_json::from_slice(payload).ok()?;
            let at = clock(value["ts"].as_u64().unwrap_or(0));
            Some(match topic {
                "fiber/power/event" => format!("{at}  {}", value["event"].as_str().unwrap_or("?")),
                _ => format!("{at}  {}", power_line(Some(&value))),
            })
        },
    ))?;
    Ok(ExitCode::SUCCESS)
}

/// The time of day of a Unix time, in UTC: `14:02:11`.
pub fn clock(ts: u64) -> String {
    let seconds = ts % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// Prints a line for every message on `topics` that is not a retained one (those are printed
/// before), until Ctrl+C. `line` makes the line, or `None` to print nothing.
async fn watch(
    broker: &Broker,
    topics: &[&str],
    mut line: impl FnMut(&str, &[u8]) -> Option<String>,
) -> Result<()> {
    let (client, mut events) = broker.connect();
    for topic in topics {
        client.subscribe(*topic, QoS::AtLeastOnce).await?;
    }
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                let _ = client.disconnect().await;
                return Ok(());
            }
            event = events.poll() => match event {
                Err(e) => bail!("cannot reach the broker at {}:{}: {e}", broker.host, broker.port),
                Ok(Event::Incoming(Packet::Publish(p))) if !p.retain => {
                    if let Some(text) = line(&p.topic, &p.payload) {
                        println!("{text}");
                    }
                }
                Ok(_) => {}
            },
        }
    }
}

/// A probe's reading as `fiber thermometers` shows it.
fn reading(value: &Value) -> String {
    match value["temperature"].as_f64() {
        Some(t) => format!("{t:.2} °C"),
        None => "no reading".into(),
    }
}

/// What `fiber thermometers` prints.
pub fn thermometers_text(topics: &Retained) -> String {
    let mut probes: Vec<(u64, &str, String)> = topics
        .iter()
        .filter_map(|(topic, value)| {
            let id = topic.strip_prefix("thermometers/")?;
            Some((value["port"].as_u64().unwrap_or(0), id, reading(value)))
        })
        .collect();
    probes.sort();
    if probes.is_empty() {
        let buses = topics
            .get("ports/1")
            .is_none_or(|port| !port["thermometers"].is_null());
        return if buses {
            "No thermometers.\n".into()
        } else {
            "The 1-Wire buses were not found; see: fiber doctor\n".into()
        };
    }
    probes
        .iter()
        .map(|(port, id, reading)| format!("{:<12}{id}  {reading}\n", format!("Port {port}")))
        .collect()
}

/// `fiber thermometers [watch]`.
pub fn thermometers(broker: &Broker, follow: bool) -> Result<ExitCode> {
    let code = show(broker, thermometers_text)?;
    if !follow || code != ExitCode::SUCCESS {
        return Ok(code);
    }
    // The port of each probe, for the line that says it is gone.
    let mut ports: BTreeMap<String, u64> = BTreeMap::new();
    runtime()?.block_on(watch(broker, &["fiber/thermometers/+"], |topic, payload| {
        let id = topic.strip_prefix("fiber/thermometers/")?.to_owned();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if payload.is_empty() {
            let port = ports
                .remove(&id)
                .map(|p| format!("port {p}  "))
                .unwrap_or_default();
            return Some(format!("{}  {port}{id}  gone", clock(now)));
        }
        let value: Value = serde_json::from_slice(payload).ok()?;
        let port = value["port"].as_u64().unwrap_or(0);
        ports.insert(id.clone(), port);
        let at = clock(value["ts"].as_u64().unwrap_or(now));
        Some(format!("{at}  port {port}  {id}  {}", reading(&value)))
    }))?;
    Ok(ExitCode::SUCCESS)
}

/// `fiber ports [on|off|cycle] [PORT...] [--seconds N]`.
pub fn ports(broker: &Broker, action: Option<&str>, ports: &[u8], seconds: Option<u64>) -> Result<ExitCode> {
    let Some(action) = action else {
        return show(broker, |topics| {
            let on: Vec<bool> = (1..=8)
                .map(|p| {
                    topics
                        .get(&format!("ports/{p}"))
                        .is_some_and(|v| v["power"] == true)
                })
                .collect();
            format!("Ports       {}\n", ports_line(&on))
        });
    };
    let power = match action {
        "on" => json!(true),
        "off" => json!(false),
        "cycle" => json!("cycle"),
        other => bail!("`{other}` is not on, off or cycle"),
    };
    let targets: Vec<Value> = if ports.is_empty() {
        vec![json!("all")]
    } else {
        ports.iter().map(|p| json!(p)).collect()
    };
    let runtime = runtime()?;
    let mut last = Value::Null;
    for target in targets {
        let params = json!({"port": target, "power": power, "seconds": seconds});
        last = runtime.block_on(request(broker, "ports.power", params, Duration::from_secs(75)))?;
    }
    let on: Vec<bool> = last["ports"]
        .as_array()
        .map(|a| a.iter().map(|v| v == true).collect())
        .unwrap_or_default();
    println!("Ports       {}", ports_line(&on));
    Ok(ExitCode::SUCCESS)
}

/// `fiber leds [auto | COLOUR [--blink ON OFF]] [LED...]`.
pub fn leds(
    broker: &Broker,
    action: Option<&str>,
    leds: &[String],
    blink: Option<(u64, u64)>,
) -> Result<ExitCode> {
    let Some(action) = action else {
        return show(broker, |topics| {
            let mut out = String::new();
            for p in 1..=8 {
                let led = topics
                    .get(&format!("ports/{p}"))
                    .and_then(|v| v["led"].as_str())
                    .unwrap_or("?")
                    .to_owned();
                out.push_str(&format!("port{p}       {led}\n"));
            }
            out
        });
    };
    let targets: Vec<String> = if leds.is_empty() {
        if action == "auto" {
            vec!["all".into()]
        } else {
            (1..=8).map(|p| format!("port{p}")).collect()
        }
    } else {
        leds.to_vec()
    };
    let runtime = runtime()?;
    for led in targets {
        let (op, params) = if action == "auto" {
            ("leds.auto", json!({"led": led}))
        } else {
            let blink = blink.map(|(on, off)| json!({"on": on, "off": off}));
            ("leds.set", json!({"led": led, "colour": action, "blink": blink}))
        };
        runtime.block_on(request(broker, op, params, Duration::from_secs(10)))?;
    }
    Ok(ExitCode::SUCCESS)
}

/// What `fiber display` prints.
pub fn display_text(topics: &Retained) -> String {
    let display = topics.get("display");
    let backlight = display
        .and_then(|d| d["backlight"].as_u64())
        .map_or("not known".to_owned(), |level| format!("{level} %"));
    let message = display
        .and_then(|d| d["message"].as_str())
        .map_or("none".to_owned(), |text| {
            text.split('\n').collect::<Vec<_>>().join(" / ")
        });
    format!("Backlight   {backlight}\nMessage     {message}\n")
}

/// What `fiber display` does.
pub enum DisplayAction {
    Message { text: String, seconds: Option<u64> },
    Clear,
    Backlight(u8),
}

/// `fiber display [message TEXT [--seconds N] | clear | backlight LEVEL]`.
pub fn display(broker: &Broker, action: Option<DisplayAction>) -> Result<ExitCode> {
    let (op, params) = match action {
        None => return show(broker, display_text),
        // Typed in a shell, a new line comes as the two characters `\n`.
        Some(DisplayAction::Message { text, seconds }) => (
            "display.message",
            json!({"text": text.replace("\\n", "\n"), "seconds": seconds}),
        ),
        Some(DisplayAction::Clear) => ("display.message", json!({"text": null})),
        Some(DisplayAction::Backlight(level)) => ("display.backlight", json!({"level": level})),
    };
    runtime()?.block_on(request(broker, op, params, Duration::from_secs(10)))?;
    Ok(ExitCode::SUCCESS)
}

/// `fiber buzzer beep [PATTERN]`.
pub fn beep(broker: &Broker, pattern: Option<&str>) -> Result<ExitCode> {
    runtime()?.block_on(request(
        broker,
        "buzzer.beep",
        json!({"pattern": pattern}),
        Duration::from_secs(10),
    ))?;
    Ok(ExitCode::SUCCESS)
}

/// `fiber buttons watch`.
pub fn buttons_watch(broker: &Broker) -> Result<ExitCode> {
    let code = show(broker, |_| String::new())?;
    if code != ExitCode::SUCCESS {
        return Ok(code);
    }
    runtime()?.block_on(watch(broker, &["fiber/buttons/event"], |_, payload| {
        let event: Value = serde_json::from_slice(payload).ok()?;
        let buttons: Vec<&str> = event["buttons"]
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        Some(format!(
            "{}  {}  {}",
            clock(event["ts"].as_u64().unwrap_or(0)),
            buttons.join("+"),
            event["gesture"].as_str().unwrap_or("?")
        ))
    }))?;
    Ok(ExitCode::SUCCESS)
}

/// `fiber firmware [status | flash CHIP]`.
pub fn firmware(broker: &Broker, flash: Option<&str>) -> Result<ExitCode> {
    match flash {
        None => show(broker, |topics| {
            format!(
                "South       {}\nNorth       {}\n",
                firmware_line(topics.get("firmware/south")),
                firmware_line(topics.get("firmware/north"))
            )
        }),
        Some(chip) => {
            let data = runtime()?.block_on(request(
                broker,
                "firmware.flash",
                json!({"chip": chip}),
                Duration::from_secs(120),
            ))?;
            println!(
                "Flashed     {chip}, image {}",
                data["crc"].as_str().unwrap_or("?")
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// What `fiber radio` prints.
pub fn radio_text(topics: &Retained) -> String {
    let Some(status) = topics.get("radio/status") else {
        return "The radio is not there (no radio/status).\n".into();
    };
    let s = |key: &str| status[key].as_str().unwrap_or("?").to_owned();
    let radio = if status["personality"].is_null() {
        "the north bridge does not answer".to_owned()
    } else {
        format!(
            "{}, {}, channel {}, gateway {}",
            s("personality"),
            s("band"),
            status["channel"],
            s("gateway")
        )
    };
    let wire = s("wire");
    let schema = wire.strip_prefix("radio ").unwrap_or(&wire);
    let held = if status["can_send"] == true {
        ""
    } else {
        "; sending held back"
    };
    let pairing = match topics.get("radio/pairing") {
        Some(p) if p["state"] == "open" => format!("open, {} s left", p["remaining"]),
        _ => "idle".to_owned(),
    };
    format!(
        "Radio       {radio}\nTOWER       {}, radio schema {schema}\nNodes       {} of 16{held}\nPairing     {pairing}\n",
        s("tower"),
        status["nodes"]
    )
}

/// What `fiber radio nodes` prints.
pub fn radio_nodes_text(topics: &Retained) -> String {
    let nodes: Vec<&Value> = topics
        .iter()
        .filter(|(topic, _)| {
            topic
                .strip_prefix("radio/nodes/")
                .is_some_and(|rest| !rest.contains('/'))
        })
        .map(|(_, value)| value)
        .collect();
    if nodes.is_empty() {
        return "No nodes.\n".into();
    }
    let dash = |v: &Value| match v.as_str() {
        Some("") | None => "-".to_owned(),
        Some(text) => text.to_owned(),
    };
    nodes
        .iter()
        .map(|n| {
            let rssi = n["rssi"]
                .as_i64()
                .map_or("never".to_owned(), |r| format!("{r} dBm"));
            let seen = n["last_seen"].as_u64().map(clock).unwrap_or_default();
            let queued = match n["queued"].as_u64() {
                Some(0) | None => String::new(),
                Some(q) => format!("{q} queued"),
            };
            let mut line = format!(
                "{}  {:<16}  {:<8}  {:<8}",
                n["addr"].as_str().unwrap_or("?"),
                dash(&n["name"]),
                dash(&n["kind"]),
                rssi
            );
            if !seen.is_empty() {
                line = format!("{line} {seen}");
            }
            if !queued.is_empty() {
                line = format!("{line}  {queued}");
            }
            format!("{}\n", line.trim_end())
        })
        .collect()
}

/// `fiber radio`.
pub fn radio(broker: &Broker) -> Result<ExitCode> {
    show(broker, radio_text)
}

/// `fiber radio nodes`.
pub fn radio_nodes(broker: &Broker) -> Result<ExitCode> {
    show(broker, radio_nodes_text)
}

/// `fiber radio config [--channel N] [--keyed-join on|off]`.
pub fn radio_config(broker: &Broker, channel: Option<u8>, keyed_join: Option<bool>) -> Result<ExitCode> {
    runtime()?.block_on(request(
        broker,
        "radio.config",
        json!({"channel": channel, "keyed_join": keyed_join}),
        Duration::from_secs(5),
    ))?;
    show(broker, radio_text)
}

/// `fiber radio pair [--seconds N]`: opens a window and waits for it to end; Ctrl+C closes it.
pub fn radio_pair(broker: &Broker, seconds: u64) -> Result<ExitCode> {
    let runtime = runtime()?;
    runtime.block_on(request(
        broker,
        "radio.pairing_open",
        json!({"seconds": seconds}),
        Duration::from_secs(5),
    ))?;
    println!("Pairing for {seconds} s: put the node into pairing mode.");
    runtime.block_on(async {
        let (client, mut events) = broker.connect();
        client.subscribe("fiber/radio/pairing", QoS::AtLeastOnce).await?;
        // The window ends on the agent's side; this waits a little longer for its word.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds + 30);
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    let _ = client.disconnect().await;
                    request(broker, "radio.pairing_cancel", json!({}), Duration::from_secs(5)).await?;
                    println!("Closed.");
                    return Ok(ExitCode::FAILURE);
                }
                () = tokio::time::sleep_until(deadline) => {
                    bail!("the agent did not say how the window ended");
                }
                event = events.poll() => match event {
                    Err(e) => bail!("cannot reach the broker at {}:{}: {e}", broker.host, broker.port),
                    Ok(Event::Incoming(Packet::Publish(p))) => {
                        let Ok(value) = serde_json::from_slice::<Value>(&p.payload) else { continue };
                        if value["state"] != "idle" {
                            continue;
                        }
                        let _ = client.disconnect().await;
                        return Ok(match (value["joined"].as_str(), value["sas"].as_str()) {
                            (Some(node), Some(sas)) => {
                                println!("Paired {node}. Compare this code with the node's: {sas}");
                                ExitCode::SUCCESS
                            }
                            _ => {
                                println!("Nobody joined.");
                                ExitCode::FAILURE
                            }
                        });
                    }
                    Ok(_) => {}
                },
            }
        }
    })
}

/// One line of `fiber radio watch`, for a message on `topic`; `names` maps an address to its node's
/// name.
pub fn radio_watch_line(
    names: &BTreeMap<String, String>,
    topic: &str,
    payload: &[u8],
    now: u64,
) -> Option<String> {
    let value: Value = serde_json::from_slice(payload).ok()?;
    let at = clock(value["ts"].as_u64().unwrap_or(now));
    if topic == "fiber/radio/pairing" {
        let node = value["joined"].as_str()?;
        return Some(format!(
            "{at}  paired {node}, code {}",
            value["sas"].as_str().unwrap_or("?")
        ));
    }
    let rest = topic.strip_prefix("fiber/radio/nodes/")?;
    let (addr, rest) = rest.split_once('/')?;
    let name = names
        .get(addr)
        .filter(|n| !n.is_empty())
        .map_or(addr, String::as_str);
    let shown = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if let Some(prop) = rest.strip_prefix("state/") {
        return Some(format!("{at}  {name}  {prop}  {}", shown(&value["value"])));
    }
    let prop = rest.strip_prefix("event/")?;
    Some(format!(
        "{at}  {name}  {prop}  {} ({})",
        shown(&value["value"]),
        value["count"]
    ))
}

/// `fiber radio watch`.
pub fn radio_watch(broker: &Broker) -> Result<ExitCode> {
    let runtime = runtime()?;
    let topics = runtime.block_on(retained(broker))?;
    if let Some(code) = check_agent(&topics) {
        return Ok(code);
    }
    let mut names: BTreeMap<String, String> = topics
        .iter()
        .filter_map(|(topic, value)| {
            let addr = topic.strip_prefix("radio/nodes/")?;
            (!addr.contains('/')).then(|| (addr.to_owned(), value["name"].as_str().unwrap_or("").to_owned()))
        })
        .collect();
    runtime.block_on(watch(
        broker,
        &["fiber/radio/nodes/#", "fiber/radio/pairing"],
        |topic, payload| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            // A node's own topic: its name may have changed.
            if let Some(addr) = topic
                .strip_prefix("fiber/radio/nodes/")
                .filter(|a| !a.contains('/'))
            {
                if let Ok(value) = serde_json::from_slice::<Value>(payload) {
                    names.insert(addr.to_owned(), value["name"].as_str().unwrap_or("").to_owned());
                }
                return None;
            }
            radio_watch_line(&names, topic, payload, now)
        },
    ))?;
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_fiber_radio_prints() {
        let mut topics = Retained::new();
        assert!(radio_text(&topics).starts_with("The radio is not there"));
        topics.insert(
            "radio/status".into(),
            json!({"personality": "tower-fsk", "band": "eu868", "channel": 0, "gateway": "0x2a7f03c1",
                   "tower": "cb3b25c", "wire": "radio 0x5f3e", "can_send": true, "nodes": 2, "ts": 1}),
        );
        assert_eq!(
            radio_text(&topics),
            "Radio       tower-fsk, eu868, channel 0, gateway 0x2a7f03c1\nTOWER       cb3b25c, radio schema \
             0x5f3e\nNodes       2 of 16\nPairing     idle\n"
        );
        topics.insert(
            "radio/status".into(),
            json!({"personality": null, "band": "eu868", "channel": 0, "gateway": "0x2a7f03c1",
                   "tower": "cb3b25c", "wire": "radio 0x5f3e", "can_send": false, "nodes": 0, "ts": 1}),
        );
        topics.insert(
            "radio/pairing".into(),
            json!({"state": "open", "remaining": 42, "ts": 1}),
        );
        let text = radio_text(&topics);
        assert!(
            text.starts_with("Radio       the north bridge does not answer\n"),
            "{text}"
        );
        assert!(
            text.contains("Nodes       0 of 16; sending held back\n"),
            "{text}"
        );
        assert!(text.ends_with("Pairing     open, 42 s left\n"), "{text}");
    }

    #[test]
    fn what_fiber_radio_nodes_prints() {
        let mut topics = Retained::new();
        assert_eq!(radio_nodes_text(&topics), "No nodes.\n");
        topics.insert(
            "radio/nodes/0x1a2b3c4d".into(),
            json!({"addr": "0x1a2b3c4d", "name": "climate:0", "kind": "climate", "rssi": -71,
                   "last_seen": 1_700_000_531u64, "queued": 0}),
        );
        topics.insert(
            "radio/nodes/0x1a2b3c4d/state/temperature/system".into(),
            json!({"value": 21.5, "ts": 1}),
        );
        topics.insert(
            "radio/nodes/0x44444444".into(),
            json!({"addr": "0x44444444", "name": "", "kind": "", "rssi": null, "last_seen": null, "queued": 1}),
        );
        assert_eq!(
            radio_nodes_text(&topics),
            "0x1a2b3c4d  climate:0         climate   -71 dBm  22:22:11\n\
             0x44444444  -                 -         never     1 queued\n"
        );
    }

    #[test]
    fn what_fiber_radio_watch_prints() {
        let mut names = BTreeMap::new();
        names.insert("0x1a2b3c4d".to_owned(), "climate:0".to_owned());
        let line = |topic: &str, payload: &str| radio_watch_line(&names, topic, payload.as_bytes(), 0);
        assert_eq!(
            line(
                "fiber/radio/nodes/0x1a2b3c4d/state/temperature/system",
                r#"{"value":21.5,"ts":50531}"#
            )
            .unwrap(),
            "14:02:11  climate:0  temperature/system  21.5"
        );
        assert_eq!(
            line(
                "fiber/radio/nodes/0x1a2b3c4d/event/button",
                r#"{"value":"click","count":42,"counter":7,"ts":50560}"#
            )
            .unwrap(),
            "14:02:40  climate:0  button  click (42)"
        );
        assert_eq!(
            line(
                "fiber/radio/nodes/0x99999999/state/led",
                r#"{"value":true,"ts":50560}"#
            )
            .unwrap(),
            "14:02:40  0x99999999  led  true"
        );
        assert_eq!(
            line(
                "fiber/radio/pairing",
                r#"{"state":"idle","joined":"0x44444444","sas":"004321","ts":50582}"#
            )
            .unwrap(),
            "14:03:02  paired 0x44444444, code 004321"
        );
        assert_eq!(
            line("fiber/radio/pairing", r#"{"state":"open","remaining":60,"ts":1}"#),
            None
        );
    }

    #[test]
    fn what_fiber_thermometers_prints() {
        let mut topics = Retained::new();
        topics.insert("ports/1".into(), json!({"power": true, "thermometers": []}));
        assert_eq!(thermometers_text(&topics), "No thermometers.\n");
        topics.insert("ports/1".into(), json!({"power": true, "thermometers": null}));
        assert!(thermometers_text(&topics).starts_with("The 1-Wire buses were not found"));
        topics.insert(
            "thermometers/28-0000034a77e0".into(),
            json!({"port": 3, "temperature": 21.5, "ts": 1}),
        );
        topics.insert(
            "thermometers/28-00000b1c2d3e".into(),
            json!({"port": 5, "temperature": null, "ts": 1}),
        );
        topics.insert(
            "thermometers/28-0000034a5b1c".into(),
            json!({"port": 3, "temperature": 21.44, "ts": 1}),
        );
        assert_eq!(
            thermometers_text(&topics),
            "Port 3      28-0000034a5b1c  21.44 °C\n\
             Port 3      28-0000034a77e0  21.50 °C\n\
             Port 5      28-00000b1c2d3e  no reading\n"
        );
    }

    #[test]
    fn what_fiber_display_prints() {
        let mut topics = Retained::new();
        assert_eq!(display_text(&topics), "Backlight   not known\nMessage     none\n");
        topics.insert(
            "display".into(),
            json!({"backlight": 80, "message": "Back in\n5 minutes", "ts": 1}),
        );
        assert_eq!(
            display_text(&topics),
            "Backlight   80 %\nMessage     Back in / 5 minutes\n"
        );
    }

    #[test]
    fn every_request_has_its_own_id() {
        let ids: std::collections::BTreeSet<String> = (0..100).map(|_| request_id()).collect();
        assert_eq!(ids.len(), 100);
        // As the agent takes them: at most 64 characters of A-Z a-z 0-9 _ . -
        assert!(
            ids.iter().all(
                |id| id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
            )
        );
    }

    #[test]
    fn the_clock() {
        assert_eq!(clock(1_794_952_515), "21:55:15");
        assert_eq!(clock(0), "00:00:00");
    }

    #[test]
    fn the_broker() {
        let local = Broker::new(None, None).unwrap();
        assert_eq!((local.host.as_str(), local.port), ("127.0.0.1", 1883));
        let lan = Broker::new(Some("fiber-abcdef12.local"), Some("pi".into())).unwrap();
        assert_eq!(lan.port, 1884, "with a login: the listener for the network");
        assert_eq!(Broker::new(Some("10.0.0.5:1999"), None).unwrap().port, 1999);
        assert!(Broker::new(Some("host:x"), None).is_err());
    }

    #[test]
    fn lines() {
        assert_eq!(ports_line(&[true; 8]), "1 2 3 4 5 6 7 8 on");
        assert_eq!(ports_line(&[false; 8]), "1 2 3 4 5 6 7 8 off");
        let mut some = [true; 8];
        some[2] = false;
        assert_eq!(ports_line(&some), "1 2 4 5 6 7 8 on, 3 off");
        let power = json!({"source": "poe", "input": 12.08, "battery": 4.1});
        assert_eq!(power_line(Some(&power)), "PoE, input 12.08 V, battery 4.10 V");
        let power = json!({"source": "poe", "input": 13.62, "battery": null});
        assert_eq!(power_line(Some(&power)), "PoE, input 13.62 V, no battery");
        assert_eq!(
            power_line(Some(&json!({"source": "unknown", "input": null}))),
            "not known"
        );
    }

    #[test]
    fn the_status() {
        let mut topics = Retained::new();
        topics.insert(
            "status".into(),
            json!({"state": "online", "schema": 1, "version": "0.2.0", "hardware": "FIBER-R2 2.1", "serial": "1000", "name": "fiber-1"}),
        );
        topics.insert(
            "power".into(),
            json!({"source": "battery", "input": 0.4, "battery": 3.9}),
        );
        for p in 1..=8 {
            topics.insert(format!("ports/{p}"), json!({"power": p != 8, "led": "auto"}));
        }
        topics.insert(
            "firmware/south".into(),
            json!({"state": "running", "version": "0.2.0", "crc": "0x6f1c2a90", "carried": "0x6f1c2a90"}),
        );
        assert_eq!(check_agent(&topics), None);
        assert_eq!(
            status_text(&topics),
            "FIBER       fiber-1, FIBER-R2 2.1, serial 1000\nAgent       online, 0.2.0, schema 1\n\
             Power       battery, input 0.40 V, battery 3.90 V\nPorts       1 2 3 4 5 6 7 on, 8 off\n\
             South       running 0.2.0, image 0x6f1c2a90 (the one carried)\n"
        );
        topics.insert("status".into(), json!({"state": "offline", "schema": 1}));
        assert_eq!(check_agent(&topics), Some(ExitCode::from(EXIT_NO_AGENT)));
        topics.insert("status".into(), json!({"state": "online", "schema": 2}));
        assert_eq!(check_agent(&topics), Some(ExitCode::from(EXIT_SCHEMA)));
    }
}
