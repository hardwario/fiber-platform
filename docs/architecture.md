# Architecture

How the pieces of the FIBER platform fit together, and why they are cut this way.

**Status:** built as this page has it, but for Bluetooth, and run on a FIBER on the bench
(2026-11-19 to 21); `TASKS.md` has the milestones, what is done and what was seen on a FIBER. The
decisions behind it are numbered there (D1, D2, ...).

## The pieces

```
 FIBER Setup (desktop)                         the FIBER (Raspberry Pi OS, CM4)
 +----------------------+   boot partition   +-----------------------------------------------+
 | component picker     | - user-data -----> | cloud-init: user, network, hostname           |
 | writes OS + settings | - fiber-install.sh | fiber-install.sh: wait, retry, then install.sh|
 +----------------------+ - fiber-seed.yaml  |         |                                     |
                                             |   /opt/fiber/current  (release archive)       |
                                             |   + bin/fiber -- platform apply --> ansible   |
                                             |   + provision/  roles: base agent mosquitto   |
                                             |                 node-red chirpstack           |
                                             |   fiber agent (daemon) --- MQTT fiber/... ----+--> Node-RED, your code,
                                             |    + Linux: LCD, buttons, buzzer, w1, RTC     |    fiber CLI
                                             |    + uart4 - south bridge (STM32C031)         |
                                             |    + uart3 - north bridge (STM32WL55) - radio +--> TOWER nodes
                                             +-----------------------------------------------+
```

- **Provisioning** turns a vanilla Raspberry Pi OS into a FIBER, component by component, and can
  be re-applied at any time: [`provisioning.md`](provisioning.md).
- **The agent** is the one process that owns the board and presents it over MQTT.
- **The south bridge** (STM32C031) is a small I/O coprocessor: status LEDs, 1-Wire port power, two
  voltages.
- **The north bridge** (STM32WL55) is the radio: a gateway for TOWER nodes.

The board itself is described in [`hardware.md`](hardware.md). Note what is wired to the CM4 and
not to a microcontroller: the LCD, the buttons, the buzzer, the 1-Wire master, the clock.

## Repository layout

The directory `crates/X` holds the package `fiber-X`.

| Path | Package | Role | Exists |
|---|---|---|---|
| `crates/protocol` | `fiber-protocol` | `no_std`, self-contained: COBS + CRC-32 + postcard framing with its own schema tag, and the two link schemas (south, north). The one wire contract between the agent and the firmware. | the frame, the south link, and the north link's messages (T25) |
| `crates/flasher` | `fiber-flasher` | The ROM-bootloader engine (ST AN3155) for STM32C0 and STM32WL; reset lines behind a trait; a fake ROM for tests. | the engine and the simulated chip; the agent writes both bridges with it on a FIBER |
| `crates/south-core` | `fiber-south-core` | `no_std`: the south bridge's logic, host-tested; also builds as a host simulator. | the logic, and the simulator (`fiber south-simulator`) |
| `crates/tower-air` | `fiber-tower-air` | `no_std`: software packet codec, on-air frame codec and gateway flow of the TOWER personality, over a radio trait and the pinned TOWER kernels. | the packet codec and the frame codec, tested against captures of a Core Module (T23, T24); the gateway's flow for known nodes, tested against a made-up radio and the captured frames (T24); pairing a node and the keyed join, tested against a node made up from TOWER's kernels (T47); pin checks |
| `crates/subghz` | `fiber-subghz` | `no_std`, no dependencies: the commands of the radio inside the STM32WL over a bus trait (the SUBGHZSPI in the firmware), and the RF switch as a trait. Layer 1 of the north bridge. | GFSK, the TCXO, the calibrations, the low-power PA; tested against the bytes the bench proved (T23) |
| `agent/` | `fiber-agent` (bin `fiber`) | Daemon, command line and platform tool. | `fiber version`, `doctor`, `platform ...`, `agent` (the south bridge, the thermometers, the buttons, the buzzer, the backlight and the radio on MQTT; writing both bridges' firmware; the LCD), the board's commands, `radio ...`; all of it run on a FIBER (2026-11-19 to 21) |
| `firmware/south/` | `fiber-south` | Embassy, STM32C031C6 (thumbv6m). Own workspace. | 15.8 KB; runs on a FIBER, flashed by the agent |
| `firmware/north/` | `fiber-north` | Embassy, STM32WL55 M4 core (thumbv7em). Own workspace. | 62.8 KB; runs on a FIBER, written by the agent (since 2026-11-20) |
| `provision/` | | `install.sh`, `components.yaml`, the Ansible roles, the `config.txt` block. | `components.yaml`, `install.sh`, `release.py`, the roles, the ready-made image's build |
| `hil/` | | Bench scripts, run by hand on a FIBER, and the radio spike's firmware (T22). | bootloader probe, the north bridge's flash read and written, an LCD test pattern, the radio spike, the TOWER bench run |
| `examples/` | | Clients of the MQTT API to copy: the command line, Python, Rust (a project of its own), Node-RED flows ([`examples.md`](examples.md)). | each run in the converge test, and once on a FIBER |

Two kinds of Cargo workspace, as in TOWER: the host workspace at the root, and one workspace per
firmware, each built inside its own directory so that its target and linker settings apply. The
firmware depends on `crates/*` by path, so the agent and the firmware compile against one
`crates/protocol`: a build can never mix link schemas. An installed artifact from another build is
caught at run time by the schema tag every frame carries.

## The agent

One process owns both UARTs, the GPIO lines, I2C, SPI and Bluetooth. A core task holds the state;
MQTT, the LCD and BLE are adapters around it, so the front panel keeps working when the broker is
down.

- **Board map.** One module holds every pin number, keyed by hardware revision.
- **Firmware.** Both firmware images are compiled into the binary. Each microcontroller's hello
  frame carries the CRC-32 it computed over its own flash; the agent programs the chip when that
  differs from its own image, when the schema tag differs, or when the chip stays silent. Attempts
  are bounded and remembered, so a chip that cannot be programmed does not wear its flash out.
- **Linux side.** Thermometers come from the kernel's 1-Wire tree (the DS2482 driver; one bus
  master per port), behind a trait so a userspace driver can replace it. The LCD is drawn through
  `spidev` with TOWER's `tower-hmi` framebuffer and menus. Buttons and buzzer use the GPIO
  character device. The clock is the kernel's business.
- **Command line.** Every subcommand that reads or commands the board is a client of the MQTT API,
  so everything the command line can do, a Node-RED flow can do too, flashing included. The
  exceptions: `fiber doctor` only reads what Linux shows of the hardware, and `fiber firmware
  chip` talks to a bootloader directly and needs the agent stopped.
- **Privileges.** The user `fiber-agent`, in the serial, GPIO, I2C and SPI groups (Bluetooth comes
  with M6), under `fiber-agent.service`: no capabilities, the system read-only, only the board's
  device classes, and a watchdog its loop feeds ([`provisioning.md`](provisioning.md), "The
  agent"). No TLS and no broker inside the agent: it requires the local Mosquitto.

### MQTT conventions

Taken from TOWER's gateway API: one prefix (`fiber/`), QoS 1, state retained, events not retained,
requests on `cmd` answered on `rsp/{id}`, and a retained `status` with a last will and a `schema`
number that changes whenever a subscriber would notice. The reference is
[`mqtt-api.md`](mqtt-api.md), written before the code. The shape:

| Topic | Retained | Content |
|---|---|---|
| `status` | yes (last will) | state, `schema`, version, hardware revision, serial |
| `power`, `power/event` | yes / no | source (PoE or battery), input and battery voltage |
| `ports/{1..8}` | yes | power, LED, probes found, fault |
| `thermometers/{rom}` | yes (empty = gone) | port, temperature, timestamp |
| `buttons/event` | no | button, gesture |
| `display` | yes | backlight, screen |
| `firmware/{south,north}` | yes | name, version, CRC, state, reset cause |
| `radio/status`, `radio/pairing` | yes | personality, band, channel, the pinned TOWER revision |
| `radio/nodes/{addr}`, `.../state/{prop}` | yes | a node and its properties |
| `radio/nodes/{addr}/event/{prop}`, `.../set/{prop}` | no / never | events, and commands to a node |

## South bridge

One idempotent frame in place of a command set. The agent sends the state it wants (`Desired`:
every LED and the port power mask) once a second; the firmware answers with what it applied and
the raw ADC counts (`Status`), and says hello at boot (`Hello`: identity, image CRC, reset cause,
fault code, option bytes).

- The ports are switched on at boot, one after another, so 1-Wire works without the agent.
- No `Desired` for five seconds: the LEDs show an "agent down" pattern; port power is left as it is.
- The firmware keeps no state across a reset: the agent's next frame restores everything.
- 32 KB of flash: no formatting code, no floats; ADC counts are scaled in the agent. The budget is
  24 KB, enforced in CI.
- At its first start the firmware clears the `nBOOT_SEL` option bit, once, so that the BOOT0 pin
  works for ever after ([`hardware.md`](hardware.md), "Flashing and recovery"). Nothing else ever
  writes option bytes.

## North bridge

Layers, so that a second radio personality fits later without a rewrite:

1. the sub-GHz command layer (the radio inside the STM32WL, the RF switch, the TCXO);
2. personalities, one active at a time: `TowerFsk` first, `StickerLora` later;
3. the link to the CM4, which names the active personality.

### The TOWER personality

A gateway for TOWER nodes: star network, registered nodes only, confirmed delivery with
acknowledgement, replay protection, pairing over the air with a code shown on the LCD, one pending
downlink per sleeping node, the EU 868 duty-cycle budget. US 915 (frequency hopping), remote shell
and bulk transfer come later.

- **Packets in software.** TOWER nodes use a SPIRIT1 radio. The STM32WL's packet engine has the
  same whitening polynomial and seed but, by the documents, the opposite bit order in each byte, so
  it cannot produce or undo SPIRIT1's whitening itself. The radio therefore runs in a raw
  fixed-length mode with sync-word detection, and whitening, the length byte and the CRC are one
  pair of pure, host-tested functions (ST ships the same approach for this chip). The radio spike
  confirms the details ([`bench-checks.md`](bench-checks.md), check 4).
- **The firmware is a transparent bridge.** It authenticates and decrypts at the network layer and
  forwards a node's application payload untouched; the agent decodes it with TOWER's own schema
  crate. A new kind of node needs no firmware change.
- **The firmware holds no registry.** Node keys, names and replay floors live in the agent and are
  pushed to the microcontroller whenever it says hello. The microcontroller persists one thing, a
  transmit-counter watermark, and the counter restarts from the higher of that and the agent's
  own watermark: a restored disk image can then never make the radio reuse an encryption nonce.

### Two radio stacks

TOWER's gateway and this one are two implementations of one protocol, and the TOWER repository is
not changed for FIBER's sake. What keeps them compatible:

- **One pin.** The hardware-free TOWER crates (`tower-net`, `tower-radio`, `tower-protocol`,
  `tower-hmi`) are git dependencies at one revision, named once in the root `Cargo.toml`. The
  security decisions and the radio arithmetic are therefore TOWER's own code.
- **Vectors.** Frames captured from a real Core Module with a known key (the bytes on the air, the
  frame, the nonce, the encrypted payload) are test vectors here.
- **A drift report.** The parts of TOWER that are written again here (its frame, its net and
  pairing, its gateway application, its radio settings, its gateway's MQTT API) are diffed
  between the old and the new revision whenever the pin moves (`just tower-drift`), and a person
  reads the diff. The list and the steps of a move are in
  [`firmware/radio-compat.md`](firmware/radio-compat.md).
- **A bench run** against Core Modules flashed from the pinned revision before any release that
  touches the radio.

TOWER promises no compatibility between revisions until it has users, and its nodes are not
updated over the air. So a FIBER's radio is compatible with **TOWER nodes built from the pinned
revision**, `fiber version --all` and `fiber radio status` say which one that is, and a change of
TOWER's wire format means reflashing the nodes and updating the FIBERs together.

## Flashing and recovery

`fiber-flasher` raises BOOT0 and pulses NRST from the CM4 (NRST is only ever pulled low or
released, never driven high), talks 8E1 to the ROM bootloader and returns the port to 8N1 for the
firmware. It checks the image before it erases anything, erases by page list, writes the block
that holds the vector table last and only after the rest has been read back, and leaves through
the bootloader's `Go` command. It refuses a chip with readout protection.

"The CM4 can always recover a microcontroller" holds as long as the four series resistors are
fitted, the south bridge's `nBOOT_SEL` bit is 0, and readout protection stays off.

## Testing without a board

- The south and north logic are hardware-free crates that also build as simulators on a
  pseudo-terminal, so the agent's MQTT contract is tested against them.
- A fake ROM bootloader behind the reset-line trait, a fake 1-Wire tree under a configurable
  root, an in-process broker, snapshots of LCD frames.
- The playbook converges twice in a Raspberry Pi OS container; the second run must change nothing.
