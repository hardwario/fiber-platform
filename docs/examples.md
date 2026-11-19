# Examples

Small programs that use the agent's MQTT API ([`mqtt-api.md`](mqtt-api.md)), to copy and change.
They are in [`examples/`](../examples).

**Status: written (T31).** Each one runs in CI, in the converge test, against the agent's service,
a simulated south bridge and a made-up 1-Wire tree. Not run on a FIBER yet.

## What they need

- **The FIBER platform** with the `agent` and `mosquitto` components; the flows need `node-red`
  too.
- **On the FIBER itself** they talk to the broker on `127.0.0.1:1883`, without a login.
- **From another computer** they need `mqtt.lan` and the login typed into FIBER Setup. The shell,
  Python and Rust examples read where the broker is from four variables:

| Variable | Meaning | Default |
|---|---|---|
| `FIBER_HOST` | The broker | `127.0.0.1` |
| `FIBER_PORT` | Its port: 1884 from the network | `1883` |
| `FIBER_USER` | The login's user name | none |
| `FIBER_PASSWORD` | The login's password | none |

## The command line

`mosquitto_sub` and `mosquitto_pub` come with the `mosquitto` component. Everything the agent
says, as it says it:

```sh
mosquitto_sub -h 127.0.0.1 -t 'fiber/#' -v
```

**A request and its answer:** [`examples/shell/request.sh`](../examples/shell/request.sh) sends
one request and prints its answer. It uses `mosquitto_rr`, which subscribes to the answer's topic
before it sends the request, so no answer is missed.

```sh
examples/shell/request.sh buzzer.beep '{"pattern":"double"}'
examples/shell/request.sh display.message '{"text":"Hello\nFIBER","seconds":60}'
examples/shell/request.sh ports.power '{"port":3,"power":"cycle","seconds":5}'
```

It prints the answer, `{"id":"...","ok":true,...}`, and ends with 0 when it says `"ok":true`,
1 when it does not, and 2 when no answer came.

## Python

With Debian's paho-mqtt (2.1 on Raspberry Pi OS trixie):

```sh
sudo apt install python3-paho-mqtt
```

- [`thermometers.py`](../examples/python/thermometers.py) prints each thermometer's reading as it
  changes, and a line when a probe goes. It checks the API's schema number first.
- [`request.py`](../examples/python/request.py) sends one request and prints its answer, with the
  same arguments and exit statuses as `request.sh`.

```sh
python3 examples/python/thermometers.py
python3 examples/python/request.py leds.set '{"led":"port1","colour":"red"}'
```

## Rust

[`examples/rust/`](../examples/rust) is a project of its own, with `rumqttc` as the agent uses it
(without TLS) and its blocking client. It prints each thermometer's reading, and with a limit it
sounds the buzzer once when a probe goes above it, and again only after it was half a degree
below.

```sh
cd examples/rust
cargo run --release -- 30
```

To run it on the FIBER, build it there, or build it on a computer for the FIBER as `just
build-examples` does: `cargo build --release --target aarch64-unknown-linux-musl`, with the
`rust-lld` linker of this repository's `.cargo/config.toml`.

## Node-RED

Two flows to import (*Menu > Import*, then pick the file). Each brings its own broker node,
*FIBER (localhost:1883)*, so it imports into any Node-RED; the starter flow's broker works as well.

- [`thermometer-alarm.json`](../examples/node-red/thermometer-alarm.json): when a thermometer goes
  above 30 °C, the display says which one and how warm, for 10 minutes or until a button is
  pressed, and the buzzer sounds the alarm. Once more only after it was a degree below. The limit
  is a line in its function node.
- [`button-beep.json`](../examples/node-red/button-beep.json): holding *enter*, the middle button
  beside the display, for 0.8 s sounds a beep. The display follows the buttons too: a click on
  *enter* opens its menu, but a hold does nothing on its pages (in the menu it goes back).

Both send their requests to `fiber/cmd` and show the answers in the debug sidebar.

A flow for what a TOWER node sends comes with the radio (T26).

## Good to know

- **A request is never retained,** and each has an `id` of its own: the agent drops a repeated
  one.
- **Subscribe to `fiber/rsp/+` (or the one answer's topic) before the request,** and match the
  answer by its `id`.
- **The retained topics** give a new subscriber the latest state at once: the thermometers, the
  ports, the power, `status`.
- **A message can come twice** (QoS 1). The examples do not mind: they print a reading again, or
  sound the buzzer only when a probe goes above the limit, not while it stays there.
