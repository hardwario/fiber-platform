# The agent's MQTT API

The agent shows the board on MQTT, and takes requests there. This page is the reference for
anyone who talks to a FIBER through its broker: a Node-RED flow, a program of your own, and the
`fiber` command line itself, which is a client of exactly these topics (D19).

**Status: a draft for the maintainer to read (T13).** Built in the agent (T17) and tested against
a simulated south bridge through Mosquitto; not run on a FIBER yet. This first part covers the
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
| `firmware/south`, `firmware/north` | agent | yes | [firmware/chip](#firmwarechip) |
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
| `battery` | The battery in volts, or `null` |

It changes when the source changes, or a voltage by more than 0.05 V; otherwise once a minute.
The source changes with a margin, so that it does not flap.

### `power/event`

```json
{"event":"battery","ts":1794952515}
```

`battery` when the FIBER goes from PoE to its battery, `poe` when it goes back.

### `ports/n`

One for each port, `ports/1` to `ports/8`:

```json
{"power":true,"led":"auto","ts":1794952515}
```

| Key | Meaning |
|---|---|
| `power` | Whether the port's 3.3 V is on |
| `led` | `auto` (the agent shows the port's state) or `set` (a client set it with `leds.set`) |

The thermometers on a port (M3) come with their own topics.

### `firmware/chip`

`firmware/south` and `firmware/north`:

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
  done (up to 2 minutes). A request with a bad `id`, a repeated `id` (the agent remembers the last
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

## Example

```sh
mosquitto_sub -h 127.0.0.1 -t 'fiber/#' -v
mosquitto_pub -h 127.0.0.1 -t fiber/cmd -q 1 \
  -m '{"id":"a1","op":"ports.power","params":{"port":"all","power":"cycle","seconds":3}}'
```
