//! Requests on `fiber/cmd` and their answers on `fiber/rsp/{id}` (`docs/mqtt-api.md`, "Requests").

use std::collections::VecDeque;

use serde_json::{Value, json};

use super::buzzer::Pattern;
use super::leds::{self, Colour, Led, Setting};
use super::state::State;

/// The most bytes of a request.
pub const MAX_REQUEST: usize = 4096;
/// How many ids are remembered, to drop a request delivered twice.
pub const REMEMBERED: usize = 64;

/// A request.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub id: String,
    pub op: String,
    pub params: Value,
}

/// An id: 1 to 64 characters of `A-Z a-z 0-9 _ . -`.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

/// Reads a request; a request that cannot be answered (not JSON, no valid `id`, no `op`, too
/// long) is dropped, and the reason is only for the log.
pub fn parse(payload: &[u8]) -> Result<Request, &'static str> {
    if payload.len() > MAX_REQUEST {
        return Err("longer than 4096 bytes");
    }
    let value: Value = serde_json::from_slice(payload).map_err(|_| "not JSON")?;
    let id = value.get("id").and_then(Value::as_str).ok_or("no id")?;
    if !valid_id(id) {
        return Err("an id that breaks the rules");
    }
    let op = value.get("op").and_then(Value::as_str).ok_or("no op")?;
    Ok(Request {
        id: id.to_owned(),
        op: op.to_owned(),
        params: value.get("params").cloned().unwrap_or(Value::Null),
    })
}

/// The last ids, so that a request delivered twice runs once.
#[derive(Debug, Default)]
pub struct RecentIds(VecDeque<String>);

impl RecentIds {
    /// Whether `id` is new; it is remembered.
    pub fn is_new(&mut self, id: &str) -> bool {
        if self.0.iter().any(|seen| seen == id) {
            return false;
        }
        if self.0.len() == REMEMBERED {
            self.0.pop_front();
        }
        self.0.push_back(id.to_owned());
        true
    }
}

/// An answer, as `fiber/rsp/{id}` carries it.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub id: String,
    pub result: Result<Option<Value>, String>,
}

impl Answer {
    pub fn ok(id: &str, data: Option<Value>) -> Self {
        Answer {
            id: id.to_owned(),
            result: Ok(data),
        }
    }

    pub fn error(id: &str, error: impl Into<String>) -> Self {
        Answer {
            id: id.to_owned(),
            result: Err(error.into()),
        }
    }

    pub fn payload(&self) -> String {
        match &self.result {
            Ok(Some(data)) => json!({"id": self.id, "ok": true, "data": data}),
            Ok(None) => json!({"id": self.id, "ok": true}),
            Err(error) => json!({"id": self.id, "ok": false, "error": error}),
        }
        .to_string()
    }
}

/// What a request comes to.
#[derive(Clone, Debug, PartialEq)]
pub enum Handled {
    /// Answered at once.
    Answer(Answer),
    /// The ports in `mask` are off now; they go on again after `seconds`, and then the answer.
    Cycle { id: String, mask: u8, seconds: u64 },
    /// A chip to flash; answered when it is done.
    Flash { id: String, chip: Chip },
    /// A pattern for the buzzer; answered when it is handed on.
    Beep { id: String, pattern: Pattern },
}

/// A chip, as `firmware.flash` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    South,
    North,
}

fn invalid(id: &str, what: &str) -> Handled {
    Handled::Answer(Answer::error(id, format!("invalid argument: {what}")))
}

/// The ports, as `ports.power` answers them.
pub fn ports_data(state: &State) -> Value {
    json!({"ports": (1..=8).map(|p| state.port_on(p)).collect::<Vec<_>>()})
}

/// Runs a request on the state.
pub fn handle(state: &mut State, request: &Request) -> Handled {
    let id = request.id.as_str();
    let params = &request.params;
    match request.op.as_str() {
        "ports.power" => {
            let mask = match params.get("port") {
                Some(Value::String(all)) if all == "all" => 0xFF,
                Some(port) => match port.as_u64() {
                    Some(p @ 1..=8) => 1u8 << (p - 1),
                    _ => return invalid(id, "port is 1 to 8, or \"all\""),
                },
                None => return invalid(id, "port is missing"),
            };
            match params.get("power") {
                Some(Value::Bool(true)) => state.ports |= mask,
                Some(Value::Bool(false)) => state.ports &= !mask,
                Some(Value::String(cycle)) if cycle == "cycle" => {
                    let seconds = match params.get("seconds") {
                        None | Some(Value::Null) => 5,
                        Some(s) => match s.as_u64() {
                            Some(s @ 1..=60) => s,
                            _ => return invalid(id, "seconds is 1 to 60"),
                        },
                    };
                    state.ports &= !mask;
                    return Handled::Cycle {
                        id: id.to_owned(),
                        mask,
                        seconds,
                    };
                }
                _ => return invalid(id, "power is true, false or \"cycle\""),
            }
            Handled::Answer(Answer::ok(id, Some(ports_data(state))))
        }
        "leds.set" => {
            let Some(led) = params.get("led").and_then(Value::as_str).and_then(Led::parse) else {
                return invalid(id, "led is \"power\" or \"port1\" to \"port8\"");
            };
            let Some(colour) = params
                .get("colour")
                .and_then(Value::as_str)
                .and_then(Colour::parse)
            else {
                return invalid(id, "colour is off, green, red, amber or yellow");
            };
            let blink = match params.get("blink") {
                None | Some(Value::Null) => None,
                Some(b) => match (
                    b.get("on").and_then(Value::as_u64),
                    b.get("off").and_then(Value::as_u64),
                ) {
                    (Some(on), Some(off)) => match leds::blink_ticks(on, off) {
                        Some(ticks) => Some(ticks),
                        None => return invalid(id, "blink on and off are multiples of 20 from 20 to 5100"),
                    },
                    _ => return invalid(id, "blink is {\"on\": ms, \"off\": ms}"),
                },
            };
            let Some(setting) = leds::set(led, colour, blink) else {
                return invalid(id, "that LED has no such colour");
            };
            state.leds[led.index()] = setting;
            Handled::Answer(Answer::ok(id, None))
        }
        "leds.auto" => {
            match params.get("led").and_then(Value::as_str) {
                Some("all") => state.leds = [Setting::Auto; 9],
                Some(name) => match Led::parse(name) {
                    Some(led) => state.leds[led.index()] = Setting::Auto,
                    None => return invalid(id, "led is \"power\", \"port1\" to \"port8\", or \"all\""),
                },
                None => return invalid(id, "led is missing"),
            }
            Handled::Answer(Answer::ok(id, None))
        }
        "firmware.flash" => match params.get("chip").and_then(Value::as_str) {
            Some("south") => Handled::Flash {
                id: id.to_owned(),
                chip: Chip::South,
            },
            Some("north") => Handled::Answer(Answer::error(
                id,
                "the north bridge's firmware is not written yet",
            )),
            _ => invalid(id, "chip is \"south\" or \"north\""),
        },
        "buzzer.beep" => {
            let pattern = match params.get("pattern") {
                None | Some(Value::Null) => Some(Pattern::Beep),
                Some(name) => name.as_str().and_then(Pattern::parse),
            };
            match pattern {
                Some(pattern) => Handled::Beep {
                    id: id.to_owned(),
                    pattern,
                },
                None => invalid(id, "pattern is beep, double, long or alarm"),
            }
        }
        "display.message" => {
            let seconds = match params.get("seconds") {
                None | Some(Value::Null) => None,
                Some(s) => match s.as_u64() {
                    Some(s @ 1..=86_400) => Some(s),
                    _ => return invalid(id, "seconds is 1 to 86400"),
                },
            };
            match params.get("text") {
                None | Some(Value::Null) => {
                    state.display.message = None;
                    state.display.message_until = None;
                }
                Some(Value::String(text)) => {
                    // A message lights the backlight, so that it is seen (D27).
                    state.display.lit_at = tokio::time::Instant::now();
                    state.display.message = Some(text.clone());
                    state.display.message_until =
                        seconds.map(|s| tokio::time::Instant::now() + std::time::Duration::from_secs(s));
                }
                Some(_) => return invalid(id, "text is a string, or null"),
            }
            Handled::Answer(Answer::ok(id, None))
        }
        "display.backlight" => match params.get("level").and_then(Value::as_u64) {
            Some(level @ 0..=100) => {
                state.display.backlight = level as u8;
                Handled::Answer(Answer::ok(id, None))
            }
            _ => invalid(id, "level is 0 to 100"),
        },
        _ => Handled::Answer(Answer::error(id, "unknown op")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::Settings;

    fn state() -> State {
        State::new(Settings { dividers: true }, Some(1))
    }

    fn request(op: &str, params: Value) -> Request {
        Request {
            id: "a1".into(),
            op: op.into(),
            params,
        }
    }

    fn error(handled: Handled) -> String {
        match handled {
            Handled::Answer(Answer { result: Err(e), .. }) => e,
            other => panic!("not an error: {other:?}"),
        }
    }

    #[test]
    fn reading_requests() {
        let r = parse(br#"{"id":"3f-2b.8_c","op":"ports.power","params":{"port":3,"power":false}}"#).unwrap();
        assert_eq!(r.op, "ports.power");
        assert_eq!(r.params["port"], 3);
        assert_eq!(
            parse(br#"{"id":"x","op":"leds.auto"}"#).unwrap().params,
            Value::Null
        );
        assert_eq!(parse(b"not json"), Err("not JSON"));
        assert_eq!(parse(br#"{"op":"x"}"#), Err("no id"));
        assert_eq!(
            parse(br#"{"id":"a b","op":"x"}"#),
            Err("an id that breaks the rules")
        );
        assert_eq!(parse(br#"{"id":"a"}"#), Err("no op"));
        assert!(!valid_id(&"x".repeat(65)));
    }

    #[test]
    fn a_request_delivered_twice_runs_once() {
        let mut ids = RecentIds::default();
        assert!(ids.is_new("a"));
        assert!(!ids.is_new("a"));
        for i in 0..REMEMBERED {
            ids.is_new(&i.to_string());
        }
        assert!(ids.is_new("a"), "forgotten after 64 others");
    }

    #[test]
    fn answers() {
        assert_eq!(Answer::ok("u1", None).payload(), r#"{"id":"u1","ok":true}"#);
        assert_eq!(
            Answer::error("u1", "not found").payload(),
            r#"{"error":"not found","id":"u1","ok":false}"#
        );
    }

    #[test]
    fn ports() {
        let mut s = state();
        let handled = handle(
            &mut s,
            &request("ports.power", json!({"port": 3, "power": false})),
        );
        let Handled::Answer(Answer {
            result: Ok(Some(data)),
            ..
        }) = handled
        else {
            panic!("{handled:?}")
        };
        assert_eq!(
            data["ports"],
            json!([true, true, false, true, true, true, true, true])
        );
        assert_eq!(s.ports, 0b1111_1011);
        handle(
            &mut s,
            &request("ports.power", json!({"port": "all", "power": true})),
        );
        assert_eq!(s.ports, 0xFF);
        assert_eq!(
            handle(
                &mut s,
                &request("ports.power", json!({"port": 2, "power": "cycle"}))
            ),
            Handled::Cycle {
                id: "a1".into(),
                mask: 0b10,
                seconds: 5
            }
        );
        assert_eq!(s.ports, 0b1111_1101, "off now, on again later");
        assert!(
            error(handle(
                &mut s,
                &request("ports.power", json!({"port": 9, "power": true}))
            ))
            .starts_with("invalid argument: port")
        );
        assert!(
            error(handle(
                &mut s,
                &request("ports.power", json!({"port": 1, "power": "on"}))
            ))
            .contains("power")
        );
        assert!(
            error(handle(
                &mut s,
                &request("ports.power", json!({"port": 1, "power": "cycle", "seconds": 61}))
            ))
            .contains("seconds")
        );
    }

    #[test]
    fn leds() {
        let mut s = state();
        let ok = handle(
            &mut s,
            &request(
                "leds.set",
                json!({"led": "port2", "colour": "amber", "blink": {"on": 500, "off": 500}}),
            ),
        );
        assert_eq!(ok, Handled::Answer(Answer::ok("a1", None)));
        assert_eq!(
            s.leds[2],
            Setting::Set {
                colour: Colour::Amber,
                blink: Some((25, 25))
            }
        );
        assert!(
            error(handle(
                &mut s,
                &request("leds.set", json!({"led": "power", "colour": "red"}))
            ))
            .contains("no such colour")
        );
        assert!(
            error(handle(
                &mut s,
                &request(
                    "leds.set",
                    json!({"led": "port2", "colour": "red", "blink": {"on": 7, "off": 20}})
                )
            ))
            .contains("multiples of 20")
        );
        handle(&mut s, &request("leds.auto", json!({"led": "all"})));
        assert_eq!(s.leds, [Setting::Auto; 9]);
    }

    #[test]
    fn other_ops() {
        let mut s = state();
        assert_eq!(
            handle(&mut s, &request("firmware.flash", json!({"chip": "south"}))),
            Handled::Flash {
                id: "a1".into(),
                chip: Chip::South
            }
        );
        assert!(
            error(handle(
                &mut s,
                &request("firmware.flash", json!({"chip": "north"}))
            ))
            .contains("not written yet")
        );
        assert_eq!(
            error(handle(&mut s, &request("reboot", Value::Null))),
            "unknown op"
        );
    }

    #[test]
    fn the_front_panel() {
        let mut state = state();
        assert!(matches!(
            handle(&mut state, &request("buzzer.beep", json!({"pattern": "long"}))),
            Handled::Beep {
                pattern: Pattern::Long,
                ..
            }
        ));
        assert!(
            error(handle(
                &mut state,
                &request("buzzer.beep", json!({"pattern": "siren"}))
            ))
            .starts_with("invalid argument")
        );
        handle(&mut state, &request("display.message", json!({"text": "a\nb"})));
        assert_eq!(state.display.message.as_deref(), Some("a\nb"));
        assert_eq!(state.display.message_until, None);
        handle(&mut state, &request("display.message", json!({"text": null})));
        assert_eq!(state.display.message, None);
        assert!(
            error(handle(
                &mut state,
                &request("display.message", json!({"text": "a", "seconds": 0}))
            ))
            .starts_with("invalid argument")
        );
        handle(&mut state, &request("display.backlight", json!({"level": 30})));
        assert_eq!(state.display.backlight, 30);
        assert!(
            error(handle(
                &mut state,
                &request("display.backlight", json!({"level": 101}))
            ))
            .starts_with("invalid argument")
        );
    }
}
