# The radio: TOWER on the north bridge

How the north bridge (an STM32WL55, its sub-GHz radio) speaks to TOWER nodes, whose radio is a
SPIRIT1. What a TOWER node sends on air, as the bench showed it, and how the STM32WL is set to
send and receive the same. The layers around it are in [`../architecture.md`](../architecture.md),
"North bridge"; the measurements in [`../bench-checks.md`](../bench-checks.md), check 4.

**Status:** settled on the bench (2026-11-19, T22) with the radio spike (`hil/radio-spike`) and
two TOWER Core Modules at the pinned revision running `radio_beacon`: frames go both ways. The
settings below are the spike's. The packet is made and checked by `fiber-tower-air`'s `packet`,
the commands by `fiber-subghz` (T23), and the spike runs on both: through them it received every
frame of a Core Module (20 in 20 s, none missed) and a Core Module received every frame it sent
(14 of 14), on 2026-11-19. The north bridge's firmware (T25) is written and builds, and the
agent carries it (T26); neither has run on a FIBER yet. With the
FIBER's antenna fitted the levels rise, and depend on the antenna (see "Good to know"); the
settings were checked against ST's own code for the module and match it. Not measured yet: the error rates at several
levels, the frequency offset in Hz.

## What a TOWER node sends

TOWER's radio settings are in its `firmware/src/radio/config.rs`; what they produce on air was read
from 56 frames of `radio_beacon` (kept as test vectors in `crates/tower-air/tests/`):

| | On air |
|---|---|
| Channels (EU 868) | 868.1, 868.3 and 868.5 MHz (channels 0 to 2); a node and its gateway share one |
| Modulation | GFSK, BT 1, 19 216.6 bit/s (what a SPIRIT1 makes of 19 200), 19.8 kHz deviation |
| Preamble | 4 bytes, `0x55` alternating |
| Sync word | `15 47 62 DB`: TOWER's `SYNC4..SYNC1` (`0xDB624715`) in reverse, as the SPIRIT1 sends `SYNC1` first |
| Length | one byte: the payload's length, without itself and without the CRC |
| Payload | TOWER's network frame (its `docs/firmware/radio.md`); the length byte allows up to 255 bytes, TOWER's own limit is lower |
| CRC | CRC-16/CCITT-FALSE: polynomial `0x1021`, from `0xFFFF`, not reflected, nothing XORed at the end; over the length byte and the payload; high byte first |
| Whitening | PN9 (x^9 + x^5 + 1) from all ones, each byte filled from its least significant bit; over the length byte, the payload and the CRC |

The STM32WL's own whitening fills each byte from its most significant bit, so it cannot make or
undo this; its own CRC engine is not used either. The radio therefore runs raw: a fixed length and
the sync word, no whitening, no CRC, and the packet is built and checked in software, by one pair
of pure functions (`fiber-tower-air`'s `packet`).

**Receiving a packet of any length** with a fixed length set: the radio is set for the longest
(255 bytes) and listens for the sync word. Two byte times (834 us) after it, the first byte is in
the radio's buffer, and it says how long the packet is (`packet::packet_len`); one byte time after
the packet's last byte the radio is put in standby, the packet read from the buffer and checked
(`packet::decode`), and it listens again. So it is ready for the next packet as soon as this one
ends, and not 106 ms later. Seen working on the bench (`hil/radio-spike`, receive mode).

## The gateway's flow

What TOWER's gateway does inside its firmware (`firmware/src/radio/net/mod.rs` and
`firmware/apps/radio_gateway.rs` at the pinned revision), written again over the north bridge's
radio in `crates/tower-air` (`gateway`, T24). Every accept or refuse is a `tower-net` kernel; the
radio, the clock and the storage are ours. Tested on the computer against a made-up radio; not
run on a FIBER yet.

- **The frame:** `fmt_type | flags | src | dest | counter | payload | tag`, the header
  authenticated, the payload encrypted under the node's key (AES-128-CCM; the nonce from `src`
  and `counter`), at most 74 bytes of payload (`fiber_tower_air::frame`).
- **Known nodes only:** a frame from a node the gateway has no key for is dropped on its clear
  header, before any decryption and without an acknowledgement. The agent gives the north bridge
  each node: its address, its key and the last counter seen from it (D8: the firmware keeps no
  registry, and the agent sees the counter of every uplink).
- **The replay rule** (`tower_net::replay`): a counter above the last one seen is new: it is
  delivered, and acknowledged when the node asked for it. The same counter again is a
  retransmission (the acknowledgement was lost): acknowledged again, at most 9 times, and not
  delivered twice. A lower counter is a replay: dropped.
- **The acknowledgement:** 20 ms after the frame (the node turns from sending to receiving), an
  `Ack` frame with the gateway's own next counter. Its 6 bytes: the counter it answers (4, little
  endian), the signal strength the gateway heard (dBm, clamped to -128 to 127), and a flag when a
  downlink waits for the node.
- **A waiting downlink:** one for each node. The flag rides on every acknowledgement to that node;
  20 ms after the node's next uplink the gateway sends it, confirmed, with 2 tries, while the node
  listens. The agent learns whether it arrived.
- **Sending to a node that listens:** a confirmed frame waits 200 ms for its acknowledgement and
  goes again, the same bytes, after 0 to 100 ms at random, up to the tries asked (1 to 10). Frames
  heard in the window that are not the acknowledgement are passed over (`tower_net::ack`).
- **The transmit counter** (`tower_net::txctr`): every frame the gateway sends, acknowledgements
  too, takes the next counter, before anything is sent. A watermark 1024 counters ahead is kept in
  the north bridge's own flash (T25); after a reset the counter starts there, or at the agent's
  number when that is higher. When the watermark cannot be written, the gateway sends nothing: a
  counter used twice under one key breaks the encryption.
- **The duty cycle** (`tower_radio::DutyGovernor`): in EU 868 at most 1 % of the time on air, a
  bucket of 36 s refilled at 1 %; every frame counts, acknowledgements too. A frame that does not
  fit is not sent, and a node whose acknowledgement did not fit sends again.
- **Payloads pass through:** what the nodes send (`Info`, `Report`, `Event`) and what they are
  sent (`Set`, `Time`, `Describe`) are bytes to the gateway; the agent reads them.
- **Not yet:** frequency hopping (US 915), bulk transfer.

## Pairing a node

How a node gets its key, as TOWER's gateway does it (`firmware/src/radio/net/pairing.rs` at the
pinned revision), written again in `crates/tower-air` (`gateway::pairing`, T47). The handshake,
the key schedule, the tags and the code are `tower_net::handshake`'s; who may be answered is
`tower_net::pairing`'s. Tested on the computer against a node made up from the same kernels; not
run on a FIBER yet.

**Over the air** (the agent opens a window, 60 s by default): an X25519 key agreement with a
commitment round, the numeric comparison of Bluetooth LE Secure Connections.

| Frame | From | Payload | Sealed under |
|---|---|---|---|
| `JoinReq` | node, to address 0 | `3`, the node's public key | the public framing key |
| `JoinResp` | gateway | `3`, its public key, its commitment to its nonce | the public framing key |
| `JoinNonce` | node | `3`, the node's nonce | the public framing key |
| `JoinReveal` | gateway | `3`, its nonce, its confirmation tag | the public framing key |
| `JoinConfirm` | node | `3`, the node's confirmation tag | the new link key |

- **The framing key is public** (`TOWER-JOIN-v2`): it only gives the four frames a CCM format, so
  a damaged frame is dropped. Secrecy comes from the key agreement; no key goes on air.
- **The six-digit code:** both ends compute it from everything sent; the agent shows it, the
  person compares it with the node's. A man in the middle shows a different one, except with a
  chance of one in a million.
- **The gateway commits to its nonce before it sees the node's,** so neither side can steer the
  code. Each answered request uses a fresh key pair, from the STM32WL's random number generator;
  without it pairing is refused, never keyed from anything else.
- **Only new nodes:** a request from a node the gateway knows, a reserved address or its own is
  not answered (anyone could ask as that node). To pair a node again, the agent removes it first.
  A request is answered only while a whole handshake (about 3.9 s) fits before the window ends.
- **The waits:** 300 ms for the node's nonce; 300 ms and 1.5 s for its confirmation (the node
  computes its shared secret first, about 1.2 s on a Core Module). The gateway answers each frame
  20 ms after it, like an acknowledgement.
- **The node chooses its address,** and keeps it. The gateway pairs the first node whose
  confirmation is right, and the agent adds it with its new key and counter 0.

**A node whose key the gateway holds already** (added by the agent: provisioned by cable, or
moved from another gateway) joins under that key, at any time, without a window: a `JoinReq`
with the `KEYED` flag to address 0, its own address as payload, sealed under its key; the gateway
answers with a `KEYED` `JoinResp` carrying the request's counter back. The request's counter must
be new on the node's replay lane, so a recorded request draws no second answer. The agent can
switch this off.

## How the STM32WL is set

From RM0453 (the STM32WL reference manual, section 5), as the spike sets it:

| What | Command | Value |
|---|---|---|
| The TCXO (inside the module) | `SetTcxoMode` | 1.7 V, 1 ms to settle; then `Calibrate` (all) and `CalibrateImage` for 863 to 870 MHz |
| Regulator | `SetRegulatorMode` | the switching regulator, as ST's code for the module (`HAL_RADIO_IsDCDC`) |
| Packet type | `SetPacketType` | GFSK |
| Frequency | `SetRfFrequency` | f x 2^25 / 32 MHz |
| Modulation | `SetModulationParams` | bit rate 53 287 (32 x 32 MHz / 19 216.6, rounded; the spike's 53 286 was a slip of 0.002 %), BT 1 (`0x0B`), receive filter 117.3, 156.2 or 187.2 kHz (`0x0B`, `0x1A`, `0x12`), deviation 20 762 (19.8 kHz x 2^25 / 32 MHz, rounded) |
| Packet | `SetPacketParams` | a preamble of 32 bits, a detector of 16 bits, a sync word of 32 bits, no address, fixed length, no CRC, no whitening |
| Sync word | registers `0x06C0` | `15 47 62 DB` |
| Receive gain | register `0x08AC` | `0x96` (boosted) |
| Transmit | `SetPaConfig`, `SetTxParams` | the low-power PA: `04 00 01 01` and 14 for +14 dBm, `01 00 01 01` and 13 for +10 dBm; a ramp of 200 us |
| RF switch | PC3, PC4, PC5 | receive: all high; transmit on the low-power PA: PC3 and PC4 high, PC5 low; off: all low (ST's HAL for the module, `HAL_RADIO_SetSwitchConfig`; on the bench only PC4 high with PC5 low sends, whatever PC3) |

Good to know:

- **The radio sets `XOSC_START_ERR` at its start,** before it is told about the TCXO; clear the
  device errors after the calibrations, and any error then is a real one.
- **The high-power PA is not wired** on this module's board: +14 dBm from the low-power PA is the
  most, and the limit of the 868.0 to 868.6 MHz band (25 mW, 1 % of the time).
- **The switching regulator** made no measurable difference to the noise floor (the boosted gain
  read 1 to 2 dB lower), nor to the levels sent and received: within 1 dB of the linear one,
  both ways, run back to back (2026-11-19).
- **The board wires the module for the low-power PA,** as the module's datasheet (DS14084,
  "Low output power configuration") shows it: `VDDPA` on `VDDRF1V55`, `VR_PA` on `BIAS_LP`,
  `BIAS_HP` to a test point. The module's `ANT` goes through `R63` (0 ohm) to the antenna jack
  `JP16`; the two shunt places beside it (`C56`, `C57`) are not fitted. The module's pin 27,
  `SW_CTL`, is not connected: the datasheet gives it no function beyond its name, and answers on
  ST's forum disagree on it (one, from a community member, says it is PC4, the switch's control
  line brought out). ST's board schematic for the module (MB1779) was not read yet.
- **The antenna is in the path** (the bench unit, 2026-11-19): with its tip 15 cm from a Core
  Module, +14 dBm arrived at -81 dBm and the Core Module's frames at -73 dBm with one 868 MHz
  antenna, and 8 to 9 dB more both ways with another (-71 to -74 dBm, -64 to -68 dBm).
- **Against a second Core Module the receiver is fine, and sending is 10 dB short:** the same
  frames reach the north bridge at 20 cm 17 to 19 dB below a Core Module at 10 cm; between the
  north bridge and one Core Module, the north bridge's +14 dBm loses 95 dB, the Core Module's
  +12 dBm or so 85 dB. Whether the 10 dB are the PA's or the SPIRIT1's RSSI scale needs a
  calibrated receiver; see [`../bench-checks.md`](../bench-checks.md), check 4.
- **No frequency correction:** this radio does not follow the sender's offset. The receive filter
  has to pass both crystals' error: TOWER's own receiver uses about 216 kHz. Between the north
  bridge (a TCXO) and one Core Module the SPIRIT1's `AFC_CORR` read -4 to 0, a small offset.
- **No link quality numbers:** this radio gives the signal strength of a packet (and of the
  channel) and nothing like the SPIRIT1's PQI and SQI, so the north bridge reports the strength
  alone.
