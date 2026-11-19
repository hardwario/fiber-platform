//! The nodes, kept by the agent (D8): `/var/lib/fiber/radio/nodes.json`.
//!
//! Written whole to a new file, synced, and renamed over the old one, so that a power cut leaves
//! the old or the new, never half of either. The north bridge keeps none of it: after its every
//! `Hello` the agent sends it the settings and the nodes again.

use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Everything the radio keeps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    /// The gateway's radio address, chosen once at random.
    #[serde(with = "hex32")]
    pub gateway: u32,
    /// 0 to 2.
    pub channel: u8,
    /// Nodes whose key the FIBER holds may join under it at any time.
    pub keyed_join: bool,
    /// The highest transmit-counter floor the north bridge has reported: given back to it after
    /// its reset, so that no counter is used twice (D8).
    pub tx_floor: u32,
    pub nodes: Vec<StoredNode>,
}

/// One node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredNode {
    #[serde(with = "hex32")]
    pub addr: u32,
    #[serde(with = "hex_key")]
    pub key: [u8; 16],
    /// At most 16 bytes; empty until named.
    pub name: String,
    /// `tower-protocol`'s kind (`climate`); empty until its first `Info`.
    pub kind: String,
    /// The last counter taken in from it: given back to the north bridge with the node, so that
    /// an old frame recorded on air is not taken in again after a restart.
    pub last_counter: u32,
}

impl Store {
    /// A new store, for a gateway at `gateway`.
    pub fn new(gateway: u32) -> Self {
        Store {
            gateway,
            channel: 0,
            keyed_join: true,
            tx_floor: 0,
            nodes: Vec::new(),
        }
    }

    /// The store at `path`; `None` when there is none yet.
    pub fn load(path: &Path) -> io::Result<Option<Store>> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Writes the store to `path`: a new file beside it, synced, renamed over it.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let new = path.with_extension("json.new");
        {
            let mut file = std::fs::File::create(&new)?;
            file.write_all(&json)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
        }
        std::fs::rename(&new, path)?;
        if let Some(dir) = path.parent()
            && let Ok(dir) = std::fs::File::open(dir)
        {
            let _ = dir.sync_all();
        }
        Ok(())
    }

    pub fn node(&self, addr: u32) -> Option<&StoredNode> {
        self.nodes.iter().find(|n| n.addr == addr)
    }

    pub fn node_mut(&mut self, addr: u32) -> Option<&mut StoredNode> {
        self.nodes.iter_mut().find(|n| n.addr == addr)
    }
}

/// A gateway address at random, never a reserved one: from `/dev/urandom`.
pub fn random_address(urandom: &Path) -> io::Result<u32> {
    use std::io::Read;
    let mut file = std::fs::File::open(urandom)?;
    loop {
        let mut bytes = [0u8; 4];
        file.read_exact(&mut bytes)?;
        let addr = u32::from_le_bytes(bytes);
        if addr != 0 && addr != u32::MAX {
            return Ok(addr);
        }
    }
}

/// An address as `0x%08x`, as on MQTT.
pub fn address(addr: u32) -> String {
    format!("0x{addr:08x}")
}

/// An address from `0x%08x` (or without the `0x`).
pub fn parse_address(text: &str) -> Option<u32> {
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    if digits.is_empty() || digits.len() > 8 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

/// A key from 32 hex digits.
pub fn parse_key(text: &str) -> Option<[u8; 16]> {
    if text.len() != 32 {
        return None;
    }
    let mut key = [0u8; 16];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(key)
}

mod hex32 {
    use super::*;

    pub fn serialize<S: Serializer>(value: &u32, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&address(*value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
        let text = String::deserialize(d)?;
        parse_address(&text).ok_or_else(|| serde::de::Error::custom("an address, 0x%08x"))
    }
}

mod hex_key {
    use super::*;

    pub fn serialize<S: Serializer>(key: &[u8; 16], s: S) -> Result<S::Ok, S::Error> {
        let text: String = key.iter().map(|b| format!("{b:02x}")).collect();
        s.serialize_str(&text)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 16], D::Error> {
        let text = String::deserialize(d)?;
        parse_key(&text).ok_or_else(|| serde::de::Error::custom("a key, 32 hex digits"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;

    fn store() -> Store {
        let mut store = Store::new(0x2a7f_03c1);
        store.channel = 2;
        store.tx_floor = 6024;
        store.nodes.push(StoredNode {
            addr: 0x1111_1111,
            key: [
                0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
                0x00,
            ],
            name: "climate:0".into(),
            kind: "climate".into(),
            last_counter: 0xB820,
        });
        store
    }

    #[test]
    fn kept_and_read_back() {
        let dir = TestDir::new("radio-store");
        let path = dir.path().join("radio/nodes.json");
        assert_eq!(Store::load(&path).unwrap(), None);
        store().save(&path).unwrap();
        assert_eq!(Store::load(&path).unwrap(), Some(store()));
        // Readable by a person: the addresses and the key in hex.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"gateway\": \"0x2a7f03c1\""), "{text}");
        assert!(
            text.contains("\"key\": \"112233445566778899aabbccddeeff00\""),
            "{text}"
        );
        // Nothing left beside it.
        assert!(!path.with_extension("json.new").exists());
        std::fs::write(&path, "{").unwrap();
        assert!(Store::load(&path).is_err());
    }

    #[test]
    fn addresses_and_keys() {
        assert_eq!(address(0x1a), "0x0000001a");
        assert_eq!(parse_address("0x0000001a"), Some(0x1a));
        assert_eq!(parse_address("1A2B3C4D"), Some(0x1a2b_3c4d));
        assert_eq!(parse_address("0x123456789"), None);
        assert_eq!(parse_address(""), None);
        assert_eq!(
            parse_key("112233445566778899aabbccddeeff00"),
            Some(store().nodes[0].key)
        );
        assert_eq!(parse_key("1122"), None);
        assert_eq!(parse_key("zz2233445566778899aabbccddeeff00"), None);
    }

    #[test]
    fn a_random_address_is_never_reserved() {
        let dir = TestDir::new("radio-random");
        let path = dir.path().join("urandom");
        // Zero first, all ones next, then a usable one.
        std::fs::write(&path, [0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 1, 2, 3, 4]).unwrap();
        assert_eq!(random_address(&path).unwrap(), 0x0403_0201);
    }
}
