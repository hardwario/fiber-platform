//! The payloads of the agent's topics (`docs/mqtt-api.md`, "Topics").

use serde_json::{Value, json};

use super::leds::Setting;
use super::state::{Chip, State};

/// The schema number of the whole tree of topics and payloads.
pub const SCHEMA: u32 = 1;

/// What `status` says about the FIBER.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub version: String,
    pub hardware: String,
    pub serial: String,
    pub name: String,
}

pub fn status(identity: &Identity, ts: u64) -> Value {
    json!({
        "state": "online",
        "schema": SCHEMA,
        "version": identity.version,
        "hardware": identity.hardware,
        "serial": identity.serial,
        "name": identity.name,
        "ts": ts,
    })
}

/// The last will on `status`.
pub fn offline() -> Value {
    json!({"state": "offline", "schema": SCHEMA})
}

/// A voltage with two decimals.
fn volts(v: f64) -> Value {
    json!((v * 100.0).round() / 100.0)
}

pub fn power(state: &State, ts: u64) -> Value {
    let (input, battery) = match state.power.volts {
        Some(v) => (volts(v.input), volts(v.battery)),
        None => (Value::Null, Value::Null),
    };
    json!({"source": state.power.source.name(), "input": input, "battery": battery, "ts": ts})
}

pub fn power_event(source: &str, ts: u64) -> Value {
    json!({"event": source, "ts": ts})
}

/// `ports/{port}`, for port 1 to 8.
pub fn port(state: &State, port: u8, ts: u64) -> Value {
    let led = match state.leds[usize::from(port)] {
        Setting::Auto => "auto",
        Setting::Set { .. } => "set",
    };
    json!({"power": state.port_on(port), "led": led, "ts": ts})
}

/// `firmware/{chip}`.
pub fn firmware(chip: &Chip, ts: u64) -> Value {
    let hello = chip.hello;
    json!({
        "state": chip.state.name(),
        "version": hello.map(|h| format!("{}.{}.{}", h.version[0], h.version[1], h.version[2])),
        "crc": hello.map(|h| format!("0x{:08x}", h.image_crc)),
        "carried": format!("0x{:08x}", chip.carried),
        "reset": hello.map(|h| reset_name(h.reset)),
        "fault": hello.and_then(|h| h.fault).map(|f| json!({"code": f.code, "address": format!("0x{:08x}", f.address)})),
        "ts": ts,
    })
}

fn reset_name(reset: fiber_protocol::south::Reset) -> &'static str {
    use fiber_protocol::south::Reset;
    match reset {
        Reset::PowerOn => "power_on",
        Reset::Pin => "pin",
        Reset::Watchdog => "watchdog",
        Reset::Software => "software",
        Reset::BrownOut => "brown_out",
        Reset::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::power::Volts;
    use crate::agent::state::{FirmwareState, Settings};
    use fiber_protocol::south::{Fault, Hello, Reset};

    #[test]
    fn payloads() {
        let identity = Identity {
            version: "0.2.0".into(),
            hardware: "FIBER-R2 2.1".into(),
            serial: "10000000abcdef12".into(),
            name: "fiber-abcdef12".into(),
        };
        assert_eq!(
            status(&identity, 5).to_string(),
            r#"{"hardware":"FIBER-R2 2.1","name":"fiber-abcdef12","schema":1,"serial":"10000000abcdef12","state":"online","ts":5,"version":"0.2.0"}"#
        );
        assert_eq!(offline().to_string(), r#"{"schema":1,"state":"offline"}"#);

        let mut state = State::new(Settings { dividers: true }, 0x6f1c_2a90);
        assert_eq!(
            power(&state, 5).to_string(),
            r#"{"battery":null,"input":null,"source":"unknown","ts":5}"#
        );
        state.power.update(Some(Volts {
            input: 12.0849,
            battery: 4.1111,
        }));
        assert_eq!(
            power(&state, 5).to_string(),
            r#"{"battery":4.11,"input":12.08,"source":"poe","ts":5}"#
        );
        state.ports = 0b11;
        assert_eq!(
            port(&state, 2, 5).to_string(),
            r#"{"led":"auto","power":true,"ts":5}"#
        );
        assert_eq!(port(&state, 3, 5)["power"], false);

        assert_eq!(firmware(&state.south, 5)["state"], "silent");
        assert_eq!(firmware(&state.south, 5)["version"], Value::Null);
        state.south.state = FirmwareState::Running;
        state.south.hello = Some(Hello {
            version: [0, 2, 0],
            image_crc: 0x6f1c_2a90,
            uid: [0; 12],
            reset: Reset::Watchdog,
            fault: Some(Fault {
                code: 2,
                address: 0x0800_1f3c,
            }),
            option_bytes: 0,
            vrefint_cal: 1655,
        });
        let f = firmware(&state.south, 5);
        assert_eq!(f["version"], "0.2.0");
        assert_eq!(f["crc"], "0x6f1c2a90");
        assert_eq!(f["carried"], "0x6f1c2a90");
        assert_eq!(f["reset"], "watchdog");
        assert_eq!(f["fault"], json!({"code": 2, "address": "0x08001f3c"}));
    }
}
