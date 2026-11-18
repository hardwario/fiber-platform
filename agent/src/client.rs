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

/// `fiber power`.
pub fn power(broker: &Broker) -> Result<ExitCode> {
    show(broker, |topics| {
        format!("Power       {}\n", power_line(topics.get("power")))
    })
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
