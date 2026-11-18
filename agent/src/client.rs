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

/// Sends a request and waits for its answer.
async fn request(broker: &Broker, op: &str, params: Value, wait: Duration) -> Result<Value> {
    let (client, mut events) = broker.connect();
    let id = format!(
        "cli-{}-{}",
        std::process::id(),
        tokio::time::Instant::now().elapsed().as_nanos()
    );
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

#[cfg(test)]
mod tests {
    use super::*;

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
