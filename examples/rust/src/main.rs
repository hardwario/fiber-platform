//! Prints each thermometer's reading on the FIBER as it changes, and with a limit sounds the
//! FIBER's buzzer when a probe goes above it (docs/examples.md).
//!
//! ```sh
//! cargo run --release          # the readings
//! cargo run --release -- 30    # and the alarm above 30 °C
//! ```
//!
//! `FIBER_HOST`, `FIBER_PORT`, `FIBER_USER` and `FIBER_PASSWORD` say where the broker is (on the
//! FIBER: 127.0.0.1:1883, no login).

use std::collections::HashSet;
use std::time::Duration;

use rumqttc::{Client, Event, MqttOptions, Packet, QoS};
use serde_json::{Value, json};

/// The API this was written for (docs/mqtt-api.md, "The rules").
const SCHEMA: u64 = 1;
/// How far below the limit a probe has to go before it can sound the alarm again: a reading that
/// wavers at the limit would otherwise sound it every round.
const HYSTERESIS: f64 = 0.5;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let limit: Option<f64> = std::env::args().nth(1).map(|a| a.parse()).transpose()?;
    let host = std::env::var("FIBER_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = std::env::var("FIBER_PORT")
        .unwrap_or_else(|_| "1883".into())
        .parse()?;

    let mut options = MqttOptions::new(format!("thermometers-{}", std::process::id()), host, port);
    options.set_keep_alive(Duration::from_secs(30));
    if let Ok(user) = std::env::var("FIBER_USER") {
        options.set_credentials(user, std::env::var("FIBER_PASSWORD").unwrap_or_default());
    }
    let (client, mut connection) = Client::new(options, 10);

    // The probes above the limit now, and the requests sent, to know their answers.
    let mut above: HashSet<String> = HashSet::new();
    let mut sent = 0u32;
    for event in connection.iter() {
        let packet = match event {
            Ok(Event::Incoming(packet)) => packet,
            Ok(Event::Outgoing(_)) => continue,
            Err(e) => {
                // The iterator connects again by itself; wait a little before it does.
                eprintln!("The broker: {e}");
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        match packet {
            // Here, so that the subscriptions come back after a reconnect. The retained topics
            // give the latest state at once; the answers come on `fiber/rsp/{id}`.
            Packet::ConnAck(_) => {
                client.try_subscribe("fiber/status", QoS::AtLeastOnce)?;
                client.try_subscribe("fiber/thermometers/+", QoS::AtLeastOnce)?;
                client.try_subscribe("fiber/rsp/+", QoS::AtLeastOnce)?;
            }
            Packet::Publish(message) => {
                let topic = message.topic.as_str();
                if topic == "fiber/status" {
                    let status: Value = serde_json::from_slice(&message.payload)?;
                    if status["schema"].as_u64() != Some(SCHEMA) {
                        return Err(format!(
                            "the agent speaks schema {}, this example {SCHEMA}",
                            status["schema"]
                        )
                        .into());
                    }
                } else if let Some(id) = topic.strip_prefix("fiber/rsp/") {
                    // Only the answers to this program's requests.
                    if id.starts_with(&format!("rust-{}-", std::process::id())) {
                        println!("The agent answers: {}", String::from_utf8_lossy(&message.payload));
                    }
                } else if let Some(probe) = topic.strip_prefix("fiber/thermometers/") {
                    if message.payload.is_empty() {
                        // An empty message: the probe is gone.
                        println!("{probe}: gone");
                        above.remove(probe);
                        continue;
                    }
                    let reading: Value = serde_json::from_slice(&message.payload)?;
                    let Some(temperature) = reading["temperature"].as_f64() else {
                        println!("{probe} on port {}: no reading", reading["port"]);
                        continue;
                    };
                    println!("{probe} on port {}: {temperature:.2} °C", reading["port"]);
                    let Some(limit) = limit else { continue };
                    if temperature > limit && above.insert(probe.to_string()) {
                        sent += 1;
                        let request = json!({
                            // New for every request: the agent drops an id it has seen.
                            "id": format!("rust-{}-{sent}", std::process::id()),
                            "op": "buzzer.beep",
                            "params": {"pattern": "alarm"},
                        });
                        println!("{probe} is above {limit} °C: sounding the alarm");
                        // Never retained: a retained request is cleared and not run.
                        client.try_publish("fiber/cmd", QoS::AtLeastOnce, false, request.to_string())?;
                    } else if temperature < limit - HYSTERESIS {
                        above.remove(probe);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}
