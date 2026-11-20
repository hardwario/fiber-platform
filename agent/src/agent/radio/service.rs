//! What the radio does with each message from the north bridge, each request and each client's
//! `set` (`docs/mqtt-api.md`, "The radio"). Nothing here does I/O: every call returns the
//! [`Effect`]s, which the agent's loop carries out.

use std::collections::{HashMap, VecDeque};

use fiber_protocol::north::{self, Band, Bytes, Message, NodeAdd, NodeRef, Outcome, Personality, Sent};
use serde_json::{Value, json};
use tower_protocol::props::{self, Access, Kind};
use tower_protocol::radio::{
    MAX_RADIO_PAYLOAD, NodeCmd, NodeMsg, RADIO_TAG, decode_node_msg, encode_node_cmd,
};

use super::store::{Store, StoredNode, address, parse_address, parse_key};
use super::values;

/// The most nodes a FIBER knows (the north bridge's table).
pub const MAX_NODES: usize = 16;
/// A node's name, at most.
pub const NAME_MAX: usize = 16;
/// The nodes' counters are written to disk at most this often, in seconds: often enough that
/// little could be replayed after a restart, seldom enough for the SD card. (TOWER's gateway
/// keeps a node's counter every 32 frames.)
const SAVE_EVERY: u64 = 60;
/// A pairing window the north bridge never closed is closed this long after its end.
const PAIRING_GRACE: u64 = 20;

/// What the loop is to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Publish under `fiber/` (the topic is given without it).
    Publish {
        topic: String,
        payload: String,
        retain: bool,
    },
    /// Send to the north bridge.
    North(Message),
    /// Write the store.
    Save,
    /// Say it in the agent's log.
    Say(String),
}

fn publish(topic: String, payload: Value, retain: bool) -> Effect {
    Effect::Publish {
        topic,
        payload: payload.to_string(),
        retain,
    }
}

fn clear(topic: String) -> Effect {
    Effect::Publish {
        topic,
        payload: String::new(),
        retain: true,
    }
}

/// Something waiting to go to a node, in the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Set { id: u8, value: i32 },
    Time,
    Describe,
}

/// What the agent knows of a node besides the store.
#[derive(Default)]
struct Live {
    rssi: Option<i16>,
    last_seen: Option<u64>,
    uplinks: u32,
    sleeping: bool,
    /// Waiting in the agent: one `Set` for each property (the newest value), a `Time`, a
    /// `Describe`.
    waiting: VecDeque<Item>,
    /// On the north bridge, which holds one message for each node.
    on_north: Option<Item>,
    /// A `Describe` was asked for since the agent started.
    asked: bool,
}

impl Live {
    fn queued(&self) -> usize {
        self.waiting.len() + usize::from(self.on_north.is_some())
    }

    fn push(&mut self, item: Item) {
        let same = |w: &Item| match (w, &item) {
            (Item::Set { id: a, .. }, Item::Set { id: b, .. }) => a == b,
            (a, b) => a == b,
        };
        match self.waiting.iter_mut().find(|w| same(w)) {
            Some(slot) => *slot = item,
            None => self.waiting.push_back(item),
        }
    }
}

/// The radio in short, for the front panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub answering: bool,
    pub channel: u8,
    pub nodes: usize,
    /// The node heard last: its name or address, its strength, when.
    pub last: Option<(String, i16, u64)>,
    pub pairing_until: Option<u64>,
}

/// The radio service.
pub struct Radio {
    store: Store,
    live: HashMap<u32, Live>,
    /// The pinned TOWER revision, as `radio/status` shows it.
    tower: String,
    north_up: bool,
    personality: Option<Personality>,
    can_send: bool,
    /// When the open pairing window ends.
    pairing_until: Option<u64>,
    /// Counters taken in and not yet written since then.
    dirty_since: Option<u64>,
}

impl Radio {
    pub fn new(store: Store, tower: String) -> Self {
        Radio {
            store,
            live: HashMap::new(),
            tower,
            north_up: false,
            personality: None,
            can_send: false,
            pairing_until: None,
            dirty_since: None,
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Everything retained, again: after a reconnect to the broker.
    pub fn publish_all(&self, now: u64) -> Vec<Effect> {
        let mut out = vec![self.status(now), self.pairing(now, None)];
        out.extend(self.store.nodes.iter().map(|n| self.node_topic(n.addr, now)));
        out
    }

    /// The north bridge stopped answering: what was queued on it is gone with it.
    pub fn north_lost(&mut self, now: u64) -> Vec<Effect> {
        self.north_up = false;
        self.can_send = false;
        for live in self.live.values_mut() {
            if let Some(item) = live.on_north.take() {
                live.waiting.push_front(item);
            }
        }
        vec![self.status(now)]
    }

    /// One message from the north bridge.
    pub fn on_north(&mut self, message: Message, now: u64, utc_offset_min: i16) -> Vec<Effect> {
        let mut out = Vec::new();
        match message {
            Message::Hello(hello) => {
                self.north_up = true;
                self.personality = Some(hello.active);
                if hello.tx_floor > self.store.tx_floor {
                    self.store.tx_floor = hello.tx_floor;
                    out.push(Effect::Save);
                }
                out.push(Effect::North(self.settings()));
                for node in &self.store.nodes {
                    out.push(Effect::North(Message::NodeAdd(NodeAdd {
                        node: node.addr,
                        key: node.key,
                        last_seen: node.last_counter,
                    })));
                }
                // The north bridge started again: its queue is empty.
                let addrs: Vec<u32> = self.store.nodes.iter().map(|n| n.addr).collect();
                for addr in addrs {
                    let live = self.live.entry(addr).or_default();
                    if let Some(item) = live.on_north.take() {
                        live.waiting.push_front(item);
                    }
                    out.extend(self.feed(addr, now, utc_offset_min));
                }
                out.push(self.status(now));
            }
            Message::Status(status) => {
                if status.tx_floor > self.store.tx_floor {
                    self.store.tx_floor = status.tx_floor;
                    out.push(Effect::Save);
                }
                if status.can_send != self.can_send {
                    self.can_send = status.can_send;
                    out.push(self.status(now));
                }
            }
            Message::Answer(answer) if answer.result != Outcome::Done => {
                out.push(Effect::Say(format!(
                    "the north bridge refused a message: {:?}",
                    answer.result
                )));
            }
            Message::Uplink(up) => out.extend(self.uplink(up, now, utc_offset_min)),
            Message::Delivery(d) => {
                let live = self.live.entry(d.node).or_default();
                match d.result {
                    Sent::Delivered => {
                        live.on_north = None;
                        out.extend(self.feed(d.node, now, utc_offset_min));
                        out.push(self.node_topic(d.node, now));
                    }
                    // It stays on the north bridge for the node's next uplink.
                    Sent::NotDelivered | Sent::DutyLimited => {}
                    Sent::Refused => out.push(Effect::Say(format!(
                        "the north bridge cannot send to {}: its counter is locked",
                        address(d.node)
                    ))),
                }
            }
            Message::Paired(paired) => {
                self.store.nodes.retain(|n| n.addr != paired.node);
                self.store.nodes.push(StoredNode {
                    addr: paired.node,
                    key: paired.key,
                    name: String::new(),
                    kind: String::new(),
                    last_counter: 0,
                });
                self.live.remove(&paired.node);
                self.pairing_until = None;
                out.push(Effect::Save);
                out.push(Effect::Say(format!(
                    "paired {}, code {:06}",
                    address(paired.node),
                    paired.sas
                )));
                out.push(self.pairing(now, Some((paired.node, paired.sas))));
                out.push(self.node_topic(paired.node, now));
                out.push(self.status(now));
            }
            Message::PairingClosed => {
                self.pairing_until = None;
                out.push(self.pairing(now, None));
            }
            Message::JoinRefused(refused) => out.push(Effect::Say(format!(
                "{} asked to join and was refused: {:?}",
                address(refused.node),
                refused.why
            ))),
            Message::KeyedJoin(join) => out.push(Effect::Say(format!(
                "{} joined under its key",
                address(join.node)
            ))),
            _ => {}
        }
        out
    }

    /// A node's frame.
    fn uplink(&mut self, up: north::Uplink, now: u64, utc_offset_min: i16) -> Vec<Effect> {
        let mut out = Vec::new();
        let Some(node) = self.store.node_mut(up.node) else {
            return vec![Effect::Say(format!(
                "an uplink from {}, which the agent does not know",
                address(up.node)
            ))];
        };
        if up.counter > node.last_counter {
            node.last_counter = up.counter;
            self.dirty_since.get_or_insert(now);
        }
        let kind = props::kind(&node.kind);
        let live = self.live.entry(up.node).or_default();
        live.rssi = Some(up.rssi);
        live.last_seen = Some(now);
        live.uplinks = live.uplinks.wrapping_add(1);
        let base = format!("radio/nodes/{}", address(up.node));
        match decode_node_msg(up.data.as_slice()) {
            Err(tower_protocol::Error::SchemaMismatch { got }) => out.push(Effect::Say(format!(
                "{} speaks another radio schema (0x{got:04x}, here 0x{RADIO_TAG:04x}): built from another \
                 TOWER revision",
                address(up.node)
            ))),
            Err(e) => out.push(Effect::Say(format!(
                "{} sent what does not decode: {e:?}",
                address(up.node)
            ))),
            Ok(NodeMsg::Info(info)) => {
                live.sleeping = info.sleeping;
                let wants_time = info.wants_time;
                let new_kind = props::kind_of_firmware(info.firmware_name);
                out.extend(self.learn_kind(up.node, new_kind, now));
                if wants_time {
                    self.live.entry(up.node).or_default().push(Item::Time);
                }
            }
            Ok(NodeMsg::Report(values)) => match kind {
                Some(kind) => {
                    for (id, value) in values {
                        if let Some(prop) = kind.prop(id) {
                            let topic = format!("{base}/{}/{}", values::topic_kind(prop), prop.name);
                            out.push(publish(
                                topic,
                                json!({"value": values::json(&prop.ty, value), "ts": now}),
                                prop.access != Access::Event,
                            ));
                        }
                    }
                }
                // Its kind is not known yet: it is asked to describe itself, once.
                None if !live.asked => {
                    live.asked = true;
                    live.push(Item::Describe);
                }
                None => {}
            },
            Ok(NodeMsg::Event { id, value, count }) => {
                if let Some(prop) = kind.and_then(|k| k.prop(id)) {
                    out.push(publish(
                        format!("{base}/event/{}", prop.name),
                        json!({"value": values::json(&prop.ty, value), "count": count,
                               "counter": up.counter, "ts": now}),
                        false,
                    ));
                }
            }
            Ok(NodeMsg::Shell(_)) => {}
        }
        out.extend(self.feed(up.node, now, utc_offset_min));
        out.push(self.node_topic(up.node, now));
        out
    }

    /// A node's kind, from its `Info`: a new kind clears the old one's states; an unnamed node
    /// is named `<kind>:<n>`.
    fn learn_kind(&mut self, addr: u32, kind: Option<&'static Kind>, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        let new = kind.map_or("", |k| k.name);
        let taken: Vec<String> = self.store.nodes.iter().map(|n| n.name.clone()).collect();
        let Some(node) = self.store.node_mut(addr) else {
            return out;
        };
        if node.kind != new {
            if let Some(old) = props::kind(&node.kind) {
                for prop in old.all().filter(|p| p.access != Access::Event) {
                    out.push(clear(format!(
                        "radio/nodes/{}/state/{}",
                        address(addr),
                        prop.name
                    )));
                }
            }
            node.kind = new.to_owned();
            out.push(Effect::Save);
        }
        if node.name.is_empty() && !new.is_empty() {
            node.name = auto_name(new, &taken);
            out.push(Effect::Save);
        }
        let _ = now;
        out
    }

    /// The next waiting item to the north bridge, when it holds nothing for the node.
    fn feed(&mut self, addr: u32, now: u64, utc_offset_min: i16) -> Vec<Effect> {
        if !self.north_up {
            return Vec::new();
        }
        let live = self.live.entry(addr).or_default();
        if live.on_north.is_some() {
            return Vec::new();
        }
        let Some(item) = live.waiting.pop_front() else {
            return Vec::new();
        };
        let cmd = match item {
            Item::Set { id, value } => NodeCmd::Set { id, value },
            Item::Time => NodeCmd::Time {
                unix: u32::try_from(now).unwrap_or(u32::MAX),
                utc_offset_min,
            },
            Item::Describe => NodeCmd::Describe,
        };
        let mut buf = [0u8; MAX_RADIO_PAYLOAD];
        let Some(data) = encode_node_cmd(&cmd, &mut buf)
            .ok()
            .and_then(|n| Bytes::new(&buf[..n]))
        else {
            return vec![Effect::Say(format!("cannot encode {cmd:?}"))];
        };
        live.on_north = Some(item);
        vec![Effect::North(Message::Queue(north::Queue { node: addr, data }))]
    }

    /// A client's `set` on `radio/nodes/{addr}/set/{prop}`.
    pub fn set(
        &mut self,
        addr: &str,
        prop: &str,
        payload: &[u8],
        now: u64,
        utc_offset_min: i16,
    ) -> Result<Vec<Effect>, String> {
        let addr = parse_address(addr).ok_or("not an address")?;
        let node = self.store.node(addr).ok_or("no such node")?;
        let kind =
            props::kind(&node.kind).ok_or("its kind is not known yet (it says so in its next Info)")?;
        let prop = kind
            .prop_named(prop)
            .ok_or_else(|| format!("a {} has no {prop}", kind.name))?;
        let value = values::parse_set(prop, payload)?;
        self.live
            .entry(addr)
            .or_default()
            .push(Item::Set { id: prop.id, value });
        let mut out = self.feed(addr, now, utc_offset_min);
        out.push(self.node_topic(addr, now));
        Ok(out)
    }

    /// A request `radio.*`: its answer's data, and what to do.
    pub fn request(
        &mut self,
        op: &str,
        params: &Value,
        now: u64,
    ) -> Result<(Option<Value>, Vec<Effect>), String> {
        let text = |key: &str| params.get(key).and_then(Value::as_str);
        let node_arg = || {
            text("addr")
                .and_then(parse_address)
                .ok_or_else(|| "invalid argument: addr is \"0x%08x\"".to_owned())
        };
        let mut out = Vec::new();
        match op {
            "radio.config" => {
                if let Some(channel) = params.get("channel").filter(|v| !v.is_null()) {
                    match channel.as_u64() {
                        Some(c @ 0..=2) => self.store.channel = c as u8,
                        _ => return Err("invalid argument: channel is 0 to 2".into()),
                    }
                }
                if let Some(keyed) = params.get("keyed_join").filter(|v| !v.is_null()) {
                    self.store.keyed_join = keyed
                        .as_bool()
                        .ok_or("invalid argument: keyed_join is true or false")?;
                }
                out.push(Effect::Save);
                if self.north_up {
                    out.push(Effect::North(self.settings()));
                }
                out.push(self.status(now));
            }
            "radio.node_add" => {
                let addr = node_arg()?;
                if addr == 0 || addr == u32::MAX || addr == self.store.gateway {
                    return Err("invalid argument: addr is reserved or the gateway's".into());
                }
                let key = text("key")
                    .and_then(parse_key)
                    .ok_or("invalid argument: key is 32 hex digits")?;
                let name = text("name").unwrap_or("").to_owned();
                if name.len() > NAME_MAX {
                    return Err(format!("invalid argument: name is at most {NAME_MAX} bytes"));
                }
                let full = self.store.nodes.len() >= MAX_NODES;
                match self.store.node_mut(addr) {
                    Some(node) => {
                        if node.key != key {
                            node.key = key;
                            node.last_counter = 0;
                        }
                        if !name.is_empty() {
                            node.name = name;
                        }
                    }
                    None if full => return Err("16 nodes already".into()),
                    None => self.store.nodes.push(StoredNode {
                        addr,
                        key,
                        name,
                        kind: String::new(),
                        last_counter: 0,
                    }),
                }
                out.push(Effect::Save);
                if self.north_up
                    && let Some(node) = self.store.node(addr)
                {
                    out.push(Effect::North(Message::NodeAdd(NodeAdd {
                        node: addr,
                        key: node.key,
                        last_seen: node.last_counter,
                    })));
                }
                out.push(self.node_topic(addr, now));
                out.push(self.status(now));
            }
            "radio.node_remove" => {
                let addr = node_arg()?;
                let Some(node) = self.store.node(addr) else {
                    return Err("no such node".into());
                };
                let base = format!("radio/nodes/{}", address(addr));
                if let Some(kind) = props::kind(&node.kind) {
                    for prop in kind.all().filter(|p| p.access != Access::Event) {
                        out.push(clear(format!("{base}/state/{}", prop.name)));
                    }
                }
                out.push(clear(base));
                self.store.nodes.retain(|n| n.addr != addr);
                self.live.remove(&addr);
                out.push(Effect::Save);
                if self.north_up {
                    out.push(Effect::North(Message::NodeRemove(NodeRef { node: addr })));
                }
                out.push(self.status(now));
            }
            "radio.node_rename" => {
                let addr = node_arg()?;
                let name = text("name").ok_or("invalid argument: name")?;
                if name.len() > NAME_MAX {
                    return Err(format!("invalid argument: name is at most {NAME_MAX} bytes"));
                }
                let node = self.store.node_mut(addr).ok_or("no such node")?;
                node.name = name.to_owned();
                out.push(Effect::Save);
                out.push(self.node_topic(addr, now));
            }
            "radio.pairing_open" => {
                let seconds = match params.get("seconds").filter(|v| !v.is_null()) {
                    None => 60,
                    Some(v) => match v.as_u64() {
                        Some(s @ 10..=600) => s,
                        _ => return Err("invalid argument: seconds is 10 to 600".into()),
                    },
                };
                if !self.north_up {
                    return Err("the north bridge does not answer".into());
                }
                if self.pairing_until.is_some() {
                    return Err("a pairing window is open already".into());
                }
                if self.store.nodes.len() >= MAX_NODES {
                    return Err("16 nodes already".into());
                }
                self.pairing_until = Some(now + seconds);
                out.push(Effect::North(Message::PairingOpen(north::PairingOpen {
                    seconds: seconds as u16,
                })));
                out.push(self.pairing(now, None));
            }
            "radio.pairing_cancel" => {
                if self.pairing_until.take().is_some() {
                    out.push(Effect::North(Message::PairingCancel));
                }
                out.push(self.pairing(now, None));
            }
            _ => return Err("unknown op".into()),
        }
        Ok((None, out))
    }

    /// Once a second: the counters written when it is time, a window the north bridge never
    /// closed closed.
    pub fn tick(&mut self, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if self.dirty_since.is_some_and(|since| now >= since + SAVE_EVERY) {
            self.dirty_since = None;
            out.push(Effect::Save);
        }
        if self
            .pairing_until
            .is_some_and(|until| now >= until + PAIRING_GRACE)
        {
            self.pairing_until = None;
            out.push(self.pairing(now, None));
        }
        out
    }

    /// The counters not yet written, to be written now (the agent stops).
    pub fn flush(&mut self) -> Vec<Effect> {
        match self.dirty_since.take() {
            Some(_) => vec![Effect::Save],
            None => Vec::new(),
        }
    }

    /// The pairing window's end, when one is open.
    pub fn pairing_until(&self) -> Option<u64> {
        self.pairing_until
    }

    /// What the front panel's radio page shows.
    pub fn summary(&self) -> Summary {
        let last = self
            .live
            .iter()
            .filter_map(|(addr, live)| Some((*addr, live.last_seen?, live.rssi.unwrap_or(0))))
            .max_by_key(|(_, seen, _)| *seen)
            .map(|(addr, seen, rssi)| {
                let name = self
                    .store
                    .node(addr)
                    .map(|n| n.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| address(addr));
                (name, rssi, seen)
            });
        Summary {
            answering: self.north_up,
            channel: self.store.channel,
            nodes: self.store.nodes.len(),
            last,
            pairing_until: self.pairing_until,
        }
    }

    fn settings(&self) -> Message {
        Message::Settings(north::Settings {
            address: self.store.gateway,
            band: Band::Eu868,
            channel: self.store.channel,
            keyed_join: self.store.keyed_join,
            tx_floor: self.store.tx_floor,
        })
    }

    fn status(&self, now: u64) -> Effect {
        let personality = match (self.north_up, self.personality) {
            (true, Some(Personality::TowerFsk)) => json!("tower-fsk"),
            _ => Value::Null,
        };
        publish(
            "radio/status".into(),
            json!({
                "personality": personality,
                "band": "eu868",
                "channel": self.store.channel,
                "gateway": address(self.store.gateway),
                "tower": self.tower,
                "wire": format!("radio 0x{RADIO_TAG:04x}"),
                "can_send": self.can_send,
                "nodes": self.store.nodes.len(),
                "ts": now,
            }),
            true,
        )
    }

    fn pairing(&self, now: u64, joined: Option<(u32, u32)>) -> Effect {
        let payload = match (self.pairing_until, joined) {
            (Some(until), _) => json!({"state": "open", "remaining": until.saturating_sub(now), "ts": now}),
            (None, Some((node, sas))) => {
                json!({"state": "idle", "joined": address(node), "sas": format!("{sas:06}"), "ts": now})
            }
            (None, None) => json!({"state": "idle", "ts": now}),
        };
        publish("radio/pairing".into(), payload, true)
    }

    fn node_topic(&self, addr: u32, now: u64) -> Effect {
        let topic = format!("radio/nodes/{}", address(addr));
        let Some(node) = self.store.node(addr) else {
            return clear(topic);
        };
        let live = self.live.get(&addr);
        publish(
            topic,
            json!({
                "addr": address(addr),
                "stack": "tower",
                "name": node.name,
                "kind": node.kind,
                "sleeping": live.is_none_or(|l| l.sleeping),
                "rssi": live.and_then(|l| l.rssi),
                "last_seen": live.and_then(|l| l.last_seen),
                "uplinks": live.map_or(0, |l| l.uplinks),
                "queued": live.map_or(0, Live::queued),
                "ts": now,
            }),
            true,
        )
    }
}

/// `<kind>:<n>` with the lowest `n` no other node's name has, the kind clipped so that the name
/// fits [`NAME_MAX`] bytes, as TOWER's bridge names nodes.
fn auto_name(kind: &str, taken: &[String]) -> String {
    (0u32..)
        .map(|n| {
            let suffix = format!(":{n}");
            let room = NAME_MAX.saturating_sub(suffix.len());
            let mut cut = kind.len().min(room);
            while !kind.is_char_boundary(cut) {
                cut -= 1;
            }
            format!("{}{suffix}", &kind[..cut])
        })
        .find(|name| !taken.iter().any(|t| t == name))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fiber_protocol::north::{Delivery, Hello, Paired, Reset, Status, Uplink};
    use tower_protocol::radio::{NodeInfo, decode_node_cmd, encode_node_msg};

    const GW: u32 = 0x2a7f_03c1;
    const NODE: u32 = 0x1111_1111;
    const KEY: [u8; 16] = [0x11; 16];

    fn radio_with_node(kind: &str) -> Radio {
        let mut store = Store::new(GW);
        store.nodes.push(StoredNode {
            addr: NODE,
            key: KEY,
            name: String::new(),
            kind: kind.into(),
            last_counter: 100,
        });
        Radio::new(store, "cb3b25c".into())
    }

    fn hello(tx_floor: u32) -> Message {
        Message::Hello(Hello {
            version: [0, 1, 0],
            image_crc: 1,
            uid: [0; 12],
            reset: Reset::PowerOn,
            fault: None,
            personalities: 1,
            active: Personality::TowerFsk,
            tx_floor,
        })
    }

    fn uplink(counter: u32, msg: &NodeMsg<'_>) -> Message {
        let mut buf = [0u8; 74];
        let n = encode_node_msg(msg, &mut buf).unwrap();
        Message::Uplink(Uplink {
            node: NODE,
            counter,
            rssi: -71,
            confirmed: true,
            data: Bytes::new(&buf[..n]).unwrap(),
        })
    }

    fn info(firmware: &str, wants_time: bool) -> NodeMsg<'_> {
        NodeMsg::Info(NodeInfo {
            firmware_name: firmware,
            firmware_version: "v0.1.0",
            session_id: 7,
            sleeping: true,
            wants_time,
        })
    }

    fn published(effects: &[Effect], topic: &str) -> Option<Value> {
        effects.iter().rev().find_map(|e| match e {
            Effect::Publish {
                topic: t, payload, ..
            } if t == topic => Some(serde_json::from_str(payload).unwrap_or(Value::Null)),
            _ => None,
        })
    }

    fn to_north(effects: &[Effect]) -> Vec<Message> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::North(m) => Some(*m),
                _ => None,
            })
            .collect()
    }

    /// The command in a `Queue` to the north bridge.
    fn queued_cmd(message: &Message) -> String {
        match message {
            Message::Queue(q) => format!("{:?}", decode_node_cmd(q.data.as_slice()).unwrap()),
            other => panic!("not a queue: {other:?}"),
        }
    }

    #[test]
    fn a_hello_gets_the_settings_and_every_node() {
        let mut radio = radio_with_node("climate");
        let out = radio.on_north(hello(6024), 1000, 60);
        let north = to_north(&out);
        assert_eq!(
            north[0],
            Message::Settings(north::Settings {
                address: GW,
                band: Band::Eu868,
                channel: 0,
                keyed_join: true,
                tx_floor: 6024
            })
        );
        assert_eq!(
            north[1],
            Message::NodeAdd(NodeAdd {
                node: NODE,
                key: KEY,
                last_seen: 100
            })
        );
        assert!(out.contains(&Effect::Save), "the higher floor is kept");
        assert_eq!(radio.store().tx_floor, 6024);
        let status = published(&out, "radio/status").unwrap();
        assert_eq!(status["personality"], "tower-fsk");
        assert_eq!(status["gateway"], "0x2a7f03c1");
        assert_eq!(status["nodes"], 1);
        // A lower floor later is not taken.
        radio.on_north(hello(10), 1001, 60);
        assert_eq!(radio.store().tx_floor, 6024);
    }

    #[test]
    fn a_node_reports_and_tells_of_events() {
        let mut radio = radio_with_node("climate");
        radio.on_north(hello(0), 0, 60);
        let mut report = heapless::Vec::new();
        report.push((0u8, 2150)).unwrap(); // temperature/system, 2 decimals
        report.push((99u8, 1)).unwrap(); // no such property: left out
        let out = radio.on_north(uplink(101, &NodeMsg::Report(report)), 1000, 60);
        let base = "radio/nodes/0x11111111";
        let state = published(&out, &format!("{base}/state/temperature/system")).unwrap();
        assert_eq!(state, json!({"value": 21.5, "ts": 1000}));
        let node = published(&out, base).unwrap();
        assert_eq!(
            (node["rssi"].clone(), node["uplinks"].clone()),
            (json!(-71), json!(1))
        );
        assert_eq!(node["stack"], "tower");
        let button = props::kind("climate").unwrap().prop_named("button").unwrap();
        let out = radio.on_north(
            uplink(
                102,
                &NodeMsg::Event {
                    id: button.id,
                    value: 2,
                    count: 42,
                },
            ),
            1001,
            60,
        );
        let event = published(&out, &format!("{base}/event/button")).unwrap();
        assert_eq!(
            event,
            json!({"value": "click", "count": 42, "counter": 102, "ts": 1001})
        );
        assert!(out.iter().any(
            |e| matches!(e, Effect::Publish { topic, retain: false, .. } if topic.ends_with("event/button"))
        ));
    }

    #[test]
    fn its_first_info_names_it_and_its_time_is_sent() {
        let mut radio = radio_with_node("");
        radio.on_north(hello(0), 0, 60);
        // Before its kind is known, a report asks it to describe itself, once.
        let mut report = heapless::Vec::new();
        report.push((0u8, 2150)).unwrap();
        let out = radio.on_north(uplink(101, &NodeMsg::Report(report.clone())), 1000, 60);
        assert_eq!(queued_cmd(&to_north(&out)[0]), "Describe");
        let out = radio.on_north(uplink(102, &NodeMsg::Report(report)), 1001, 60);
        assert!(to_north(&out).is_empty());
        // Its Info: a climate node, wanting the time.
        let out = radio.on_north(uplink(103, &info("radio_climate", true)), 1002, 60);
        assert_eq!(radio.store().node(NODE).unwrap().kind, "climate");
        assert_eq!(radio.store().node(NODE).unwrap().name, "climate:0");
        assert!(out.contains(&Effect::Save));
        // The Describe still waits on the north bridge; once delivered, the time follows.
        assert!(to_north(&out).is_empty());
        let out = radio.on_north(
            Message::Delivery(Delivery {
                node: NODE,
                result: Sent::Delivered,
                ack_rssi: Some(-60),
            }),
            1003,
            60,
        );
        assert_eq!(
            queued_cmd(&to_north(&out)[0]),
            "Time { unix: 1003, utc_offset_min: 60 }"
        );
    }

    #[test]
    fn a_set_is_checked_and_queued() {
        let mut radio = radio_with_node("climate");
        radio.on_north(hello(0), 0, 60);
        let addr = "0x11111111";
        assert!(radio.set(addr, "heartbeat", b"5", 1, 60).is_err(), "out of range");
        assert!(
            radio.set(addr, "temperature/system", b"20", 1, 60).is_err(),
            "a state"
        );
        assert!(
            radio.set(addr, "volume", b"5", 1, 60).is_err(),
            "no such property"
        );
        assert!(
            radio.set("0x99999999", "heartbeat", b"600", 1, 60).is_err(),
            "no such node"
        );
        // The first goes to the north bridge at once; the next two wait, one for each property,
        // the newest value.
        let out = radio.set(addr, "heartbeat", b"600", 1, 60).unwrap();
        let heartbeat = props::kind("climate")
            .unwrap()
            .prop_named("heartbeat")
            .unwrap()
            .id;
        assert_eq!(
            queued_cmd(&to_north(&out)[0]),
            format!("Set {{ id: {heartbeat}, value: 600 }}")
        );
        assert!(to_north(&radio.set(addr, "report-period", b"30", 2, 60).unwrap()).is_empty());
        let out = radio.set(addr, "report-period", b"45", 3, 60).unwrap();
        assert_eq!(published(&out, "radio/nodes/0x11111111").unwrap()["queued"], 2);
        // Not delivered: it stays on the north bridge, nothing new goes.
        let not = |result| {
            Message::Delivery(Delivery {
                node: NODE,
                result,
                ack_rssi: None,
            })
        };
        assert!(to_north(&radio.on_north(not(Sent::NotDelivered), 4, 60)).is_empty());
        let out = radio.on_north(not(Sent::Delivered), 5, 60);
        let period = props::kind("climate")
            .unwrap()
            .prop_named("report-period")
            .unwrap()
            .id;
        assert_eq!(
            queued_cmd(&to_north(&out)[0]),
            format!("Set {{ id: {period}, value: 45 }}")
        );
    }

    #[test]
    fn pairing() {
        let mut radio = radio_with_node("climate");
        assert_eq!(
            radio.request("radio.pairing_open", &json!({}), 0),
            Err("the north bridge does not answer".into())
        );
        radio.on_north(hello(0), 0, 60);
        let (_, out) = radio
            .request("radio.pairing_open", &json!({"seconds": 30}), 100)
            .unwrap();
        assert_eq!(
            to_north(&out),
            vec![Message::PairingOpen(north::PairingOpen { seconds: 30 })]
        );
        assert_eq!(
            published(&out, "radio/pairing").unwrap(),
            json!({"state": "open", "remaining": 30, "ts": 100})
        );
        assert!(
            radio.request("radio.pairing_open", &json!({}), 101).is_err(),
            "open already"
        );
        assert!(
            radio
                .request("radio.pairing_open", &json!({"seconds": 5}), 101)
                .is_err()
        );
        let out = radio.on_north(
            Message::Paired(Paired {
                node: 0x4444_4444,
                key: [0x5a; 16],
                sas: 4321,
            }),
            110,
            60,
        );
        assert_eq!(
            published(&out, "radio/pairing").unwrap(),
            json!({"state": "idle", "joined": "0x44444444", "sas": "004321", "ts": 110})
        );
        assert_eq!(radio.store().node(0x4444_4444).unwrap().key, [0x5a; 16]);
        assert!(out.contains(&Effect::Save));
        // A window the north bridge closes; one it never closes, closed 20 s after its end.
        radio.request("radio.pairing_open", &json!({}), 200).unwrap();
        let out = radio.on_north(Message::PairingClosed, 260, 60);
        assert_eq!(published(&out, "radio/pairing").unwrap()["state"], "idle");
        radio.request("radio.pairing_open", &json!({}), 300).unwrap();
        assert!(radio.tick(379).is_empty());
        assert_eq!(
            published(&radio.tick(380), "radio/pairing").unwrap()["state"],
            "idle"
        );
    }

    #[test]
    fn nodes_added_renamed_removed() {
        let mut radio = radio_with_node("climate");
        radio.on_north(hello(0), 0, 60);
        let add = |addr: &str, key: &str| json!({"addr": addr, "key": key, "name": "lab"});
        let key = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";
        assert!(
            radio
                .request("radio.node_add", &add("0x00000000", key), 1)
                .is_err()
        );
        assert!(
            radio
                .request("radio.node_add", &add("0x2a7f03c1", key), 1)
                .is_err(),
            "the gateway"
        );
        assert!(
            radio
                .request("radio.node_add", &add("0x22222222", "5a5a"), 1)
                .is_err()
        );
        let (_, out) = radio
            .request("radio.node_add", &add("0x22222222", key), 1)
            .unwrap();
        assert_eq!(
            to_north(&out),
            vec![Message::NodeAdd(NodeAdd {
                node: 0x2222_2222,
                key: [0x5a; 16],
                last_seen: 0
            })]
        );
        assert_eq!(published(&out, "radio/nodes/0x22222222").unwrap()["name"], "lab");
        let (_, out) = radio
            .request(
                "radio.node_rename",
                &json!({"addr": "0x22222222", "name": "barn"}),
                2,
            )
            .unwrap();
        assert_eq!(published(&out, "radio/nodes/0x22222222").unwrap()["name"], "barn");
        assert!(
            radio
                .request(
                    "radio.node_rename",
                    &json!({"addr": "0x22222222", "name": "a name far too long"}),
                    2
                )
                .is_err()
        );
        let (_, out) = radio
            .request("radio.node_remove", &json!({"addr": "0x11111111"}), 3)
            .unwrap();
        assert_eq!(to_north(&out), vec![Message::NodeRemove(NodeRef { node: NODE })]);
        // Its topics cleared: the node's, and the states of its kind.
        assert!(out.contains(&clear("radio/nodes/0x11111111".into())));
        assert!(out.contains(&clear("radio/nodes/0x11111111/state/temperature/system".into())));
        assert_eq!(
            radio.request("radio.node_remove", &json!({"addr": "0x11111111"}), 4),
            Err("no such node".into())
        );
        assert_eq!(
            radio.request("radio.volume", &json!({}), 4),
            Err("unknown op".into())
        );
        let (_, out) = radio.request("radio.config", &json!({"channel": 2}), 5).unwrap();
        assert!(matches!(to_north(&out)[0], Message::Settings(s) if s.channel == 2));
        assert!(radio.request("radio.config", &json!({"channel": 3}), 5).is_err());
    }

    #[test]
    fn counters_are_written_lazily() {
        let mut radio = radio_with_node("climate");
        radio.on_north(hello(0), 0, 60);
        let mut report = heapless::Vec::new();
        report.push((0u8, 2150)).unwrap();
        let out = radio.on_north(uplink(500, &NodeMsg::Report(report.clone())), 1000, 60);
        assert!(!out.contains(&Effect::Save));
        assert_eq!(radio.store().node(NODE).unwrap().last_counter, 500);
        radio.on_north(uplink(501, &NodeMsg::Report(report)), 1030, 60);
        assert!(radio.tick(1059).is_empty());
        assert_eq!(radio.tick(1060), vec![Effect::Save]);
        assert!(radio.tick(2000).is_empty());
    }

    #[test]
    fn a_node_of_another_schema_is_said() {
        let mut radio = radio_with_node("climate");
        let mut data = [0u8; 4];
        data[..2].copy_from_slice(&(RADIO_TAG ^ 0x0101).to_le_bytes());
        let out = radio.on_north(
            Message::Uplink(Uplink {
                node: NODE,
                counter: 101,
                rssi: -70,
                confirmed: false,
                data: Bytes::new(&data).unwrap(),
            }),
            1,
            60,
        );
        assert!(
            out.iter()
                .any(|e| matches!(e, Effect::Say(s) if s.contains("another radio schema")))
        );
        let _ = Status::default();
    }

    #[test]
    fn names() {
        assert_eq!(auto_name("climate", &[]), "climate:0");
        assert_eq!(auto_name("climate", &["climate:0".into()]), "climate:1");
        assert_eq!(auto_name("greenhouse_monitor", &[]), "greenhouse_mon:0");
    }
}
