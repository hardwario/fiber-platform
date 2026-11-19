//! TOWER's property values as the topics carry them (`docs/mqtt-api.md`, "Node properties"), with
//! `tower-protocol`'s own rules: its types, its `parse` and its fixed-point writing, so that a value
//! reads the same here as in TOWER's bridge.

use serde_json::Value;
use tower_protocol::props::{Access, Prop, Type, write_fixed};

/// Where a property is published: `state` (retained) or `event`.
pub fn topic_kind(prop: &Prop) -> &'static str {
    match prop.access {
        Access::Event => "event",
        _ => "state",
    }
}

/// Whether a client may set it over the radio: controls and settings, not local settings.
pub fn settable(prop: &Prop) -> bool {
    matches!(prop.access, Access::Control | Access::Param)
}

/// The JSON of a wire value: a boolean, a number with its decimals exact, or a name (a value
/// outside the names stays a number).
pub fn json(ty: &Type, value: i32) -> Value {
    match *ty {
        Type::Bool => Value::Bool(value != 0),
        Type::Number { decimals: 0, .. } => Value::from(value),
        Type::Number { decimals, .. } => {
            let mut text = String::new();
            let _ = write_fixed(&mut text, value, decimals);
            serde_json::from_str::<serde_json::Number>(&text).map_or(Value::from(value), Value::Number)
        }
        Type::Enum(names) => match usize::try_from(value).ok().and_then(|i| names.get(i)) {
            Some(name) => Value::from(*name),
            None => Value::from(value),
        },
    }
}

/// The wire value of a client's `set` payload: the value as JSON (`true`, `21.5`, `"auto"`), as
/// `{"value": ...}`, or as plain text (`on`, `21.5`, `auto`, `0xff8000`). `Err` says why not.
pub fn parse_set(prop: &Prop, payload: &[u8]) -> Result<i32, String> {
    if !settable(prop) {
        return Err(format!("{} cannot be set over the radio", prop.name));
    }
    let text = std::str::from_utf8(payload)
        .map_err(|_| "not UTF-8".to_owned())?
        .trim();
    let text = match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => match map.get("value") {
            Some(value) => plain(value),
            None => return Err("an object without \"value\"".into()),
        },
        Ok(value) => plain(&value),
        // Not JSON: plain text.
        Err(_) => text.to_owned(),
    };
    prop.ty
        .parse(&text)
        .ok_or_else(|| format!("{text:?} is not a value of {}", prop.name))
}

/// A JSON value as the text `Type::parse` takes.
fn plain(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_protocol::props;

    fn prop(kind: &str, name: &str) -> &'static Prop {
        props::kind(kind).unwrap().prop_named(name).unwrap()
    }

    #[test]
    fn values_as_json() {
        let temperature = prop("climate", "temperature/system");
        assert_eq!(json(&temperature.ty, 2150), serde_json::json!(21.5));
        assert_eq!(json(&temperature.ty, -25), serde_json::json!(-0.25));
        assert_eq!(json(&temperature.ty, 2100), serde_json::json!(21.0));
        assert_eq!(json(&prop("climate", "led").ty, 1), serde_json::json!(true));
        let accel = prop("climate", "accel");
        assert_eq!(json(&accel.ty, 1), serde_json::json!("low"));
        assert_eq!(json(&accel.ty, 99), serde_json::json!(99));
        assert_eq!(
            json(&prop("climate", "heartbeat").ty, 900),
            serde_json::json!(900)
        );
        assert_eq!(topic_kind(prop("climate", "button")), "event");
        assert_eq!(topic_kind(temperature), "state");
    }

    #[test]
    fn a_set_in_every_form() {
        let heartbeat = prop("climate", "heartbeat");
        assert_eq!(parse_set(heartbeat, b"600"), Ok(600));
        assert_eq!(parse_set(heartbeat, b"{\"value\":600}"), Ok(600));
        assert_eq!(parse_set(heartbeat, b" 600\n"), Ok(600));
        assert!(parse_set(heartbeat, b"5").is_err(), "below its range");
        let delta = prop("climate", "report-delta");
        assert_eq!(parse_set(delta, b"0.25"), Ok(25));
        assert!(parse_set(delta, b"0.255").is_err(), "more decimals than it has");
        let led = prop("climate", "led");
        assert_eq!(parse_set(led, b"false"), Ok(0));
        assert_eq!(parse_set(led, b"on"), Ok(1));
        let accel = prop("climate", "accel");
        assert_eq!(parse_set(accel, b"\"high\""), Ok(3));
        assert_eq!(parse_set(accel, b"high"), Ok(3));
        assert!(parse_set(accel, b"loud").is_err());
        let color = prop("control", "color");
        assert_eq!(parse_set(color, b"0xff8000"), Ok(0xff8000));
        // Not settable over the radio: a state, an event, a local setting.
        assert!(parse_set(prop("climate", "temperature/system"), b"20").is_err());
        assert!(parse_set(prop("climate", "button"), b"click").is_err());
        assert!(parse_set(prop("climate", "band"), b"us915").is_err());
        assert!(parse_set(heartbeat, b"{\"other\":1}").is_err());
    }
}
