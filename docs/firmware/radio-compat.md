# Keeping the two radio stacks together

The FIBER's gateway speaks TOWER's radio, and a part of TOWER's code is written again here
([`../architecture.md`](../architecture.md), "Two radio stacks"). TOWER promises nothing about its
radio between two revisions, and its nodes are not updated over the air. So a FIBER works with
TOWER nodes built from the pinned revision (the root `Cargo.toml`), and moving the pin is a task
of its own, done as this page says.

**Status: a draft (T27).** The drift report and the check of its list are built and run on the
computer. The pin has not moved since they were written (it is TOWER's newest commit on
2026-11-20), so the procedure has not been used for real yet. The bench run passed on 2026-11-20
with two Core Modules; the 24-hour run has not been made.

## What is written again here

Each file below is TOWER's at the pinned revision; the drift report shows each of them in full
when it changes. The list the tools use is `tower_watched` in the `justfile`.

| TOWER's file | Here | What |
|---|---|---|
| `firmware/src/radio/frame.rs` | `crates/tower-air/src/frame.rs` | The network frame: the header, the nonce, AES-128-CCM |
| `firmware/src/radio/net/mod.rs` | `crates/tower-air/src/gateway.rs` | The flow: confirmed delivery, the acknowledgement, the replay rule, the duty cycle |
| `firmware/src/radio/net/pairing.rs` | `crates/tower-air/src/gateway/pairing.rs` | Pairing over the air and the keyed join, the gateway's side |
| `firmware/apps/radio_gateway.rs` | `crates/tower-air/src/gateway.rs`, `firmware/north/src/main.rs` | The gateway application: what waits for a node and when it is sent |
| `firmware/src/radio/config.rs` | `firmware/north/src/radio.rs`, `crates/tower-air/src/packet.rs` | The radio's settings: the channels, the modulation, the sync word, the CRC, the whitening |
| `firmware/examples/net_confirmed.rs` | `crates/tower-air/tests/frames.rs` | The example the captured frames come from, with its key |
| `docs/firmware/radio.md` | [`radio.md`](radio.md) | TOWER's guide to its radio, the wire protocol among it |
| `docs/cli/mqtt-api.md` | [`../mqtt-api.md`](../mqtt-api.md), "The radio" | The MQTT API of TOWER's gateway, which `fiber/radio/` follows |
| `cli/src/gateway/topics.rs` | `agent/src/agent/radio/service.rs` | The topics of TOWER's gateway, as code |
| `cli/src/gateway/payload.rs` | `agent/src/agent/radio/service.rs`, `values.rs` | The payloads of TOWER's gateway, as code |

## What is not written here

- **The four pinned crates** (`crates/net`, `crates/radio`, `crates/protocol`, `crates/hmi` in
  TOWER) are compiled in: every accept and refuse, the handshake, the property tables. Their
  changes come with the pin by themselves, and the tests here hold what this side relies on (the
  captured frames, the flow, pairing, the property values). The report shows how much changed in
  each, and their commits.
- **Channel agility, frequency hopping and bulk transfer** (`net/afa.rs`, `net/fhss.rs`,
  `net/bulk.rs`) are not in the FIBER's gateway. They are off unless an application turns them on,
  and at the pin none does. The report shows how much they changed; an application of TOWER that
  turns one on is drift to read (it shows in `radio_gateway.rs`).
- **The toolchain:** TOWER's `rust-toolchain.toml` is in the report; a newer Rust there means a
  newer Rust here (AGENTS.md, "The TOWER pin").

## The drift report

```sh
just tower-drift origin/main    # or any revision of TOWER: a commit, a tag
```

It fetches TOWER (into `target/tower-drift`, so it needs the internet; not part of `just check`)
and prints, between the pin and that revision:

- **The commits** that touch a watched file, oldest first.
- **The watched files,** each diff in full, and a line for each one that did not change.
- **The rest, by size:** the four crates, AFA, FHSS, bulk, the toolchain, and the commits of the
  crates.

A person reads it. For each change they write down, in the task that moves the pin, one of three
things: nothing to do here; a change here (which file, with a test); or not compatible (nodes
built from the old revision and from the new one cannot share a FIBER, so the release says that
the nodes must be flashed again).

**The list stays true:** `just check-north` (in `just check` and in CI) fails when a watched file
is not there at the pin (TOWER moved it: the report would say nothing about it), when a TOWER file
named in a comment of `crates/tower-air` or `firmware/north` is not watched, and when this page
does not name a watched file.

## Moving the pin, step by step

1. **A task of its own** in `TASKS.md`, never part of another change, and never Dependabot's.
2. **The report:** `just tower-drift <new>`, read in full, each change written down in the task
   as above.
3. **The changes here first,** each with its test: the counterpart in the table, the captured
   frames again if the frame changed (from TOWER's `net_confirmed` on a Core Module, with its
   key).
4. **The pin:** the `rev` of all four crates in the root `Cargo.toml` together, then
   `cargo update -p tower-net -p tower-radio -p tower-protocol -p tower-hmi`, and the same in
   the two workspaces whose lock files hold TOWER's crates through `crates/tower-air`
   (`firmware/north`, `hil/radio-spike`): `just check-north` fails when the north bridge's
   revision is not the root's. A newer Rust in TOWER moves `rust-toolchain.toml` and
   `RUST_TOOLCHAIN` in each workflow with it.
5. **The tests:** `just check`. A test of `crates/tower-air` or the agent that fails means TOWER
   changed something on the air: it is not a test to adjust, it is a finding of step 2 missed.
6. **The bench run** below, against Core Modules flashed from the new revision.
7. **The release** says which revision of TOWER its nodes must be built from (`fiber version
   --all` and `fiber radio status` say it too).

## The bench run

What shows on a FIBER that the two stacks still speak to each other. Done by hand on 2026-11-20
(T25, T26), then with `hil/tower_bench.py`, which the steps below name.

**On the bench:** a FIBER with the release under test; Core Modules flashed with TOWER's
`radio_button` from the pinned revision (`tower flash`); TOWER's command line built from the
same revision on the computer, which drives each node through its USB console (`tower exec
"node pair"`, `tower exec "button simulate 100"`); SSH to the FIBER from the computer, where the
script reads its MQTT.

```sh
python3 hil/tower_bench.py clicks fiber@fiber.local /dev/cu.usbserial-1120 /dev/cu.usbserial-1140
python3 hil/tower_bench.py set fiber@fiber.local /dev/cu.usbserial-1120
python3 hil/tower_bench.py restarts fiber@fiber.local /dev/cu.usbserial-1120
python3 hil/tower_bench.py soak fiber@fiber.local /dev/cu.usbserial-1120 /dev/cu.usbserial-1140 --hours 24 --every 60
```

1. **Pairing**, by hand: `fiber radio pair`, and `node pair` on the node (`tower exec`). The
   node is listed by `fiber radio nodes`, named when its `Info` comes, and its `node status`
   names the FIBER's gateway; the code shows on the FIBER's display. The same for the second
   node.
2. **What they send arrives** (`clicks`): the nodes' clicks in turn, each on
   `fiber/radio/nodes/{addr}/event/button`, the counts of each without a gap.
3. **At the first try** (`clicks`): a node repeats a frame whose acknowledgement it did not get,
   byte for byte, after waiting 200 ms for it, and counts none of its tries. So a second try
   shows in the time from the click (the node's answer to `button simulate`) to its event on the
   FIBER: on the bench 0.08 s at the first try, 0.2 s or more later at the second. A click
   whose event comes more than 0.15 s later than the shortest delay, or never, was not at the
   first try. At least 99 % passes (M5).
4. **A `set` reaches them** (`set`): `led` set to `false` and back to `true` on `set/led`, each
   reported on `state/led` after the node's next click.
5. **A restart of each side:** a node (`restarts`: `system reboot`, then a click that must come
   at the first try; its count starts again at 1, and its counter is 1024 higher, TOWER's
   reserve); by hand, the north bridge (`fiber firmware flash north`, which ends in a reset, or a
   power cycle) and the agent (`sudo systemctl restart fiber-agent`), each followed by `clicks`.
   After the north bridge's restart the agent hands it each node's last counter again
   (`NodeAdd`): a frame at or below it would be refused as a replay.

**A replay sent on air** needs a radio that sends a captured frame again, and the bench has none:
the refusal itself is tested on the computer (`crates/tower-air/tests/gateway.rs`), and the bench
shows that each node's counter goes on across each restart (step 5).

**The 24-hour run** (`soak --hours 24 --every 60`): both nodes, a click a minute in turn (720
for each), their own reports between, a line every 10 minutes with the north bridge's state,
and the FIBER otherwise left alone. It passes with every
click on MQTT, at least 99 % at the first try, and the north bridge running without a fault
throughout. Not run yet.
