<p align="center">
  <img src="docs/images/hardwario-fiber-logo-stacked.svg" alt="HARDWARIO FIBER" height="64">
</p>

<h1 align="center">FIBER Platform</h1>

<p align="center">
  Everything that runs on a HARDWARIO FIBER:<br>
  its setup, its agent and its firmware.
</p>

<p align="center">
  <a href="https://github.com/hardwario/fiber-platform/actions/workflows/ci.yml"><img src="https://github.com/hardwario/fiber-platform/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/hardwario/fiber-platform" alt="Licence: MIT"></a>
</p>

<p align="center">
  <a href="#where-it-stands">Where it stands</a> ·
  <a href="#what-it-will-give-you">What it will give you</a> ·
  <a href="#how-it-works">How it works</a> ·
  <a href="#building-from-source">Building from source</a> ·
  <a href="docs/README.md">Documentation</a>
</p>

## What it is

FIBER Platform is the software on the HARDWARIO **[FIBER](https://docs.hardwario.com/fiber/)**, an
industrial IoT gateway built on a Raspberry Pi Compute Module 4.
[FIBER Setup](https://github.com/hardwario/fiber-setup) writes Raspberry Pi OS onto a FIBER. This
repository is what then runs there:

- the **setup** that turns the plain system into a FIBER, one component at a time, and that you
  can apply again whenever you change your mind;
- **`fiber`**, one program: the agent that shows the board on MQTT, and the command line to ask
  it and tell it things;
- the **firmware** of the two microcontrollers on the board, written in Rust with Embassy.

The aim: tick a few boxes in FIBER Setup, start the FIBER, and build your application on a system
that is already wired together, and that you can read and change.

## Where it stands

**Not ready to use yet.** The plan, the design and the map of the board are written, and the
foundations are in place: the repository's rules, its checks, and a `fiber` program that knows
its version and can choose components (`fiber platform list`, `status`, `enable`, `disable`).
Nothing here installs on a FIBER yet, and nothing has been tried on a real board.

[`TASKS.md`](TASKS.md) lists every task, what is in progress and what is done.

## What it will give you

In FIBER Setup you choose what the FIBER installs on its first start:

| Component | What you get |
|---|---|
| Basic system configuration | The board's hardware working under plain Raspberry Pi OS: the real-time clock, both serial links, the 1-Wire ports, the display's bus |
| FIBER agent | The board on MQTT and on the command line: thermometers, status lights, buttons, the display, the buzzer, the radio |
| Mosquitto | An MQTT broker |
| Node-RED | Flows wired to the broker, with one to start from |
| ChirpStack | A LoRaWAN network server. With a concentrator card in the FIBER it is the gateway too |

With nothing chosen the FIBER stays a plain Raspberry Pi OS.

### Good to know

- **Each component can be turned on or off later,** with one command on the FIBER. Your choice is
  one file you can read, `/etc/fiber/platform.yaml`.
- **The radio makes the FIBER a gateway** for [HARDWARIO TOWER](https://github.com/hardwario/tower)
  wireless nodes: pair a node, and what it measures arrives on MQTT.
- **Everything the command line can do, your own program can do:** the command line talks to the
  agent over the same MQTT topics.
- **The firmware looks after itself.** The agent carries the firmware of both microcontrollers
  and writes it to them when theirs differs.
- **Plain Raspberry Pi OS stays plain.** Services come from their own package sources, with their
  settings where you expect them. No containers.

## How it works

1. **FIBER Setup** writes Raspberry Pi OS onto the FIBER, with your settings and your choice of
   components.
2. **On the first start** the FIBER fetches a release of this platform, checks it against its
   checksum and unpacks it under `/opt/fiber`. A later release goes beside it, so going back is
   one step.
3. **`fiber platform apply`** brings the system to what `/etc/fiber/platform.yaml` says. It runs
   Ansible on the FIBER itself, one small role per component, and can be run again at any time.
4. **The agent** is the one program that owns the board. It reads the thermometers, drives the
   display, the buttons, the buzzer and the status lights, and shows all of it on MQTT under
   `fiber/...`.
5. **The two microcontrollers** each sit on a serial link of the Compute Module, which can also
   write their firmware. The south bridge switches the status lights and the power of the 1-Wire
   ports; the north bridge is the radio.

The board, pin by pin, is in [`docs/hardware.md`](docs/hardware.md); how the pieces fit together
is in [`docs/architecture.md`](docs/architecture.md).

## Building from source

Requirements: [rustup](https://rustup.rs) (the Rust version this repository pins installs
itself), [`just`](https://just.systems) 1.59 or newer, and `python3`.

```sh
git clone https://github.com/hardwario/fiber-platform.git
cd fiber-platform

just test                              # the tests
cargo run -p fiber-agent -- version    # the fiber program
just build-fiber                       # the fiber program for the FIBER: static, arm64 Linux
just check                             # everything CI checks
```

`just check` also needs `cargo-audit` (`cargo install --locked cargo-audit`) and the internet. It
touches no hardware.

## Project layout

| Path | Contents |
|---|---|
| [`agent`](agent) | `fiber`: the platform's commands, the agent and the command line |
| [`crates`](crates) | What the agent and the firmware share (`crates/X` is the package `fiber-X`) |
| `firmware` | The firmware of the south bridge (STM32C031) and of the north bridge (STM32WL55). To come |
| [`provision`](provision) | The components (`components.yaml`); the installer and the Ansible roles, one per component, to come |
| [`hil`](hil) | Scripts that are run by hand on a FIBER on the bench |
| [`docs`](docs/README.md) | All documentation |

## Documentation

| Page | What it holds |
|---|---|
| [The hardware map](docs/hardware.md) | The board as the software sees it: the chips, the pins, how the microcontrollers are written |
| [Architecture](docs/architecture.md) | The pieces and how they fit: the agent, both firmwares, the radio |
| [Provisioning](docs/provisioning.md) | The components, the file that holds your choice, the release archive, the package sources |
| [Bench checks](docs/bench-checks.md) | What has to be seen on a real FIBER first, and what was seen |
| [The command line](docs/cli.md) | The `fiber` commands and their exit statuses |

## Contributing

Issues and pull requests are welcome. Before a pull request, run `just check`. A few things to
keep:

- [`AGENTS.md`](AGENTS.md) has the rules of the repository, for people and for coding agents;
- [`TASKS.md`](TASKS.md) is the plan: a change belongs to a task there;
- an interface (MQTT topics, the command line, the messages to the firmware) is written in
  `docs` before its code;
- no long dashes, in code, comments, documentation or commit messages.

## Licence

MIT, © 2026 HARDWARIO a.s. See [LICENSE](LICENSE).

The MIT licence covers the source code. The HARDWARIO and FIBER names and the HARDWARIO FIBER logo
are HARDWARIO a.s.'s and are not licensed for use in derived products.

The radio builds on parts of [HARDWARIO TOWER](https://github.com/hardwario/tower) (MIT). The
services a FIBER installs (Mosquitto, Node-RED, ChirpStack) are downloaded from their own sources,
under their own licences, and are not part of this repository.
