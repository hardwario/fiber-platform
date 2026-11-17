# AGENTS.md: working in the FIBER platform

The rules of this repository, for coding agents and for people. What the platform is and how it
is built is in [`docs/`](docs/README.md); the work and its state are in [`TASKS.md`](TASKS.md).
This file says how to work here.

## What this repository is

Everything that runs **on** a HARDWARIO FIBER: the setup that turns a plain Raspberry Pi OS into
a FIBER, the `fiber` program (the agent and its command line), and the firmware of the two
microcontrollers on the board. Its sibling, FIBER Setup (`../fiber-setup`), writes the system
onto a FIBER; this repository is what then runs there.

The FIBER is an industrial IoT gateway built on a Raspberry Pi Compute Module 4. Beside the CM4
the board has a **south bridge** (STM32C031C6: the status lights, the power of the 1-Wire ports,
two voltages) and a **north bridge** (STM32WL55: the radio, a gateway for TOWER nodes). The LCD,
the buttons, the buzzer, the 1-Wire master and the clock are wired to the CM4 itself.

The aim is a FIBER that anybody can start building on in a very short time. Between two ways of
doing something, take the one a Raspberry Pi user already knows, and the one with less in it.

## Start here

1. Read [`TASKS.md`](TASKS.md): what is in progress, what comes next, what was decided.
2. Read the page of [`docs/`](docs/README.md) for the part you are about to touch.
3. Put your task into "Tasks in progress" (below) before you change anything else.

## Tasks and decisions

[`TASKS.md`](TASKS.md) is the plan and its state. It has five sections, always in this order:
**Tasks in progress**, **Tasks ahead**, **Tasks done**, **Decisions ahead**, **Decisions done**.
Tasks are numbered `T1`, `T2`, ... and decisions `D1`, `D2`, ...

**Keep it true at every moment.** A session can end at any point: a closed window, a lost
connection, a full context. The next session has only this file to go on from. So the file is
changed before the work, during it and after it, never at the end of the day.

**Before you start a task**

1. Find it in "Tasks ahead", or add it there with the next free number if it is new.
2. Move it to "Tasks in progress".
3. Write its `State` line (the date and where it stands). Read its steps again and bring them
   up to date with what is known by now; a new task gets its steps here. Each step is small
   enough to finish and check in one go.
4. Save the file. Only then touch anything else.

**While you work**

- Tick a step when it is done and checked, at once, not in a batch at the end.
- Keep the `State` line current: what is half done, what the next step is, anything the next
  session would otherwise have to find out again. Write it before a step that takes long or can
  fail.
- When what you learn changes the plan, change the checklist. A task that turns out to be two
  becomes two.
- A question only the maintainer can answer goes to "Decisions ahead", with the choices and the
  one you recommend. Say in the task that it waits for it (`waiting for D14`), and go on with
  what does not depend on the answer.

**When the task is done**

1. Check its "Done when" line. If part of it could not be checked (no board on the bench, no
   push yet), the task stays in progress and its `State` says what is missing.
2. Move it to "Tasks done" with the date, what was checked, and its commits if there are any.
3. Look at the tasks that needed it and at the milestone table, and bring them up to date.

**Decisions.** An answered decision moves to "Decisions done" with the answer and the date. A
later decision that changes an earlier one says so in both; nothing is deleted.

**Numbers** are never reused or changed. `T7` and `D5` mean the same thing for ever, so commits,
comments and documents can name them.

## What to trust

- **The board:** the schematic, as written down in [`docs/hardware.md`](docs/hardware.md).
- **The radio protocol:** `github.com/hardwario/tower` at the pinned revision (below).
- **Package sources and versions:** the upstream documentation, as written down with its date in
  [`docs/provisioning.md`](docs/provisioning.md).
- **Not the earlier on-device software** (`hardwario/fiber-agent`,
  `hardwario/fiber-platform-legacy`). It is being replaced, and some of what it believes about
  the board is wrong: it assumed there are no reset and BOOT0 lines to the north bridge, and it
  drove a chip-select pin that is not fitted. Take nothing from it without checking it against
  the three sources above.

What has not been seen working on a real FIBER is written as not yet tried
([`docs/bench-checks.md`](docs/bench-checks.md)). Do not write that something works because a
document says it should.

## Layout and names

**The naming rule:** the directory `crates/X` holds the package `fiber-X`. `agent/` is
`fiber-agent`, and its program is `fiber`. The table of everything, with what exists so far, is
in [`docs/architecture.md`](docs/architecture.md).

- **The host workspace** is the root (`crates/*`, `agent`).
- **Each firmware is a workspace of its own** (`firmware/south`, `firmware/north`), left out of
  the host workspace and built inside its own directory, so that its target and linker settings
  apply. The firmware uses `crates/*` by path: the agent and the firmware are built from one
  `crates/protocol`.
- One `rust-toolchain.toml`, one `rustfmt.toml` (110 columns), one `LICENSE`, one `.github/`.

**Where knowledge goes.** Rules in this file, the work in `TASKS.md`, everything else in
`docs/`, by subject. A directory's own `README.md` is a few lines: what it is and how to build
it. Every interface is written in `docs/` before its code: the MQTT topics, the command line, the
messages between the agent and the firmware, the Bluetooth profile.

## The TOWER pin

The hardware-free crates of the TOWER radio stack (`tower-net`, `tower-radio`, `tower-protocol`,
`tower-hmi`) come from git at **one** revision, named once, in the root `Cargo.toml`. The `tower`
repository is never changed for the FIBER's sake (D1).

TOWER promises nothing about its radio between two revisions. Moving the pin is therefore a task
of its own in `TASKS.md`, never part of another change, and never Dependabot's:

1. Compare the parts of TOWER's firmware that are written again here
   (`firmware/src/radio/frame.rs`, `firmware/src/radio/net/`) between the old revision and the
   new one, and read what changed.
2. Move the `rev` of all four crates together, and the north bridge firmware's with them. If
   TOWER needs a newer Rust, move `rust-toolchain.toml` and the workflows too.
3. Run the tests that hold the pin (`crates/tower-air`, `agent`). One that fails means TOWER
   changed something on the air. It is not a test to adjust.
4. Run the bench against Core Modules flashed from the new revision.

## Commands

```sh
just check     # everything CI checks; needs no hardware
just test      # the tests of the host workspace
just fmt       # rustfmt
```

`just check` is fmt, clippy with `-D warnings`, the tests, the `thumbv7em` build of the `no_std`
crates, rustdoc, the static arm64 build of `fiber`, the release archive built twice and compared,
`shellcheck`, the bench scripts' self-test, the long-dash check and `cargo audit`. It needs
`cargo-audit`, `python3`, `shellcheck`, `dtc` and the internet. Run it before you call a step
done.

`just check-container`, `just check-install` and `just check-converge` run the program, the
installer and an apply in containers; `just lint-ansible` lints the playbook and the roles in one.
They need Docker on an arm64 machine; CI runs them on every push.

Nothing in `just check` touches a FIBER. The scripts in `hil/` do. Run one against a board only
when the maintainer asks for it.

## How we work

- **Straight on `main`.** No branches unless the maintainer asks for one.
- **Commit, tag and push only when the maintainer asks.**
- **No `unwrap`, no `expect`, no `unsafe`** in the host workspace (the tests may use the first
  two). The agent owns the board; it must not fall over on an error it could report.
- **One place for pin numbers:** the agent's board module, with `docs/hardware.md` as its
  reference. No GPIO number, device path or I2C address anywhere else.
- **On the FIBER, stay in our own places:** `/opt/fiber`, `/etc/fiber`, `/var/lib/fiber`. A file
  written anywhere else is named `fiber-*` or is a drop-in, and is on the footprint list
  ([`docs/provisioning.md`](docs/provisioning.md)).
- **Provisioning:** `ansible.builtin` modules only; no collection, no pip package, no container.
  A service gets its settings and its password before its first start, and none may ever answer
  with a default password.
- **The toolchain** is the one the pinned TOWER crates need. CI installs the same one
  (`RUST_TOOLCHAIN` in each workflow); its `audit` job fails when the two differ.
- **CI:** every GitHub Action is pinned by its full commit, with the version in a comment. The
  workflows may only read the repository. Every cargo call is `--locked`.
- **Licence:** MIT, © 2026 HARDWARIO a.s., one `LICENSE` at the root.

## Nothing to stay compatible with, yet

The platform has no users yet. MQTT topics, the messages to the firmware, the layout under
`/opt/fiber` and `/etc/fiber`, the command line and the Bluetooth profile may all still change.
Add no migrations and nothing that keeps an old form alive, and keep nothing compatible with the
earlier on-device software (D10). Keep only what finds a mismatch (the schema tags, the `schema`
number on MQTT), so that an old firmware or an old client is refused and not half understood.

## Writing style

The style is FIBER Setup's. It holds everywhere: the README, the documentation, what the `fiber`
program prints, comments in the code, commit messages, this file.

- **Plain words, short sentences.** Say what happens and what the reader does next: "The FIBER
  joins your network and installs the software."
- **No long dashes,** neither the em dash nor the en dash. Use a comma, a colon, a semicolon or
  brackets, or make two sentences. A plain hyphen is fine. `just check` fails on a long dash.
- **One name for one thing,** every time: the FIBER, the agent, the south bridge, the north
  bridge, the first start, a component.
- **A colon brings the explanation or the list.** List items are short and alike. A point worth
  finding again starts with a few words in bold, as in this list.
- **Headings say what the reader is looking for,** in a few words: "What it does", "How it
  works", "Good to know".
- **Code, file names, commands, topics and pins in backticks.** Names shown on a screen in
  italics.
- **Numbers as digits, with their units:** 5 minutes, 32 KB, 3.3 V.
- **Say what was tried and what was not.** "Not run on a FIBER yet" is a good sentence.
- **Nothing to sell.** No "simply", "powerful" or "seamless", no exclamation marks, no emoji.
- **Comments say why,** in the same voice, with the measured fact where there is one: "How long
  the gadget may take to show its eMMC (seen: about 10 s)."
- **Commit messages** are one sentence that says what the change does, starting with a verb and
  a capital letter, with no prefix: "Tell the user about a newer version". Details follow a
  colon or a semicolon: "Run on macOS: find the FIBER in the I/O Registry". A release is "FIBER
  Platform 1.2.0".
- **Licence** is the noun, as in FIBER Setup.

## Do not

- Do not start, pause or finish a task without bringing [`TASKS.md`](TASKS.md) up to date.
- Do not commit, tag or push unless the maintainer asked.
- Do not move the TOWER pin as part of another change, and do not change the `tower` repository.
- Do not take anything from the earlier on-device software unchecked.
- Do not put a pin number outside the agent's board module.
- Do not add an Ansible collection, a pip package or a container to the provisioning.
- Do not run a script from `hil/` against a FIBER unless the maintainer asked.
- Do not point a workflow at an action by its tag or branch, or give a workflow more rights.
- Do not use long dashes.
