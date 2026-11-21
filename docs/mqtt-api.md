# The agent's MQTT API

The agent shows the board on MQTT, and takes requests there. This page is the reference for
anyone who talks to a FIBER through its broker: a Node-RED flow, a program of your own, and the
`fiber` command line itself, which is a client of exactly these topics (D19).

**Status: a draft for the maintainer to read (T13).** Built in the agent (T17) and tested against
a simulated south bridge through Mosquitto; on a FIBER (2026-11-19) `status`, `power`, `ports`,
`firmware/south`, `display` and the requests `ports.power`, `leds.set`, `leds.auto`,
`display.message` and `firmware.flash` work, also from another computer on port 1884. This first part covers the
agent's core and the south bridge (M2); the thermometers (M3), the front panel (M4) and the radio
(M5) add their topics with their tasks.

## The rules

They are those of TOWER's gateway API, so that one client can talk to both.

- **The prefix** is `fiber/`. The topics below are written without it: `status` is
  `fiber/status`.
- **The broker** is the FIBER's Mosquitto (the `mosquitto` component): port 1883 on the FIBER
  itself without a login, and with `mqtt.lan` port 1884 for the network with the login typed into
  FIBER Setup ([`provisioning.md`](provisioning.md)). The agent connects to 1883 as
  `fiber-agent`.
- **QoS 1 everywhere.** A message can arrive twice: de-duplicate events on `ts` and answers on
  their `id`.
- **State is retained, events are not.** A new subscriber gets the latest state of everything at
  once. After every reconnect to the broker the agent publishes all of its retained topics again.
- **The last will** is on `status`: retained, `{"state":"offline","schema":1}`. The broker sends it
  when the agent's connection drops, so a subscriber sees a dead agent without timing anything.
- **The schema number** is in `status`: the version of this whole tree of topics and payloads,
  now **1**. It changes with any change a subscriber would notice (a topic or a key renamed,
  removed or of another type); a new key or a new topic does not change it, so ignore keys you do
  not know. A client written for another number stops and says so; `fiber` exits with 125.
- **Payloads** are UTF-8 JSON. `ts` is Unix time in seconds, from the FIBER's clock. Voltages are
  volts, with two decimals.

## Topics

"agent" publishes, "client" means you publish and the agent listens.

| Topic | By | Retained | Payload |
|---|---|---|---|
| `status` | agent (+ last will) | yes | [status](#status) |
| `power` | agent | yes | [power](#power) |
| `power/event` | agent | no | [power/event](#powerevent) |
| `ports/{1..8}` | agent | yes | [ports/n](#portsn) |
| `thermometers/{id}` | agent | yes (empty = gone) | [thermometers/id](#thermometersid) |
| `firmware/south`, `firmware/north` (T26) | agent | yes | [firmware/chip](#firmwarechip) |
| `buttons/event` | agent | no | [buttons/event](#buttonsevent) |
| `display` | agent | yes | [display](#display) |
| `radio/status` | agent | yes | [radio/status](#radiostatus) |
| `radio/pairing` | agent | yes | [radio/pairing](#radiopairing) |
| `radio/nodes/{addr}` | agent | yes (empty = removed) | [radio/nodes/addr](#radionodesaddr) |
| `radio/nodes/{addr}/state/{prop}` | agent | yes (empty = gone) | [state/prop](#radionodesaddrstateprop) |
| `radio/nodes/{addr}/event/{prop}` | agent | no | [event/prop](#radionodesaddreventprop) |
| `radio/nodes/{addr}/set/{prop}` | client | **never** | [set/prop](#radionodesaddrsetprop) |
| `cmd` | client | **never** | [a request](#requests) |
| `rsp/{id}` | agent | no | [an answer](#requests) |

### `status`

```json
{"state":"online","schema":1,"version":"0.2.0","hardware":"FIBER-R2 2.1",
 "serial":"10000000abcdef12","name":"fiber-abcdef12","ts":1794952515}
```

| Key | Meaning |
|---|---|
| `state` | `online`; the last will says `offline` |
| `schema` | The schema number of this API |
| `version` | The `fiber` program's version |
| `hardware` | The board and its revision |
| `serial` | The CM4's serial number |
| `name` | The FIBER's host name |

### `power`

```json
{"source":"poe","input":12.08,"battery":4.11,"ts":1794952515}
```

| Key | Meaning |
|---|---|
| `source` | `poe` (the 12 V rail is there), `battery`, or `unknown` (the south bridge does not answer, or the board has no dividers) |
| `input` | The 12 V rail in volts, or `null` |
| `battery` | The battery in volts, or `null`: when the voltages cannot be read, and when it reads below 2 V, as with no cell fitted (a Li-Ion cell that works never reads under 2.5 V) |

It changes when the source changes, or a voltage by more than 0.05 V; otherwise once a minute.
Each voltage is the average of the last 8 readings, one a second: a single reading is too noisy
for 0.05 V. The source changes with a margin, so that it does not flap.

### `power/event`

```json
{"event":"battery","ts":1794952515}
```

`battery` when the FIBER goes from PoE to its battery, `poe` when it goes back.

### `ports/n`

One for each port, `ports/1` to `ports/8`:

```json
{"power":true,"led":"auto","thermometers":["28-0000034a5b1c"],"ts":1794952515}
```

| Key | Meaning |
|---|---|
| `power` | Whether the port's 3.3 V is on |
| `led` | `auto` (the agent shows the port's state) or `set` (a client set it with `leds.set`) |
| `thermometers` | The ids of the probes on the port, as Linux lists them; `null` when the 1-Wire buses were not found (`fiber doctor` says why) |

**The port's LED, when `auto`:**

- **Green:** the port's power is on, and every probe on it gives readings (also when it has none).
- **Red:** a probe on the port gives no readings. It stays red until the probe reads again, or
  until Linux drops it: about 100 s after it was unplugged.
- **Off:** the port's power is off.

Each time the south bridge starts (with the FIBER, or after its firmware is written), the port
LEDs first show a test for about 6 s: red, then green, then amber running from port 1 to port 8,
then dark from port 8 back to port 1. A client's `leds.set` and the agent's colours show after
it.

### `thermometers/id`

One for each DS18B20 on the ports. The id is the name Linux gives the probe: its family `28`,
then its serial number in hex.

```json
{"port":3,"temperature":21.44,"ts":1794952515}
```

| Key | Meaning |
|---|---|
| `port` | The port it is on, 1 to 8 |
| `temperature` | Degrees Celsius with two decimals, or `null` when it gave no reading in the last three rounds |

The agent reads all probes every 10 s, the eight ports side by side. It sends a probe's topic
when its temperature changed by 0.1 °C or more since the last one sent, and at least once a
minute. **An empty retained message** means the probe is gone: Linux no longer lists it (about
100 s after it was unplugged), or its port's power was switched off.

**A probe moved to another port** shows there only after Linux has let it go on its old port:
about 100 s after it was pulled. Until then its old port is red and its `temperature` is `null`;
then its topic names the new port.

What is not a reading: an empty read (Linux reads every error as empty), and 85.00 °C, the value
a DS18B20 holds after power-on, unless the reading before it was above 75 °C.

### `firmware/chip`

`firmware/south` and `firmware/north` (from T26; the north bridge's not seen on a FIBER yet):

```json
{"state":"running","version":"0.2.0","crc":"0x6f1c2a90","carried":"0x6f1c2a90",
 "reset":"power_on","fault":null,"ts":1794952515}
```

| Key | Meaning |
|---|---|
| `state` | `running` (it answers, with the image the agent carries), `flashing`, `silent` (it does not answer), `failed` (flashing it failed three times in an hour; the agent stops trying) |
| `version` | What the chip's `Hello` says, or `null` |
| `crc` | The CRC-32 of the image on the chip, or `null` |
| `carried` | The CRC-32 of the image the agent carries for it |
| `reset` | Why the chip last started: `power_on`, `pin`, `watchdog`, `software`, `brown_out`, `other` |
| `fault` | The fault it reported in its last `Hello`, as `{"code":1,"address":"0x08001f3c"}`, or `null` |

### `buttons/event`

The three buttons beside the display, `up`, `enter` and `down`:

```json
{"buttons":["enter"],"gesture":"click","ts":1794952515}
```

| Key | Meaning |
|---|---|
| `buttons` | The buttons of the gesture: one, or several pressed together |
| `gesture` | `click` (pressed and let go within 0.5 s), `hold` (held for 0.8 s, once), `repeat` (every 0.2 s while it is still held after a `hold`) |

The display's own pages (M4) also follow the buttons; a client sees the same events.

### `display`

```json
{"backlight":80,"message":"Back in 5 minutes","ts":1794952515}
```

| Key | Meaning |
|---|---|
| `backlight` | The backlight's brightness when it is lit, 0 to 100 % |
| `message` | The message a client put on the display (`display.message`), or `null`: cleared, its time up, or sent away with a press of a button |

The backlight dims to 20 % (or stays at its brightness, when that is lower) 60 s after the last
press of a button or the last message (`display.message`), and comes back with the next.

## The radio

**Status: a draft (T26).** Built in the agent and tested against a simulated north bridge
(`agent/src/agent/north.rs`); not run with a real one yet. The north bridge's side is
[`firmware/north-link.md`](firmware/north-link.md).

The FIBER is a gateway for HARDWARIO TOWER nodes (M5). Their topics are those of TOWER's own
gateway (its `docs/cli/mqtt-api.md` at the pinned revision) under `fiber/radio/`, where TOWER's
own bridge uses `tower/`: a client written for TOWER's topics changes its prefix and finds the
nodes and their properties where it knew them. The nodes, their keys and names are the agent's
(D8), in `/var/lib/fiber/radio/nodes.json`, written whole to a new file and renamed over the old,
so that a power cut leaves one or the other.

### `radio/status`

```json
{"personality":"tower-fsk","band":"eu868","channel":0,"gateway":"0x2a7f03c1",
 "tower":"cb3b25c","wire":"radio 0x5f3e","can_send":true,"nodes":3,
 "north":{"uptime":584,"duty_left_ms":35983,"frames":{"taken_in":21,"repeated":1,"replayed":0,
 "not_authentic":0,"unknown_node":0,"not_for_us":42}},"ts":1794952515}
```

| Key | Meaning |
|---|---|
| `personality` | What the radio speaks: `tower-fsk` (D4); `null` while the north bridge does not answer |
| `band`, `channel` | `eu868`; 0 to 2 (868.1, 868.3, 868.5 MHz). The nodes must use the same |
| `gateway` | The gateway's radio address, chosen once at random and kept |
| `tower` | The pinned TOWER revision the radio is built from: the nodes must be built from it too |
| `wire` | `tower-protocol`'s radio schema tag: a node built from other tables is refused on it |
| `can_send` | The transmit counter has its floor (nothing is sent before; the north bridge's `Status`) |
| `nodes` | How many nodes the FIBER knows (at most 16) |
| `north` | What the north bridge counts since it started (its `Status`, [`north-link.md`](firmware/north-link.md)), or `null` before its first: `uptime` in seconds, `duty_left_ms` (36 000 when the duty cycle's budget is full), and `frames`, the frames it heard by what became of them: `taken_in`, `repeated` (a copy: the node did not hear the acknowledgement), `replayed`, `not_authentic`, `unknown_node`, `not_for_us` (another network's, most often) |

Published again when the north bridge's counts of frames change, at most every 10 s (its
`Status`), and when anything else here changes.

### `radio/pairing`

```json
{"state":"idle","joined":"0x1a2b3c4d","sas":"004321","ts":1794952515}
```

| Key | Meaning |
|---|---|
| `state` | `open` (a window is open) or `idle` |
| `remaining` | Seconds until the window closes; only while `open` |
| `joined`, `sas` | The node that joined and the six-digit code to compare with the node's (zero-padded); only on the `idle` that ends a window with a join |

The code is shown on the display too. A node joins with an address it chose; the agent names it
when its first `Info` comes ([below](#radionodesaddr)).

### `radio/nodes/addr`

`{addr}` is the node's address as `0x%08x`.

```json
{"addr":"0x1a2b3c4d","stack":"tower","name":"climate:0","kind":"climate","sleeping":true,
 "rssi":-71,"last_seen":1794952480,"uplinks":42,"queued":0,"ts":1794952515}
```

| Key | Meaning |
|---|---|
| `stack` | Which radio it belongs to: `tower` (a LoRa node later says `lora`, under the same tree) |
| `name` | At most 16 bytes; `""` until named. An unnamed node is named `<kind>:<n>` (`climate:0`) when its kind is first known, as TOWER's bridge does |
| `kind` | From the node's firmware name (`radio_climate` is `climate`), by `tower-protocol`'s tables; `""` until its first `Info` |
| `sleeping` | It sleeps between uplinks: what is sent to it waits for its next uplink |
| `rssi` | The strength of its last uplink, dBm, or `null` |
| `last_seen` | Unix time of its last uplink, or `null` |
| `uplinks` | Uplinks since the agent started |
| `queued` | Values waiting to go to it ([set](#radionodesaddrsetprop)) |

### Node properties

A node runs one of TOWER's apps, and its kind names a table of properties in `tower-protocol`
(`props.rs`): what it reports and what it takes, each with a name, a type and an access. The
agent turns them into topics as TOWER's bridge does, and checks every value against them:

| Access | Published on | Settable with `set` |
|---|---|---|
| state | `state/{prop}`, retained | no |
| control | `state/{prop}`, retained | yes |
| event | `event/{prop}`, not retained | no |
| setting | `state/{prop}`, retained | yes |
| local setting | `state/{prop}`, retained | no (the node's USB console only) |

A name is one level (`report-period`), or two for a state in a group, the readings of one
quantity from several sensors (`temperature/system`; `state/temperature/+` takes them all).
Values are JSON as the type says: `true`, a number with its decimals exact (`21.5`), or a name
(`"auto"`). The kinds and their properties are TOWER's, listed in its `docs/cli/mqtt-api.md`.

### `radio/nodes/addr/state/prop`

```json
{"value":21.5,"ts":1794952515}
```

Retained; an empty message clears it (the node removed, or flashed with another app).

### `radio/nodes/addr/event/prop`

```json
{"value":"click","count":42,"counter":1234,"ts":1794952515}
```

| Key | Meaning |
|---|---|
| `value` | The event's detail |
| `count` | The node's own count of this event since it started: a drop means it restarted, a gap events lost on the air |
| `counter` | The radio frame's counter: it grows for each node, the key to drop a message QoS 1 brought twice |

### `radio/nodes/addr/set/prop`

Published by a client, never retained. The payload is the value as JSON (`true`, `21.5`,
`"auto"`), as `{"value": ...}`, or as plain text (`on`, `21.5`, `auto`, a whole number also in
hex, `0xff8000`). The agent checks it against the node's kind (the property exists, may be set,
the value is in range) and queues it; a refused one goes no further (`fiber radio watch` says
why). The north bridge holds one message for each node, which a sleeping node takes after its
next uplink; the agent keeps the rest, one for each property, the newest value. There is no
answer: the node's report on `state/{prop}` confirms it when it takes it. Not yet: a node that
listens (the `control` kind) takes its value after its next report too; the north bridge cannot
send at once yet.

## Requests

**A request** is published on `cmd`, QoS 1, never retained:

```json
{"id":"3f2b8c1e","op":"ports.power","params":{"port":3,"power":false}}
```

| Key | Meaning |
|---|---|
| `id` | 1 to 64 characters of `A-Z a-z 0-9 _ . -`, new for every request (a UUID is good) |
| `op` | What to do (below) |
| `params` | Its parameters; left out or `null` when it takes none |

**The answer** comes on `rsp/{id}`, QoS 1, not retained:

```json
{"id":"3f2b8c1e","ok":true,"data":{"ports":[true,true,false,true,true,true,true,true]}}
```
```json
{"id":"3f2b8c1e","ok":false,"error":"the south bridge does not answer"}
```

- **Subscribe to `rsp/+` once,** before the first request, and match answers by `id`.
- **Never retain a request.** A retained one is cleared and not run.
- **Every request gets one answer,** within 10 s, except `firmware.flash`, which answers when it is
  done (up to 2 minutes). While a chip is being flashed, the other requests wait for it. A request with a bad `id`, a repeated `id` (the agent remembers the last
  64) or a payload that is not JSON is dropped without an answer.

Errors that any op can answer:

| `error` | Why |
|---|---|
| `unknown op` | There is no such op |
| `invalid argument: ...` | A parameter is missing or wrong; the rest says which |
| `the south bridge does not answer` | The op needs it, and it is `silent` or being flashed |

### `ports.power`

Switches the power of a port, or of all of them.

| Param | Meaning |
|---|---|
| `port` | 1 to 8, or `"all"` |
| `power` | `true`, `false`, or `"cycle"`: off, then on again after `seconds` |
| `seconds` | For `"cycle"`: 1 to 60; 5 when left out |

`data`: `{"ports":[...]}`, the power of all eight ports after it, from port 1. A cycle answers
when the port is on again.

**Nothing of it is kept:** when the agent starts, every port is on and every LED `auto`, whatever
a client had set before (seen on a FIBER, 2026-11-19). While the agent is stopped, the south bridge
keeps the ports as they were and shows its fallback on the LEDs. A client that wants a port off
asks again after `status` comes back `online`.

### `leds.set`

Sets one LED, which then stays so until `leds.auto`.

| Param | Meaning |
|---|---|
| `led` | `"power"`, or `"port1"` to `"port8"` |
| `colour` | `"off"`, `"green"`, `"red"`, `"amber"` (a port LED's green and red together), `"yellow"` (the power LED only) |
| `blink` | Left out: steady. Or `{"on":500,"off":500}` in milliseconds, each a multiple of 20 from 20 to 5100 |

`data`: none.

### `leds.auto`

Gives an LED back to the agent, which shows the port's state on it (or the FIBER's, on the power
LED).

| Param | Meaning |
|---|---|
| `led` | As for `leds.set`, or `"all"` |

`data`: none.

### `firmware.flash`

Writes the image the agent carries to a chip, also when the chip runs it already.

| Param | Meaning |
|---|---|
| `chip` | `"south"` or `"north"` |

`data`: `{"crc":"0x6f1c2a90"}`, the CRC-32 of the image now on the chip. While it runs,
`firmware/{chip}` says `flashing`. Errors of its own: `the bootloader does not answer`, `the chip
has readout protection`, `the image does not check out after writing`.

### `buzzer.beep`

Sounds the buzzer (about 2.4 kHz; it has one tone).

| Param | Meaning |
|---|---|
| `pattern` | `"beep"` (0.1 s), `"double"` (two of them), `"long"` (0.5 s), `"alarm"` (five beeps); `"beep"` when left out |

`data`: none. It answers at once; a pattern that sounds while another is asked for is cut short,
and a press of a button stops it.

### `display.message`

Puts a message on the display, over the pages, and lights the backlight, until it is cleared, its
time is up, or somebody at the FIBER presses a button (`display` then says `"message": null`). It
shows 5 lines of 21 characters (Spleen 6 x 12); what does not fit is cut.

| Param | Meaning |
|---|---|
| `text` | The message, with `\n` between lines; `null` clears it |
| `seconds` | How long it stays: 1 to 86400; until it is cleared when left out |

`data`: none.

### `display.backlight`

| Param | Meaning |
|---|---|
| `level` | The brightness when lit, 0 to 100 % |

`data`: none. It is kept until the agent restarts.

### `radio.config`

| Param | Meaning |
|---|---|
| `channel` | 0 to 2; unchanged when left out |
| `keyed_join` | Whether a node whose key the FIBER holds may join under it at any time; unchanged when left out (`true` at first) |

`data`: none. Kept with the nodes. Every node must be on the same channel.

### `radio.node_add`

Adds a node whose key is known already (provisioned by cable, or moved from another gateway).

| Param | Meaning |
|---|---|
| `addr` | Its address, `"0x%08x"` (not 0, not `0xffffffff`, not the gateway's) |
| `key` | Its AES-128 key, 32 hex digits |
| `name` | Optional, at most 16 bytes |

`data`: none. Refused while a pairing window is open: a node that paired meanwhile could find the
north bridge's 16 places taken. Close the window first (`radio.pairing_cancel`).

### `radio.node_remove`, `radio.node_rename`

| Param | Meaning |
|---|---|
| `addr` | The node |
| `name` | For `radio.node_rename`: at most 16 bytes |

`data`: none. A removed node's topics are cleared. To pair a node again, remove it first.

### `radio.pairing_open`, `radio.pairing_cancel`

| Param | Meaning |
|---|---|
| `seconds` | For `radio.pairing_open`: 10 to 600; 60 when left out |

`data`: none. The outcome comes on `radio/pairing`.

The radio's requests can also answer `the north bridge does not answer`, `no such node`,
`16 nodes already` and `a pairing window is open already`.

## Example

```sh
mosquitto_sub -h 127.0.0.1 -t 'fiber/#' -v
mosquitto_pub -h 127.0.0.1 -t fiber/cmd -q 1 \
  -m '{"id":"a1","op":"ports.power","params":{"port":"all","power":"cycle","seconds":3}}'
```
