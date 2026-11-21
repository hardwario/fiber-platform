# The north link

The serial link between the agent and the north bridge (STM32WL55, the radio): what goes over the
wire, field by field, and when. As with the south link ([`south-link.md`](south-link.md)), the
agent and the firmware build these messages from one crate, `crates/protocol` (`fiber-protocol`),
and a frame from a peer built from another source is refused by its schema tag.

**Status: a draft (T25).** Written before its code. The messages are in `crates/protocol`
(`north`) and the firmware speaks them (`firmware/north`); the agent's side is T26. Not run on a
FIBER yet; the maintainer has not read it.

## What it is for

The north bridge is the FIBER's TOWER gateway: it runs the gateway's flow
([`radio.md`](radio.md), "The gateway's flow" and "Pairing a node"). It keeps no list of nodes
(D8): after every `Hello` the agent sends it the settings, the transmit counter's floor and every
node with its key. What the nodes send, the north bridge passes on as bytes; the agent reads them
with `tower-protocol` (T26). A radio of another kind later (STICKER LoRa, D4) is another
personality on the same link: one at a time, chosen by the agent.

## The wire

- **USART1** of the north bridge (PA9 TX, PA10 RX) to `uart3` of the CM4 (`/dev/ttyAMA3`),
  **115200 baud, 8N1**, no flow control. The ROM bootloader uses the same pins with even parity
  ([`../hardware.md`](../hardware.md)).
- **The frame is the south link's:** COBS, then one `0x00`; inside, `type` (1), `tag` (2), `seq`
  (2), the postcard payload, and a CRC-32/IEEE (4), at most 128 bytes. A receiver checks the CRC,
  then the tag, then the type, then that the payload is one message with nothing left over; it
  drops what does not check out and counts it.
- **Its own schema tag,** worked out from the north link's message types as the south link's is
  from its own. A north bridge and an agent from different builds refuse each other's frames; the
  agent then writes the firmware it carries.
- **Bytes of any length** (a node's payload, up to 74 bytes) are a length and the bytes, as
  postcard writes a byte string. Every other field has a fixed size.

## The messages

| `type` | Message | Direction | When |
|---|---|---|---|
| `0x01` | `Hello` | north to agent | At start, and when asked |
| `0x02` | `Answer` | north to agent | To every message from the agent but `HelloRequest` |
| `0x03` | `Status` | north to agent | Every 10 s, and when asked |
| `0x04` | `Uplink` | north to agent | A node's new frame, taken in |
| `0x05` | `Delivery` | north to agent | A queued message went out, or did not |
| `0x06` | `Paired` | north to agent | A node paired |
| `0x07` | `PairingClosed` | north to agent | The window ended with nobody paired, or was cancelled |
| `0x08` | `JoinRefused` | north to agent | A node asked to join and was not answered |
| `0x09` | `KeyedJoin` | north to agent | A known node joined under its key |
| `0x0A` | `Node` | north to agent | One for each node, in answer to `ListNodes` |
| `0x10` | `HelloRequest` | agent to north | When the agent starts, or wants to know |
| `0x11` | `SetPersonality` | agent to north | To choose the radio's personality |
| `0x12` | `Settings` | agent to north | After every `Hello`, and when a setting changes |
| `0x13` | `NodeAdd` | agent to north | After every `Hello`, for each node; when a node is added |
| `0x14` | `NodeRemove` | agent to north | When a node is removed |
| `0x15` | `ListNodes` | agent to north | When the agent wants to compare |
| `0x16` | `Queue` | agent to north | A message to wait for a node's next uplink |
| `0x17` | `Unqueue` | agent to north | To drop it |
| `0x18` | `PairingOpen` | agent to north | To open a pairing window |
| `0x19` | `PairingCancel` | agent to north | To close it early |
| `0x1A` | `StatusRequest` | agent to north | When the agent wants a `Status` now |

**Every message from the agent is answered** (but `HelloRequest`, which a `Hello` answers): an
`Answer` with its `seq` and how it went. The agent sends one message at a time, waits up to
200 ms for its answer, and sends it again when none comes. A message that arrives twice (its
answer was lost) does the same thing twice, and each message is made so that this is harmless.

### `Hello`

| Field | Type | Meaning |
|---|---|---|
| `version` | `[u8; 3]` | The firmware's version |
| `image_crc` | `u32` | CRC-32/IEEE of its own image in flash; the agent compares it with the image it carries |
| `uid` | `[u8; 12]` | The chip's unique id |
| `reset` | `Reset` | Why it last started (as the south link's) |
| `fault` | `Option<Fault>` | A fault it caught before the reset (as the south link's) |
| `personalities` | `u8` | The personalities it has, one bit each: bit 0 `TowerFsk` |
| `active` | `Personality` | The one it runs |
| `tx_floor` | `u32` | Every transmit counter it has used is below this (its kept watermark) |

After a `Hello` the radio is silent: it receives nothing and sends nothing until a `Settings`
gives it an address.

### `Answer`

| Field | Type | Meaning |
|---|---|---|
| `seq` | `u16` | The `seq` of the message it answers |
| `result` | `Outcome` | `Done`, or why not: `Unknown` (a node it does not have), `Full` (16 nodes), `Invalid` (a reserved address, a payload too long, a setting out of range), `NotNow` (no `Settings` yet, or a window already open), `Locked` (the watermark cannot be kept: it sends nothing), `NoEntropy` (no random numbers for pairing) |

### `Status`

| Field | Type | Meaning |
|---|---|---|
| `uptime` | `u32` | Seconds since the firmware started |
| `tx_floor` | `u32` | As in `Hello`: the agent keeps it, and gives it back after a reset |
| `can_send` | `bool` | The counter has its floor and its watermark is kept |
| `duty_left_ms` | `u32` | What is left of the duty cycle's budget (36 000 ms when full) |
| `frames` | `Frames` | The frames heard since start, by what became of them (below) |
| `rx_dropped` | `u16` | Link frames dropped since start; it wraps |

`Frames`, each a `u32` that wraps:

| Field | A frame |
|---|---|
| `taken_in` | Taken in and passed on as an `Uplink` |
| `repeated` | A copy of a frame already taken in: the node did not hear the acknowledgement and sent it again (answered again, not passed on) |
| `replayed` | An older counter than the node's last: dropped |
| `not_authentic` | From a known node, but its tag does not check under the node's key |
| `unknown_node` | To this gateway, from a node it does not have |
| `not_for_us` | To another address, not data (a stray acknowledgement), or not a TOWER frame of this layout: another network's, most often |

A keyed join is counted in none of them: it is answered, and the agent hears of it (`KeyedJoin`,
`JoinRefused`).

### `Uplink`

| Field | Type | Meaning |
|---|---|---|
| `node` | `u32` | Who sent it |
| `counter` | `u32` | Its counter: the agent keeps the last one for each node, and gives it back in `NodeAdd` |
| `rssi` | `i16` | Its strength, dBm |
| `confirmed` | `bool` | It asked for an acknowledgement, and got one |
| `data` | bytes, at most 74 | The payload: one of TOWER's application messages |

**The north bridge acknowledges a node only while it can pass the frame on.** When its queue
towards the agent is full, it takes no frame in: the node gets no acknowledgement and sends again.

### `Delivery`

| Field | Type | Meaning |
|---|---|---|
| `node` | `u32` | Whose message |
| `result` | `Sent` | `Delivered` (acknowledged), `NotDelivered` (no acknowledgement after its tries: it stays queued), `DutyLimited` (not sent: it stays queued), `Refused` (not sent at all: the counter is locked) |
| `ack_rssi` | `Option<i8>` | The strength the node heard it at, from its acknowledgement |

### `Paired`, `PairingClosed`, `JoinRefused`, `KeyedJoin`

| Message | Fields |
|---|---|
| `Paired` | `node` (`u32`: the address it chose), `key` (`[u8; 16]`: its new link key), `sas` (`u32`: the six-digit code, 0 to 999 999). The north bridge has added it already, from counter 0; the agent keeps it, and the window is closed |
| `PairingClosed` | none: the window's time is up, or it was cancelled |
| `JoinRefused` | `node` (`u32`), `why`: `NotV3`, `AlreadyRegistered`, `InvalidAddress`, `WindowClosing`, `LowOrderKey`, `KeyedOff`, `KeyedUnknown`, `KeyedNotAuthentic`, `KeyedStale` (with `counter` and `last_seen`), `CannotSend` ([`radio.md`](radio.md)) |
| `KeyedJoin` | `node` (`u32`) |

At most one `JoinRefused` a second: a refused node asks again for its whole window.

### `Node`

| Field | Type | Meaning |
|---|---|---|
| `node` | `u32` | Its address |
| `last_seen` | `u32` | The last counter taken in from it |
| `queued` | `bool` | A message waits for it |

### `SetPersonality`

| Field | Type | Meaning |
|---|---|---|
| `personality` | `Personality` | `TowerFsk`, the only one now |

Choosing another one forgets the settings and the nodes: the agent sends them again, as after a
`Hello`.

### `Settings`

| Field | Type | Meaning |
|---|---|---|
| `address` | `u32` | The gateway's address (not 0, not `0xFFFFFFFF`) |
| `band` | `Band` | `Eu868`, the only one now |
| `channel` | `u8` | 0 to 2: 868.1, 868.3, 868.5 MHz |
| `keyed_join` | `bool` | Answer the keyed joins of known nodes |
| `tx_floor` | `u32` | The agent's floor for the transmit counter: the highest `tx_floor` it was told, 0 when none |

The counter goes on from the higher of the north bridge's watermark and this floor, so that a
FIBER whose north bridge lost its flash, or whose agent came back from a copy of its disk, never
uses a counter twice (D8). The gateway sends nothing before the first `Settings`.

### `NodeAdd`, `NodeRemove`, `ListNodes`

| Message | Fields |
|---|---|
| `NodeAdd` | `node` (`u32`), `key` (`[u8; 16]`), `last_seen` (`u32`: the last counter the agent has from it). Added again with the same key, its counter only moves up; with another key it starts at `last_seen`, and what was queued for it is dropped |
| `NodeRemove` | `node` (`u32`); what was queued for it is dropped |
| `ListNodes` | none: a `Node` for each, then the `Answer` |

### `Queue`, `Unqueue`

| Message | Fields |
|---|---|
| `Queue` | `node` (`u32`), `data` (bytes, at most 74): sent after the node's next uplink, in place of what was queued before; every acknowledgement to the node says that something waits |
| `Unqueue` | `node` (`u32`) |

### `PairingOpen`, `PairingCancel`

| Message | Fields |
|---|---|
| `PairingOpen` | `seconds` (`u16`: 10 to 600; 60 is TOWER's default) |
| `PairingCancel` | none |

While a window is open the north bridge listens for requests to join; the uplinks of known nodes
wait until it closes (they send again, as TOWER's own gateway does).

### `HelloRequest`, `StatusRequest`

No payload: answered with a `Hello`, a `Status`.

## The timing

- **At start** the firmware sends a `Hello` and waits, silent on the radio, for `Settings`.
- **The agent, on a `Hello`:** `Settings`, then a `NodeAdd` for each node, each answered before
  the next. The radio starts with the `Settings`.
- **A `Status` every 10 s.** The agent keeps its `tx_floor`; no `Status` for 30 s means the north
  bridge has stopped, and the agent resets it (T26).
- **Uplinks as they come,** each passed on before the north bridge takes the next frame in.
