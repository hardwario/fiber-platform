# TASKS

The plan of the FIBER platform and where it stands: every task (`T1`, `T2`, ...) and every decision
(`D1`, `D2`, ...), each in one of the five sections below.

This file is kept true at every moment, so that a session can stop anywhere and the next one can
go on from here alone. [`AGENTS.md`](AGENTS.md), "Tasks and decisions", says how. Numbers are
never reused or changed.

**The goal.** Someone ticks a few boxes in FIBER Setup, starts the FIBER, and finds a working,
pre-wired IoT stack on plain Raspberry Pi OS that they can read, change and apply again.

**Where the details are.** A task says what to do, in which order, what to watch out for and how
to tell that it is done. The facts it works with are in the documents: the board in
[`docs/hardware.md`](docs/hardware.md), the design in
[`docs/architecture.md`](docs/architecture.md), the components and their package sources in
[`docs/provisioning.md`](docs/provisioning.md), the questions for the bench in
[`docs/bench-checks.md`](docs/bench-checks.md).

| Milestone | What | Tasks | Done when |
|---|---|---|---|
| M0 | Foundations and bench checks | T1 to T3 | Both microcontrollers answer their bootloader from the CM4; CI is green (T1 done) |
| M1 | Provisioning | T4 to T12 | A FIBER written with every box ticked comes up by itself; a second apply changes nothing |
| M2 | The agent's core and the south bridge | T13 to T18 | The agent flashes the south bridge on its own and shows the board on MQTT |
| M3 | Thermometers | T19 | A DS18B20 shows up on MQTT and in a Node-RED flow |
| M4 | The front panel | T20, T21 | The LCD shows the name and the address; the buttons move through the pages (held on a FIBER, 2026-11-19) |
| M5 | The north bridge | T22 to T27, T47 | TOWER nodes pair with the FIBER and their events arrive on MQTT |
| M6 | Bluetooth | T28, T29 | A phone puts a FIBER on Wi-Fi |
| M7 | Documentation and polish | T30 to T33 | A new user gets from the box to a thermometer in a flow with the documentation alone |

**The order.** M1 and M2 do not depend on each other and can go side by side. The three tasks
that answer the biggest unknowns come first: T2 (do the bootloaders answer), T3 (does the south
bridge firmware fit) and T22 (does the radio hear a TOWER node), which can start as soon as T2
is done, long before the rest of M5.

## Tasks in progress

### T2. Bench checks on a FIBER (M0)

State (2026-11-19): everything that needs no eyes, ears or hands is answered, on one unit; what is
left needs someone at the bench. Evening: the FIBER shut down cleanly (release `0.1.0-bench.12`,
the north bridge holding the spike's receive image) for the maintainer to fit a RAK5146 card;
check 3 next, with nothing driving GPIO11 first. The FIBER (reachable over SSH as `fiber`, sudo with FIBER Setup's
default password) is a plain Raspberry Pi OS of 2026-10-06 (trixie, kernel 6.18.60) on a CM4 rev
1.1 with 1 GB (`a03141`), written by FIBER Setup 1.2.0; the platform is installed on it since
(see T12). A J-Link on the computer is wired to the south bridge's SWD header; it was used last,
to read the south bridge's pins (T16), and that confirmed the warning below: since then the CM4
cannot reach the south bridge's bootloader (`fiber firmware chip south` gets `0x00`), until the
next power-on. The firmware runs; a new one would have to go through the J-Link. Answered: both bootloaders (check 1); the I2C bus and its three chips; the clock;
`/dev/spidev6.0` and PWM channel 1 (check 2 and 6 in part, all in `docs/bench-checks.md`). The
earlier software's LCD driver was read: ST7920, mode 3, 400 kHz, and the picture turned by 180
degrees. The maintainer looked: the agent's pages show, and the panel is mounted upside down, so
the turn stays (T20). The maintainer saw the pages upright, the backlight lit, the buttons and the
buzzer working. After a power cycle (2026-11-19) the buzzer stayed silent at power-on, and the CM4
reached the south bridge's bootloader again (the J-Link's reads had shut it out until then). Left,
for someone at the bench: a RAK5146 card in the slot; a second unit. The test pattern is
`hil/lcd_test.py` (a copy is in the home directory of `fiber` on the unit; stop the agent first,
it drives the LCD).

What only a real board answers. Flashing, recovery, the LCD and the ChirpStack role all rest on
these answers, and each takes minutes ([`docs/bench-checks.md`](docs/bench-checks.md), checks 1 to
3 and 6).

- [x] Write a FIBER-R2 rev 2.1 with FIBER Setup, with the FIBER software turned off, and log in
      over SSH (FIBER Setup 1.2.0, the image of 2026-10-06)
- [x] Add `dtoverlay=uart3` and `dtoverlay=uart4` to `/boot/firmware/config.txt`, restart, and
      see `/dev/ttyAMA3` and `/dev/ttyAMA4`
- [x] Run `sudo python3 bootloader_probe.py` (from `hil/`). North bridge: ACK, product id
      `0x497`, `nSWBOOT0` = 1. South bridge: ACK, product id `0x453`
- [x] Write down the south bridge's option bytes as a production unit has them. `nBOOT_SEL` = 1
      on a unit whose firmware runs means the CM4 cannot reach that chip's bootloader at all
      (this unit: `0xFEFFFEAA`, `nBOOT_SEL` = 0)
- [ ] If a second unit is at hand, run the probe on it too: the question is what production
      units carry, not what one board does
- [x] `i2cdetect -y 10` (after `dtparam=i2c_vc=on`): 0x18 (DS2482), 0x19 (LIS2DH12) and 0x51
      (PCF85063A) answer. The schematic leaves the bus's pull-up resistors unfitted
- [x] The LCD: add `dtoverlay=spi6-1cs`, see `/dev/spidev6.0`, and get a test pattern onto the
      display with chip select on GPIO18 and reset on GPIO16 (the earlier software used SPI mode
      3 at 400 kHz, with `0xF8` before a command and `0xFA` before data). Nothing may object to
      the overlay taking GPIO19 (2026-11-19: the agent's pages show, sent that way)
- [x] The backlight on PWM channel 1 (`dtoverlay=pwm,pin=13,func=4`): it lights, at the 80 % the
      agent sets, and dims after a minute without a press (the maintainer, 2026-11-19)
- [x] The buttons on GPIO23, 24 and 25 read low while pressed (2026-11-19: they read high when not
      pressed, and the agent's pages follow them)
- [x] The buzzer sounds with GPIO17 low (2026-11-19, heard)
- [x] Listen at power-on too: until `config.txt` sets the line, only the Raspberry Pi's weak
      pull-down holds it (2026-11-19: silent through a power cycle, the maintainer listened)
- [ ] With a RAK5146 USB card in the mini-PCIe slot: which level on GPIO11 (pin 22) lets the
      card appear on USB (`0483:5740`). 2026-11-19: the card is in, but has no power: its
      converter (`U27`) runs from `BAT+`, and the unit has no cell; next with a cell fitted
- [ ] Write every result, with the date, into `docs/bench-checks.md`, and correct
      `docs/hardware.md` where the board disagrees with it

Watch out: the probe resets both microcontrollers, and no other program may hold the two serial
ports (stop the earlier FIBER software if the unit has it). After any access over SWD an
STM32C0 keeps BOOT0 low until the next power-on, so do not mix a debug probe into this check.

- Needs: a FIBER-R2 rev 2.1 on the bench, reachable over SSH.
- Done when: every result is in `docs/bench-checks.md`.

### T9. The `chirpstack` role (M1)

State (2026-11-19): runs on a FIBER without a card, after one fix: ChirpStack refuses a password
shorter than 8 characters ("Password must be at least 8 characters"), and FIBER Setup suggests
`fiber`, so with its default the component failed at every try. The role now says so before it
sets anything, with what to do (T36); what FIBER Setup should do is D25. With a longer password
in `login.json`, as the message says, it came up: its login works, admin/admin is refused, Redis
answers on loopback only, ChirpStack takes 43 MB. Waiting for a card.
Earlier (2026-11-18): everything else is done, commit `26a78d0`, and its CI run (44197050069) is green: the API never answers to admin/admin during the
apply, the seed's login works, Redis listens on loopback only, the API key is kept, the
concentrator daemon and its udev rule are there, a second apply changes nothing. Tried
first in a container with ChirpStack 4.20.0's own arm64 binary: `set-password --password-file`
and `create-api-key` work before the first start, as the user `chirpstack`; `--stdin` wants a
terminal. Settled: ChirpStack's user stays `admin` (its command line can only set the password
of a user that exists), with the password typed into FIBER Setup; the service stays off until
its password is set; the package's `region_eu868.toml` is used as it is (it has the eight
channels); the MQTT forwarder runs only with the concentrator daemon, both started by udev. Read
in the daemon's source (v4.7.1): over USB it sets up no reset line, so GPIO17, the buzzer, is
left alone.

A LoRaWAN network server on the FIBER, and with a RAK5146 USB card in the mini-PCIe slot also the
gateway.

- [x] ChirpStack's package source (in `docs/provisioning.md`, key vendored, fingerprint
      checked); `chirpstack-sqlite` and `chirpstack-mqtt-forwarder`, installed so that they do
      **not** start (`policy_rc_d: 101`); Debian's `redis-server`, on loopback
- [x] `/etc/chirpstack/chirpstack.toml`: SQLite at `/var/lib/chirpstack/chirpstack.sqlite`
      (write-ahead log, a busy timeout of 5 s), the API's address and a secret of our own, the
      region from `lorawan.region`, the MQTT integration on the local broker in JSON. The
      package's own region file for EU 868
- [x] Before the first start, as the user `chirpstack`: `set-password --email admin
      --password-file ...` from a file in `/run` that is deleted at once, then `create-api-key
      --name fiber`, once (every call makes a new key); the key goes to `/etc/fiber/secrets`
- [x] The concentrator daemon: the sx1302 tarball by its sha256, its settings (`rak_5146`,
      `USB`, the port `/dev/rak5146`, its sockets under `/run`), our own unit bound to the
      card's device
- [x] The udev rule: USB `0483:5740` gets the name `/dev/rak5146` and starts the daemon. The
      daemon is always installed with the component, never "when a card is there": the card can
      appear long after the start
- [x] The MQTT forwarder on the local broker, with the same topic prefix as the region
- [x] In CI: at no moment does a socket answer with admin/admin; a second apply changes nothing
- [ ] On a FIBER with the card: the gateway shows uplinks in ChirpStack

Watch out: the package would otherwise start at once on `0.0.0.0:8080` with admin/admin. Its
source keeps one version of each package, so an exact pin breaks at the next upstream release:
pin the major version. `0483:5740` is the id of any STM32 serial port, not of this card alone.
The card is fed from the battery rail and goes away when the battery is low.

- Needs: T6, T8; T2 (the level on mini-PCIe pin 22) for the part with a card.
- Done when: the CI checks pass, and a FIBER with a card shows uplinks.

### T12. FIBER Setup: the component picker (M1)

State (2026-11-19): measured on a FIBER with 1 GB, every component ticked, the installer run by
hand from a release served on the FIBER itself (FIBER Setup's first start was not tried): 6 min
23 s over Wi-Fi to the end of the apply (the restart not counted), 147 MB downloaded, 687 MB more
on disk (895 MB before npm's caches were removed, T36), 288 MB of memory in use with everything
running (Node-RED 128 MB, ChirpStack 43 MB). Each component's download and installed size is in
`provision/components.yaml` now, from apt's history and a fresh npm install on the FIBER: Node-RED
115 and 490 MB, ChirpStack 17 and 68 MB, Mosquitto 1 and 2 MB (they add up to what was measured
in all). The time in FIBER Setup's texts stays until its own first start is timed. An apply with
nothing to do takes about 3 minutes. Waits also for D25: FIBER Setup's suggested
password, `fiber`, is too short for ChirpStack.
Earlier (2026-11-18): everything that needs neither the first release nor a FIBER is done, in FIBER
Setup's commit `83d372a`, which is on this computer only: its catalog names the installer built here
from `0257447`, marked as not published. Waits for the bench (T2), as D24 says: the platform runs on a
FIBER from a release built by hand first; then the first release is tagged, FIBER Setup's catalog
takes its version and sha256 and the copy of `components.yaml` from it, and FIBER Setup's commits are
pushed and its CI watched.
Checked: FIBER Setup's `cargo test` (100), clippy, `npm run check`, `npm test` (26), the build; the
views walked in the browser preview with one FIBER and with two, in English and in Czech (the picker,
nothing ticked, each stage of the first start, the links on the last step). Tried in the platform's
systemd container with Avahi, against the release built here (served inside the container):
`fiber-install.sh --start` puts the service in place; a failed download is tried again by the service
after 30 s and then goes through; the installer is checked by its sha256, takes the seed and deletes it,
and brings `mosquitto` up; the script's record goes `failed` (with `error=download`), `install`, then
away, while the platform's goes `running` and `done`; a stand-in installer that exits 10 leaves
`state=restart`, and the next start (another boot id) marks it done; with a clock 3 years behind and no
NTP, the clock is moved forward to the date of the HTTP answer. FIBER Setup's query, built alone for
arm64 Linux, read both records from a second container. Seen on the way: while Avahi registers a
changed record, it answers nothing for a moment, so the app keeps the last state it saw and shows an
end only when it reads it twice. Not tried: cloud-init itself, the Windows and Linux builds, anything
on a FIBER. The two changes here (`557941f`, `0257447`) are pushed, CI green (44248217878,
44249793562).

In `../fiber-setup`. The one place where a user makes the choice.

- [x] `Settings.install_fiber` becomes a list of components, plus the two settings the platform
      has a choice for (`mqtt.lan`, `hardware.minipcie`): `firstboot.rs` (the struct, its checks,
      what it writes), `types.ts`, `wizard.svelte.ts`, `api.ts`, the command line's `FlashArgs`,
      the *Software* card in `Settings.svelte`, and every view that looks at the old switch
      (`Image`, `Write`, `FirstStart`, `Done`). The texts in all five languages
- [x] The list comes from a copy of `components.yaml` at the pinned release: what a component
      requires is ticked with it, and its download size is shown once it is measured. The
      titles and the sentences are translated in FIBER Setup, by component id; a test fails on a
      component without them
- [x] `personalise()` writes `fiber-seed.yaml` beside `fiber-install.sh`: the components, the
      settings, and the user name and password for the services (D16). The file joins the list
      of old files that are cleared from the boot partition. A test here reads a seed with every
      kind of character FIBER Setup escapes, written as FIBER Setup writes it
- [ ] `catalog.rs`: the installer by its version and sha256, as the system image is, in place
      of an address on a `main` branch. `fiber-install.sh` fetches it and checks it; the test
      that keeps the two in step stays. Built and tested; what it names waits for the first
      release, after the bench (D24): its version and sha256, and `image/components.yaml` copied
      from it. Then FIBER Setup's commits are pushed and its CI watched
- [x] The clock: take the date over plain HTTP before anything over HTTPS, and only ever move
      the clock forward
- [x] `fiber-install.service` tries again after a failure without waiting for a restart: written
      by cloud-init, started without blocking it, `Restart=on-failure`. Exit status 10 restarts the
      FIBER; 20 is done for the script, as the platform goes on by itself
- [x] cloud-init sets the final name, `fiber-<last 8 characters of the serial number>`, always
      with `manage_etc_hosts: true`; the line that edits `/etc/hosts` goes
- [x] Here: the Avahi record says nothing a restart makes untrue: no `restart` in it. The
      script publishes its own record (`_fiber-setup._tcp`: waiting for the network, the clock,
      the download, the installation, the restart) and removes it when it is done; after a
      restart it runs once more to say so
- [x] Here: the record is read again by Avahi after every write. Found in a container: Avahi
      0.8 reads a service file again only when its time has changed in whole seconds, so a
      record written twice within a second kept the first (`running` long after `done`). A
      new record now has a time at least one second after the old one's
- [x] *Follow the first start* reads the progress from the platform's Avahi record (a DNS-SD
      query sent straight to the FIBER's address, so nothing on the computer listens on port
      5353). Tried against Avahi in a container. The last step shows a link for every chosen
      component, and says in plain words that these services can be reached from the network
      with the password from the settings
- [x] The choice "set Wi-Fi later from your phone" stays hidden until T29 (`PHONE_SETUP` in
      `wizard.svelte.ts`)
- [x] With nothing ticked no script and no seed are written, as today
- [x] Its README: the FIBER software is this platform, with its components, the one password
      and what was tried. "About 10 minutes" stays until the bench shows the time (the last step)
- [ ] On a FIBER: every box ticked; the same with NTP blocked; with the Ethernet cable plugged
      in only later; with the power cut in the middle of the installation; with a RAK5146 card
      and with the LTE modem in the slot; and on a module with 1 GB of RAM, with the time, the
      download size and the memory in use written down. Each component's download and installed
      size go into `size` in `provision/components.yaml`, which is empty until then, and the time
      into FIBER Setup's texts in place of "about 10 minutes"

Watch out: today the script tests `https://github.com` before the clock is right. On a network
that blocks NTP, TLS then fails, the script says "no internet", and its clock fallback (also
HTTPS) is never reached. The password field is pre-filled with `fiber`, and from now on that
password also opens services on the network. ChirpStack's user is `admin` (T9), not the login's.

- Needs: T5 to T8; T2 and then the first release (D24). D11: the `base` role alone writes the
  `config.txt` block, so FIBER Setup does not.
- Done when: a FIBER written by the new FIBER Setup comes up with the chosen components, and the
  last step shows a link for each.

### T13. The interfaces, written first (M2)

State (2026-11-18): waiting for the maintainer to read the three drafts. Written:
`docs/mqtt-api.md`, the board's commands in `docs/cli.md`, `docs/firmware/south-link.md`. Settled
for the drafts: a request carries `op`, as TOWER's gateway API does, not `method` as this task first
said, so that one client shape fits both; the south link's `Desired` has a state for each of the 18
LED lines (in ticks of 20 ms), the agent can ask for a `Hello` at any time (`HelloRequest`), and
`Status` counts the frames the firmware dropped.

Nothing of the agent is written before its interfaces are on paper (D19).

- [x] `docs/mqtt-api.md`: the rules (the prefix `fiber/`, QoS 1, state retained, events not, a
      request on `cmd` as `{id, op, params}` answered on `rsp/{id}` as `{id, ok, data}` or
      `{id, ok, error}`, the `schema` number, the last will), then the topics `status`, `power`,
      `power/event`, `ports/{1..8}`, `firmware/{south,north}`, each with its payload and an
      example, and the ops `ports.power`, `leds.set`, `leds.auto`, `firmware.flash`
- [x] `docs/cli.md`: `fiber agent`, `status`, `power`, `ports [on|off|cycle N]`, `leds`,
      `firmware [status|flash|chip]`, `doctor`; what each prints, and its exit statuses
- [x] `docs/firmware/south-link.md`: the frame, the messages `Hello`, `Desired` and `Status`
      field by field, and the timing (a `Desired` every second, the fallback after 5 s)

- Done when: the maintainer has read and agreed the three documents.

### T14. `fiber-protocol` (M2)

State (2026-11-18): built on the draft of `docs/firmware/south-link.md`, and `just check` passes: 11
tests, and the builds for `thumbv6m` and `thumbv7em`. Next: the push and CI. It stays in progress
until T13's drafts are agreed: a change there is a change here. Settled: the schema tag is worked
out as TOWER's is (a structural hash of the postcard schema, names left out, folded to 16 bits), by
code of our own, so it does not move with the TOWER pin; the south link's tag is `0x9a90` now
(`Hello` got `vrefint_cal` for T17); `serde` and `postcard` without their standard library.

The one contract between the agent and the firmware, built by both from the same source.

- [x] The frame: COBS around `type(1) | tag(2) | seq(2) | payload | crc32(4)`, ended by a zero
      byte; the payload is postcard. A decoder that takes bytes as they come, and a check that
      looks at the CRC first, then the tag, then the type
- [x] The schema tag: worked out from the message types when the crate is built, one for each
      link, so that a firmware and an agent from different builds refuse each other. It is ours
      alone and does not move with the TOWER pin
- [x] The south link's messages: `Hello` (who it is, the CRC-32 of its own image, why it reset,
      a fault code, its option bytes), `Desired` (18 LED states and the port mask), `Status`
      (what is applied, the three raw ADC counts, the time since start), `HelloRequest`
- [x] `no_std`, nothing allocated, no growing collections in a message
- [x] Tests: every message there and back; a damaged frame, a wrong tag and a frame with bytes
      left over are refused; a few frames kept as bytes; the crate builds for `thumbv6m` and
      `thumbv7em` in `just check`

- Needs: T13.
- Done when: the tests pass and both microcontroller targets build.

### T15. `fiber-flasher` (M2)

State (2026-11-19): on a FIBER: `fiber firmware chip` reads both chips (`0x453` and `0x497`) and
refuses while the agent holds the port; the agent wrote and verified the south bridge three times
(about 6 s each); a flash stopped hard at page 22 of 64 (the agent killed) left the chip in its
bootloader, and the next start flashed it again by itself. Left: the same with the power cut,
which also takes the south bridge's power, and needs someone at the bench.
Earlier (2026-11-18): `crates/flasher` is built and its 11 tests pass against the simulated chip: the image written and started; another chip, readout
protection and a bad image refused before anything is erased; three attempts with BOOT0 held
longer; a write cut short by a power cut leaves the chip waiting in its bootloader, and the next
write puts it right; a reset after a first write lands in the bootloader again (why it leaves
through `Go`); a byte that reads back wrong stops it before the vector table; the port lock.
Settled: the crate is the engine, behind two traits (the serial port and the two lines); the
adapters to the real serial port and GPIO lines come with the agent (T17), which owns them.

Writing a microcontroller's firmware from the CM4, in a way that cannot leave it dead.

- [x] The chips: product id `0x453` (STM32C031, 16 pages of 2 KB) and `0x497` (STM32WL55, 128
      pages of 2 KB). Any other id is refused before anything is erased
- [x] The reset lines behind a trait: NRST is pulled low or let go, never driven high; BOOT0 is
      driven. Entering the bootloader is tried a few times, holding BOOT0 longer each time
- [x] The serial port: 115200 baud with even parity for the bootloader, and back to no parity
      for the firmware
- [x] The commands: Get, Get ID, Read Memory, Write Memory (at most 256 bytes, in multiples of
      8), Extended Erase by a list of pages, Go. No mass erase
- [x] The image is checked before anything is erased: its size, where its stack starts, where
      its reset vector points
- [x] The block with the vector table is written last, after everything else was read back and
      compared. A flash that is cut short then leaves a chip that waits in its bootloader, not
      one that starts half a program
- [x] It leaves through `Go`, not through a reset: after a first flash the chip still believes
      its memory is empty and a reset lands in the bootloader again
- [x] The option bytes are read and reported (`nBOOT_SEL`, `nSWBOOT0`, readout protection), and
      a chip with readout protection is refused. The flasher never writes them
- [x] Progress is reported to the caller, nothing is printed; an error type of its own
- [x] A simulated bootloader for the tests that models the two lines, the "empty" flag, and the
      refusals a real one gives
- [x] A lock, so that a command run by hand keeps off a serial port the agent is using
- [x] On a FIBER: both chips' identity is read; a test image is written to the south bridge and
      verified; a write stopped hard in the middle, then recovered (2026-11-19)
- [ ] On a FIBER: the same with the power cut in the middle

Watch out: a write of the STM32C0's option bytes that is cut short can leave the chip with
readout protection on and its bootloader out of reach, which only SWD can undo. That is why only
the south bridge firmware changes them, once (T16).

- Needs: T2 (the bootloaders answer).
- Done when: the three steps on a FIBER pass.

### T16. The south bridge firmware (M2)

State (2026-11-19): runs on a FIBER, flashed there by the agent: its `Hello` names its image, a
`Status` comes every second, the agent's `Desired` goes in (a port cycled and its LED set through
MQTT were answered). One fix from the bench: `VREFINT_CAL` is at 0x1FFF 756A, not 0x1FFF 7568
(`TS_CAL1`), so the first build's voltages were 1.59 times too low (T36). This unit's option
bytes have `nBOOT_SEL` = 0 already, so the one-time write did not run. The pins follow `Desired`,
read over SWD while it ran (the output registers of GPIOA, B and D): all ports on and every LED
green as the agent asked; port 3 off (PB7 low, its LED dark); port 1's LED red; with the agent
stopped, after 5 s, every LED dark but the power LED's yellow line, low for about 150 ms once a
second, and the ports as they were. Not seen by eye: that the LEDs light (low = lit, by the
schematic). It stays in progress with T13's drafts.
Earlier (2026-11-18): built on the draft of `docs/firmware/south-link.md`: `crates/south-core` (6 tests) and `firmware/south`, 16224 bytes of
flash (15.8 KB of the 24 KB), with fmt, clippy, rustdoc and the size in `just check-south` and a CI
job, green since `30fb908` (44202681510). Not run on a chip: not the LEDs, not the ports, not the
ADC, and above all not the write of the option bytes, which is for the bench with a debug probe at
hand. Settled: one tick counter for every LED; the start pattern is the power LED blinking green,
the fallback the power LED blinking yellow with the port LEDs dark; a panic is reported without its
place (3.2 KB); the ADC's longest sample time, for the dividers' resistance. The simulator for the
agent's tests is `fiber south-simulator`, on a pseudo-terminal (made with T17).

The status lights, the power of the 1-Wire ports and two voltages, in at most 24 KB.

- [x] `crates/south-core`: the LED engine (off, on, blinking, by colour, on a 50 Hz tick), what
      happens with a `Desired`, the fallback pattern after 5 s without one, the ports switched
      on one after another at start. Tested on the computer, and built as a simulator on a
      pseudo-terminal for the agent's tests
- [x] `firmware/south`: a workspace of its own for `thumbv6m-none-eabi`, with the linker and
      the profile from T3 (`docs/bench-checks.md`, check 5: `flip-link`, `opt-level = "z"`,
      fat LTO, one codegen unit, `panic = "abort"`)
- [x] The board: USART1 on PA9/PA10 at 115200; the 18 LED lines (low = lit) and the 8 port
      lines (high = on) as `docs/hardware.md` has them; the ADC on PA11, PA12 and the internal
      reference, as raw counts; the watchdog; the 32.768 kHz crystal
- [x] `Hello` at start, with the CRC-32 the firmware works out over its own image
- [x] A start pattern on the LEDs until the agent sends its first `Desired`
- [x] A panic stores a fault code and the address in memory that survives a reset, resets, and
      reports them in the next `Hello`
- [x] The option bytes: read at start; if `nBOOT_SEL` is 1, bit 24 is cleared, once (the value
      then is `0xDEFFE1AA` on a factory part). Nothing else in them is ever touched. The stale
      "empty" flag is cleared at start
- [x] `just` recipes for building and measuring; a CI job: build, clippy, rustdoc, and the size
      against the budget
- [x] `docs/firmware/south.md`

Watch out: no formatting code, no floats: the counts are turned into volts by the agent. The
chip has no EEPROM and keeps nothing: after a reset the agent's next `Desired` puts everything
back. It is fed from the CM4's 3.3 V and is off whenever the CM4 is. T3's skeleton took 15.0 KB;
a panic handler that reads the panic's location costs 3.2 KB of it (every source path in the
HAL), so a panic may have to record its fault code without a location; the HardFault's address
costs 82 bytes.

- Needs: T3, T14.
- Done when: it builds under 24 KB, and on a FIBER the lights and the ports follow `Desired`.

### T17. The agent's core (M2)

State (2026-11-19): on a FIBER: the agent flashed the south bridge, which had the earlier firmware,
on its own at its first start (M2's first line), and again after an update; a flash stopped hard
recovered by itself (the power cut proper is left, T15); stopping the agent gives the fallback
pattern on the south bridge's pins, read over SWD (M2's third line, T16); `fiber status`,
`power`, `ports`, `leds`,
`firmware status` and `firmware flash` work through MQTT. The voltages were too low until the
firmware's fix (T36), and a single reading was too noisy for the 0.05 V rule: 38 messages on
`power` in 2 minutes, so the agent averages 8 readings now (T36). Left: the power cut, the
mini-PCIe lines (a card). After the agent restarts, the ports are all on and the LEDs automatic
again, whatever a client had set: as the code has it, and now written in `docs/mqtt-api.md`. Open: with no cell the
battery reads 1.7 V and is shown as a voltage; perhaps `power` should say there is no battery.
Earlier (2026-11-18): everything that needs no board is written and tested (`just check` and
the converge test pass on the computer; commit `b8f038d`, CI run 44206746581 green). `fiber agent`
runs: the core's logic (`agent/src/agent/`), the loop around it (the south link every second, MQTT,
requests, port cycles), MQTT through rumqttc without TLS, `agent.toml` with defaults. The agent
carries the south bridge's image (`just build-fiber` builds it in) and flashes the chip through the
flasher's adapters to the real port and GPIO lines; tested against the loop with a fake bus, not
against a chip. The command line's clients are written: `status`, `power`, `ports`, `leds`,
`firmware status` and `firmware flash` through MQTT, `firmware chip` on the bootloader directly
(tested against the simulated one); `power --follow` is not (T19 builds it). The converge test runs
the agent through Mosquitto against `fiber south-simulator` on a pseudo-terminal (step 8d); the
service came with T18. Not done: the mini-PCIe lines (they wait for T2's levels); anything on a
FIBER. Settled: `Hello` carries `VREFINT_CAL`; a board without dividers is a setting (`[board]
dividers`); rumqttc's loop runs in a task of its own, and a publish while the broker is down is
dropped (every retained topic is sent again after the reconnect); the simulator says it runs the
image the program carries, so that the agent leaves it alone; without the south bridge's port the
agent goes on with MQTT (found by T18).

The one program that owns the board. This task gives it the south bridge and its first topics.

- [x] The crates: `tokio` on one thread, `rumqttc` without TLS, `tokio-serial`, `gpiocdev`
- [x] `agent/src/board.rs`: every pin, device path and I2C address of rev 2.1, from
      `docs/hardware.md`. A test that no GPIO is used twice (done with T7, for `fiber doctor`)
- [x] The shape: one task holds the state; MQTT is an adapter around it (the LCD and Bluetooth
      will be two more). The board keeps working while the broker is down
- [x] The south link: `Desired` every second, `Status` and `Hello` read; the counts turned into
      volts (battery: the pin times 2; input: the pin times 398 / 68); PoE or battery, with a
      margin so that it does not flap
- [x] Firmware: the south bridge's image is built into the program (the north bridge's comes
      with T26). A chip whose `Hello` names another image or another schema tag, or that says
      nothing, is flashed (T15). At most three tries for one image in an hour, remembered
      across restarts
- [ ] The mini-PCIe card's two lines (GPIO10 and 11), set for the card that
      `hardware.minipcie` names, with the levels T2 found
- [x] MQTT: the last will on `status`, every retained topic sent again after a reconnect,
      requests and answers, the `schema` number
- [x] `/etc/fiber/agent.toml`, with defaults that need no file
- [x] The command line as a client of MQTT: `status`, `power`, `ports`, `leds`, `firmware
      status`, `firmware flash` (through the agent). `firmware chip` talks to the bootloader
      itself and refuses while the agent holds the port; `doctor` only reads
- [x] The service: the user `fiber-agent`, read-only system directories, only the devices it needs,
      no capabilities, a watchdog. Written with T18, whose role installs it
- [x] Tests without a board: against the south bridge simulator, the simulated bootloader, and
      a broker inside the test (Mosquitto in the converge test, step 8d)

Watch out: the schematic has a variant of the board without the two voltage dividers. On such a
board the two voltages read nothing useful, and `power` has to say so.

- Needs: T14 to T16.
- Done when: the three lines under "M2" hold on a FIBER.

### T19. Thermometers (M3)

State (2026-11-19): three DS18B20 on ports 1, 2 and 5 of the bench's FIBER (the maintainer).
`fiber thermometers` lists each on its port (about 24 °C), and `thermometers/{id}` and `ports/{n}`
carry them. The k-th master is port k, for these three: switching one port off (`fiber ports
cycle`) takes away its probe alone, gone (an empty retained message) within 10 s, back 3 to 7 s
after the power returns. Left: what the port lights show (the maintainer looks), a shorted port,
eight ports with probes, the starter flow on a FIBER.
Earlier: on a FIBER the overlay gives eight 1-Wire masters and `fiber doctor` counts them.
Earlier (2026-11-18): written and tested on the computer: the docs (drafts, like T13's: the maintainer
has not read them), `agent/src/agent/thermometers.rs` (the trait, the kernel's reader on a thread of
its own, what counts as a reading), the topics, the port lights, `fiber thermometers [watch]`,
`fiber power --follow`, the starter flow, `fiber doctor` counting the masters as the agent does.
`just check` and the converge test pass (commit `fc863c3`, CI run 44211968012 green); the converge
test runs the service against a made-up 1-Wire tree (a reading, no reading, gone). What is left
needs a FIBER. The kernel's source was read for this (rpi-6.12.y and v6.12): nothing in sysfs ties a
1-Wire bus master to the DS2482 or to its channel, so the masters are taken by their order. The
schematic says port k is DS2482 channel k - 1, and no part reports a shorted port.

The first thing most users will plug in: a DS18B20 on one of the eight ports.

- [x] `docs/` first: what the kernel gives (`docs/hardware.md`, "The 1-Wire ports in Linux"),
      the topics and the port lights (`docs/mqtt-api.md`), `fiber thermometers` (`docs/cli.md`)
- [x] Thermometers behind a trait. The first implementation reads the kernel's 1-Wire tree. The
      bus masters are `/sys/devices/w1_bus_masterN`, with nothing that names their chip or
      channel; the DS2482 registers its eight one after another, channel 0 first, so: exactly
      eight masters, sorted by number, the k-th is port k (more or fewer is said, and nothing is
      read). The eight ports read side by side (each master has its own lock), the probes of a
      port one after another (a read blocks for about 750 ms); off the main task
- [x] What is not a reading: an empty read (every error in the kernel's driver reads empty); the
      85 °C a probe reports right after power-on; a probe that is gone (the kernel keeps it for
      10 missed searches, about 100 s)
- [x] The port lights: what green, red and off mean for a port, written in `docs/mqtt-api.md`
      first
- [x] `thermometers/{id}` (retained; an empty message when a probe is gone) and the probes of
      each port in `ports/{n}`; `fiber thermometers` and `fiber thermometers watch`
      (`fiber ports cycle` came with T17), and with the same loop `fiber power --follow`, which
      `docs/cli.md` has had since T13 and T17 did not build
- [x] Tests against a made-up 1-Wire tree under a root that the tests set (and in the converge
      test, through the service)
- [x] The Node-RED starter flow shows the thermometers
- [x] On a FIBER: a probe shows up on its port; the k-th master is port k, by switching a port off
      (ports 1, 2 and 5, 2026-11-19); a probe that loses its power is gone within 10 s and back
      within 7 s
- [ ] On a FIBER: the port lights; a shorted port; eight ports with probes; the starter flow

Watch out: if plugging in feels slow (the kernel searches a bus only every ten seconds), the
other implementation is a driver of our own for the DS2482 over `i2c-dev`, about 700 lines. The
trait is there so that this is a swap (D17). `therm_bulk_read` (all probes converted at once)
cannot be used: the driver puts it on one master only, the master of the first DS18B20 it saw.

- Needs: T7, T17.
- Done when: a DS18B20 shows up on `fiber/thermometers/...` and in the starter flow; plugging a
  probe in and out, and a shorted port, behave.

### T28. The Bluetooth profile, written first (M6)

State (2026-11-18): the draft of `docs/ble.md` is written; it waits for the maintainer to read
it, and then for the people who make HARDWARIO Manager. Nothing is built on it before they
agree. Settled for the draft: the requests and answers are the same JSON as on MQTT; a message
goes in pieces of one header byte each (first, last, a number modulo 64), at most 4 KB; `info`
can be read without pairing, so the app can list FIBERs; bonds are not kept.

A new, small profile (D3), agreed with the people who make the app before any code.

- [x] `docs/ble.md`: one service with three characteristics, `info` (read), `rpc_rx` (write)
      and `rpc_tx` (notify). They carry the agent's requests, and only these: `device.info`,
      `setup.state`, `wifi.scan`, `wifi.connect`, `net.status`
- [x] Who may talk: pairing with a passkey that the FIBER shows on its LCD; the two request
      characteristics only on a link paired that way; advertising and pairing open only after a
      button on the FIBER was pressed
- [x] What happens if phones pair unreliably: the same code used in a key exchange of our own,
      above Bluetooth's (the plan only; designed if the bench needs it)
- [ ] Agreed with the HARDWARIO Manager app

- Done when: the app's side has said yes to the document.

### T31. Examples (M7)

State (2026-11-19): on a FIBER, against the real agent: `request.sh` (a message, a port cycled, seen
on `ports/3`) and `request.py` (an LED set and back to auto) were answered `ok`; `thermometers.py`
and the Rust example connect and wait (no probe plugged in); both flows import into the FIBER's
Node-RED and connect, and stay there; the button flow beeped on a hold of *enter* (the maintainer
heard it). Not on the FIBER: a reading, the alarm (no probe). Left: those, and the TOWER flow (T26).
Later the same day the alarm flow, still in the bench FIBER's Node-RED, fired from a real probe
(port 5, warmed by hand to 30.75 °C): the maintainer saw "Port 5: ... / above 30 °C" and heard the
alarm, and found no way to take the message off the display for its 10 minutes (D26).
Earlier (2026-11-18): everything that runs without a FIBER is done and checked, commit `877ce39`, CI
run 44262065157 green (the converge test with the examples in it, on the arm64 runner): `just check`
(with `check-examples` and the example's lock file in `audit`) and the converge test pass. In step 10c, against the agent's service: `request.sh` and `request.py` put a message on
the display and get "the buzzer is not there" for a beep; `thermometers.py` and the Rust example
print the probe's reading; at 31.5 °C the alarm flow puts "Port 3: 31.5 °C / above 30 °C" on the
display and asks for the alarm, and the Rust example asks for it too and prints the answer; a hold of
*enter* (published as the agent would) makes the button flow ask for a beep. The Python examples use
paho-mqtt 2.1, Debian trixie's; the button flow beeps on a hold of *enter*, as a click opens the
display's menu. Left: the TOWER node's flow (T26) and a run on a FIBER (T2). Learned the hard way:
the converge test reads its script while it runs, so the script must not be edited during a run.

Small programs that use the agent's MQTT API ([`docs/mqtt-api.md`](docs/mqtt-api.md)), for a
user to copy.

- [x] `docs/examples.md` first: each example, what it shows, how to run it on a FIBER
- [x] The command line: `mosquitto_sub` and `mosquitto_pub`, and a request with its answer
      (`examples/shell/request.sh`)
- [x] Python, with Debian's `python3-paho-mqtt`: the thermometers as they change, and a request
      with its answer (`examples/python/`)
- [x] Rust, with `rumqttc` as the agent has it: a project of its own (`examples/rust/`, its own
      lock file), built, linted and audited by `just check`, built for the FIBER and run in the
      converge test
- [x] Node-RED flows to import: a thermometer above a limit puts a message on the display and
      sounds the buzzer; the *enter* button sounds a beep. Each with its own broker node, so it
      imports into any Node-RED. In the converge test they are deployed through Node-RED's API,
      and what they ask the agent for is checked on `fiber/cmd`
- [ ] Node-RED: what a TOWER node sends. Waits for T26
- [ ] On a FIBER: each example once (with T2)

- Needs: T17 to T21 (the topics they use); T26 for the TOWER node's flow.
- Done when: every example runs in CI, and once on a FIBER.

### T22. The radio spike (M5)

State (2026-11-19): what needs no TOWER Core Module is done, on the FIBER of T2; the rest waits for
a Core Module at the pinned revision. The north bridge's flash was read out first through its
bootloader (256 KB, sha256 `1690ce2d...`, in `north-backup.bin` in the home directory of `fiber`
on the unit and in `target/bench/` on the maintainer's computer); `sudo python3 north_flash.py
write north-backup.bin` puts the earlier firmware back. The spike (`hil/radio-spike/`) runs on
the north bridge now and only listens. It is plain registers, not Embassy: `embassy-stm32` wants
a section of shared memory to start on the dual-core WL55, and the spike needs nothing of the
HAL. Results in `docs/bench-checks.md`, check 4: the radio up without a device error, the noise
floor at -114 to -118 dBm, no sync word heard on any of TOWER's three channels (about 5 minutes
of listening, after the survey). Then a Core Module r1 on the maintainer's computer, running
TOWER's `radio_beacon` at the pinned revision as the node (a 16-byte frame every second on channel
0, `[seq:4 LE][0xA5 x 12]`), flashed with TOWER's own `just fw flash` (the `tower` repository is
only built, not changed). **The north bridge receives it**, and 56 captures settled the air format
(`docs/bench-checks.md`, check 4): the sync word goes on air byte-reversed (`15 47 62 DB`); the
whitening is PN9 from all ones, least significant bit first, over the length byte, the payload and
the CRC; the CRC is CRC-16/CCITT-FALSE (`0x1021`, from `0xFFFF`) over the length byte and the
payload. The captures are test vectors in `crates/tower-air`. Watch out: the beacon was built
without `core-module-r1`, so it has no USB console (the r1's console is the STM32's own USB); to
flash it again the module must be put into DFU by hand (hold BOOT, press RESET); it keeps
sending its beacon (it is unplugged since). A second Core Module, an r2 with its FTDI bridge,
runs `radio_beacon` as the sniffer (flashed over its UART bootloader, no buttons), and **it
receives the frames the north bridge builds and sends** (spike in its send mode): through its
CRC, SQI 32, AFC -4 to 0, at -88 dBm from +14 dBm, -91 from +10, about -105 from -3. So T22's
"Done when" holds: frames go both ways and the captures are in the repository. Left: the error
rates at four levels (attenuators), several Core Modules, `docs/firmware/radio.md`. The FIBER's
antenna was disconnected through every radio measurement (the maintainer); the distance was not
measured. Each level is to be measured again with it on. Pushed so far
(`a3ea469..2bdc6fd`), CI run 44516902698 green; the antenna's records with `00cedd0..9d0167b`, CI
run 44577573971 green.

TOWER nodes use a SPIRIT1 radio; the north bridge has another one. Whether the two understand
each other is the biggest unknown of the plan, so it is answered before anything is built on it
([`docs/bench-checks.md`](docs/bench-checks.md), check 4). About two days.

- [x] A small firmware for the STM32WL55's M4 core (the M0+ core is never started), written to
      the chip through its bootloader (`hil/north_flash.py`). Plain registers through the PAC,
      not Embassy (State says why); the radio's SPI and busy line polled, no interrupt
- [x] The radio up: the TCXO switched on (1.7 V, 1 ms to settle); the calibration for 863 to
      870 MHz; the RF switch on PC3, PC4 and PC5 to receive (all high). The core runs on MSI,
      not on the TCXO; transmit is not used yet
- [x] GFSK as TOWER sends it: 19 216.6 bit/s (the rate a SPIRIT1 really produces, not the round
      19 200), 19.8 kHz deviation, BT 1, sync word `0xDB624715`, a preamble detector of 16 bits,
      117, 156 and 187 kHz receive bandwidth. Set, and the radio receives; not proven against a
      node
- [x] Receive raw: fixed length, the radio's own whitening and CRC off. Capture the bytes from a
      Core Module that runs TOWER's `radio_beacon` at the pinned revision. Without a sync word
      (the radio takes the bytes after the preamble), as the sync word in TOWER's register order
      matched nothing: on air it is byte-reversed. The IRQ is polled, not taken
- [x] From the captures settle the whitening (a 9-bit sequence that starts with all ones; in
      which order its bits fall into a byte) and the CRC-16 (polynomial `0x1021`; its start
      value; what it covers). Keep the captures as test vectors: PN9 from all ones, least
      significant bit first, from the length byte to the CRC; CRC-16/CCITT-FALSE over the
      length byte and the payload (56 of 56 frames)
- [ ] Try the shorter way too: the radio's own variable length and whitening, with a correction
      in software. It loses the two CRC bytes; every frame still carries an 8-byte tag
- [x] Send frames built in software to the Core Module, and read the frequency offset it
      reports: received through its CRC, `AFC_CORR` -4 to 0 (one module, an r2, 2026-11-19)
- [ ] The same on several modules
- [ ] The error rate at -60, -90, -100 and -105 dBm, for 117, 156 and 187 kHz bandwidth
- [x] The noise floor, with the linear regulator and with the switching one, with and without
      the boosted receive gain: -114 to -118 dBm, the regulator makes no difference. With the
      antenna disconnected: the receiver's own floor; with it on, still to be measured
- [x] The two-way levels again with the FIBER's antenna on (2026-11-19: an external antenna on
      the north bridge, its tip 15 cm from the Core Module r2): +14 dBm arrives at -81 dBm, the
      Core Module at -73 dBm; with another 868 MHz antenna 8 to 9 dB more both ways (-71 to -74
      dBm, -64 to -68 dBm), so the antenna is in the path. The spike's
      settings match ST's code for the module (the RF switch, the switching regulator, now in the
      spike; the regulator measured the same as the linear one, both ways), and the board matches
      the module's datasheet. `docs/bench-checks.md`, check 4
- [x] A reference (2026-11-19): a second Core Module, 10 cm from the first, both about 20 cm from
      the antenna. The receiver is fine (the same frames 17 to 19 dB below a Core Module at half
      the distance); the RF switch is right, tried in all 8 states (only PC4 high with PC5 low
      sends); sending is 10 dB short of receiving on the same pair (95 against 85 dB).
      `docs/bench-checks.md`, check 4. The antenna-off runs earlier had no measured distance (an
      "about a metre" in the documents was a guess, taken out)
- [ ] The north bridge's transmit power with a calibrated receiver (a spectrum analyser or a
      power meter on its jack through an attenuator): whether the 10 dB are the PA's or the
      SPIRIT1's RSSI scale
- [ ] The receiver's sensitivity against a known signal (the earlier firmware measured a weak
      receiver)
- [x] The results into `docs/bench-checks.md` and `docs/firmware/radio.md` (2026-11-19)

Watch out: by the documents the radio's own whitening has the right polynomial and seed and the
wrong bit order, so it cannot be used as it is. That is read from documents and from a capture
of a related chip, not seen on a SPIRIT1. This radio has no automatic frequency correction, so
the receive filter has to be wide enough for both crystals' error.

- Needs: T2 (the bootloader answers, to write the test firmware); a TOWER Core Module.
- Done when: frames go both ways, and the captures are in the repository.

### T42. A press sends a client's message away (D26) (M4)

State (2026-11-19): written and tested on the computer, `just check` passes, commit `1fc485b`, CI run
44577573971 green. On the bench's FIBER as release `0.1.0-bench.10`: a test message ("Press any button / to
send this away") went with the maintainer's second press of *up*, at 17:57:18; the first, 2 s
before, met a dimmed display and only lit it, as D26 has it (`fiber-bench-watch` recorded both
clicks and `"message": null`). Left: an alarm stopped by a press on the FIBER, and whether the page
stayed (the maintainer looks). The alarm example's message could not be taken off the
display on the bench; D26 says how.

- [x] `docs/front-panel.md`, `docs/mqtt-api.md` (`display.message`, `display`, `buzzer.beep`),
      `docs/examples.md` first
- [x] A press while a client's message shows or a client's pattern sounds: the message goes,
      the pattern stops (`Pattern::Silence`, the agent's own), `display` says `"message": null`
      at once, and nothing else of that press acts (its click still sounds). On a dimmed display
      the press only lights it, as before, and stops a pattern that sounds; the message goes
      with the next press
- [x] Tests (the buzzer's silence, the agent's press); the alarm example's text
- [x] On the FIBER: a message sent away with a press (2026-11-19; again at 19:36:24 with the
      simulated alarm, one press of *enter*)
- [ ] On the FIBER: an alarm stopped by a press

- Done when: the tests pass, and on the FIBER a press takes a message away and stops the alarm.

### T45. An LED test at the south bridge's start (M2)

State (2026-11-19): the first form (all red 3 s, dark 1 s, all green 3 s) ran on the bench's FIBER
(release `0.1.0-bench.13`, image `0x6040a3a0`). Then, on the maintainer's word ("an animation, so
it is more interesting"), an animation in its place: from 1 s after the start, the ports light red
one after another (port 1 to 8, 160 ms apart), turn green in the same order, then amber, then go
dark from port 8 back to 1; a hold of 0.3 s between, about 6 s in all. Each LED shows red, green
and both, and the order of the ports shows. Written and tested, `just check` passes; the image
is 16248 bytes, CRC `0xfbcd5b36`; on the FIBER as release `0.1.0-bench.14`, written by the agent
at 19:27:53; commits `a836f50`, `3e60968`, CI run 44590725201 green. Shown again for the
maintainer at 19:46:23 (`fiber firmware flash south`, the third write of the hour) and at 19:49:39
(a 50 ms pulse on the south bridge's reset with `pinctrl`, which writes nothing; the agent saw
`"reset":"pin"`). Left: the maintainer says whether every port shows all three colours in order. The power LED keeps its own pattern. The test runs to its end also when a
`Desired` comes in the meantime (after a flash the agent is already there): 8 s at most.

- [x] The first form: docs, `crates/south-core`, tests, `just check`, written on the FIBER
- [x] The animation: the docs first, then `crates/south-core` and its tests, `just check`
- [x] On the FIBER: the agent writes the new image (2026-11-19, 19:27:53)
- [ ] On the FIBER: the maintainer sees the animation

- Done when: the tests pass, and on the FIBER the maintainer sees red, dark, green after a start.

### T25. The north bridge firmware and its link (M5)

State (2026-11-19): started, without hardware (the maintainer away from the bench), after T24 and
T47 (pushed, `f47d3bd`). The link document is written (`docs/firmware/north-link.md`, a draft
the maintainer has not read): the south link's frame with a tag of its own; every message from
the agent answered (`Answer`), sent again when no answer comes within 200 ms; the radio silent
until `Settings`; uplinks acknowledged only while there is room to pass them on. The messages
are in `crates/protocol` (`north`, its tag `0xa5bc` pinned by a test; a node's payload as a byte
string of at most 74). Seen on the way: adding the module moved the south bridge's image
(`0xfbcd5b36` to `0xb79f9cb6`, 16240 bytes) with no change of its own (the crate's symbol names
moved, and the linker's layout with them): the agent writes it once more at its next install.
Next: the firmware, `firmware/north`.

- [x] `docs/firmware/north-link.md` first: `Hello` (who it is, its image's CRC, the
      personalities it has and the one that is active, why it reset), `SetPersonality`, the
      settings (the gateway's address, the band, the channel), nodes added, removed and listed,
      the block of transmit counters, a message queued for a node or dropped, pairing opened and
      cancelled, what a node sent (its address, counter, signal strength, bytes), how a
      transmission ended, the radio's numbers. A schema tag of its own
- [ ] `firmware/north`: a workspace of its own for `thumbv7em-none-eabi`; the M4 core only; the
      clock from the TCXO; USART1 at 115200; the random number generator for pairing; the
      watchdog; the stale "empty" flag cleared at start
- [ ] It keeps no list of nodes: after every `Hello` the agent sends the settings, the counters
      and the nodes. It acknowledges a node only while it has room to pass the message on
- [ ] The one thing it keeps: the transmit-counter watermark, in two pages of flash, each value
      written with its complement
- [ ] The personality switch, with `TowerFsk` as the only one for now (D4)
- [ ] A CI job (build, clippy, rustdoc), and a check that its TOWER revision is the root
      `Cargo.toml`'s
- [ ] The north bridge as a simulator on a pseudo-terminal, for the agent's tests

- Needs: T14, T15, T24.
- Done when: the firmware runs on a FIBER, says `Hello`, and takes its settings from a test
  program.

## Tasks ahead

In the order of the plan. Every task has its steps already; when a task starts, they are read
again and brought up to date before the first one is ticked.

### M0. Foundations and bench checks

Its tasks, T1 to T3, are in progress or done.

### M1. Provisioning

Done when: a FIBER written with every box ticked reaches `state=ok` by itself, also on a network
that blocks NTP; a second `apply` changes nothing; no service ever answers with a default
password; update and rollback work.

Its tasks, T4 to T12, are in progress or done.

### M2. The agent's core and the south bridge

Done when: a unit with the earlier south bridge firmware is flashed by the agent on its own; a
power cut in the middle of flashing recovers; stopping the agent shows the fallback LED pattern.

Its tasks, T13 to T18, are in progress or done.

### M3. Thermometers

Its one task, T19, is in progress.

### M4. The front panel

Done when: the LCD shows the name and the address after the start, and the buttons move through
the pages. Held on a FIBER (2026-11-19); its tasks, T20 and T21, are done.

### M5. The north bridge

Done when: two TOWER Core Modules at the pinned revision pair with the FIBER, their events arrive
on MQTT and a `set` reaches them; at least 99 % of frames are acknowledged at the first try;
replays are refused after a restart of either side; 24 hours pass without a fault.

#### T26. The agent's radio service

- [ ] `docs/mqtt-api.md` first: `radio/status` (the personality, the band, the channel, the
      pinned TOWER revision and its schema tag), `radio/pairing`, `radio/nodes/{addr}` with
      `state/{prop}`, `event/{prop}` and `set/{prop}`, and the methods `radio.config`,
      `radio.node_add`, `radio.node_remove`, `radio.node_rename`, `radio.pairing_open`,
      `radio.pairing_cancel`. Every node says which stack it belongs to, so that LoRa later adds
      nodes and not a second tree
- [ ] The nodes in `/var/lib/fiber/radio/`: the gateway's address, and for each node its
      address, key, name, kind and the last counter seen. Written so that a power cut cannot
      tear it
- [ ] The agent's own transmit-counter watermark; the counter starts from the higher of this and
      the north bridge's own, so that a FIBER restored from a copy of its disk cannot use an
      encryption nonce twice (D8)
- [ ] Node messages read with `tower-protocol` (its schema tag checked first) and turned into
      topics by its property tables
- [ ] Pairing: opened from the command line, from MQTT or from the menu, for 60 s; the six-digit
      code on the LCD and in `radio/pairing`
- [ ] The north bridge's image built into the program, and flashed by the agent as the south
      bridge's is (T17)
- [ ] `fiber radio status | config | nodes | pair | watch`
- [ ] Tests against the north bridge simulator

- Needs: T17, T25, T47; T21 for the code on the LCD.
- Done when: the lines under "M5" hold with one Core Module.

#### T27. Keeping the two radio stacks together

TOWER promises nothing about its radio between two revisions, and its nodes cannot be updated
over the air. So a FIBER works with TOWER nodes built from the pinned revision, and this task
makes moving the pin a procedure and not an adventure.

- [ ] `docs/firmware/radio-compat.md`: moving the pin, step by step (AGENTS.md, "The TOWER
      pin"), with the drift report: what changed in TOWER's `radio/frame.rs` and `radio/net/`
      between the two revisions
- [ ] The bench run in `hil/`: two Core Modules flashed from the pinned revision; pairing; what
      they send arrives; a `set` reaches them; the share of frames acknowledged at the first
      try; a replay after a restart of each side
- [ ] The 24-hour run, with what it found
- [ ] The procedure used once, for real, on the next move of the pin

- Needs: T26.
- Done when: the bench run and the 24 hours pass, and their results are written down.

### M6. Bluetooth

Done when: a phone with the updated HARDWARIO Manager puts a FIBER on Wi-Fi, and a wrong passkey
is refused.

#### T29. Wi-Fi from the phone

- [ ] The GATT service, the advertisement and the pairing agent through BlueZ, with `zbus`
- [ ] Wi-Fi through NetworkManager: scan, connect, the state; a polkit rule that lets the user
      `fiber` do that
- [ ] The `agent` role: Bluetooth unblocked, and `bluetoothd` started the way the service needs
- [ ] The LCD: the passkey page, and "Set up from the phone" in the menu
- [ ] FIBER Setup shows its choice "set Wi-Fi later from your phone" again (T12)
- [ ] Tests with D-Bus made up; on the bench with a phone

- Needs: T21, T28.
- Done when: the two lines under "M6" hold.

### M7. Documentation and polish

Done when: a new user gets from the box to a thermometer in a Node-RED flow with the
documentation alone.

#### T30. The FIBER pages of docs.hardwario.com

In `../docs/fiber`. The pages there describe a setup by hand that no longer works.

- [ ] The installation page, around FIBER Setup
- [ ] New pages: the components, the `fiber` command, the MQTT topics, pairing a TOWER node, the
      front panel
- [ ] What the old pages get wrong, put right: the Node-RED installer that is gone, the default
      password, the name of the FIBER, the `config.txt` lines

- Needs: a FIBER Setup that installs the platform is out (T12, D24): the site publishes what is
  pushed to it, and until then its readers' FIBER Setup installs the earlier software.

#### T32. A ready-made image

- [ ] An image built in CI from the roles' `install` step, with every component in it and none
      turned on
- [ ] The first start then only does the roles' `bootstrap` step, and needs no internet
- [ ] FIBER Setup offers it in place of the stock image (its catalog already expects one)

- Needs: T11.

#### T33. Retire the earlier repositories

- [ ] `hardwario/fiber-agent` and `hardwario/fiber-platform-legacy`, as D12 says

- Needs: T12. D12: left as they are until a FIBER Setup with the new installer is out, then
  archived.

### Not planned

Later, perhaps: US 915 with frequency hopping; the STICKER LoRa personality (D4); a component for
the LTE modem; the accelerometer; dimmed LEDs; InfluxDB and Grafana (D13); the remote shell and
bulk transfer of the TOWER radio.

Not at all: what the earlier agent did for another product (its web portal, alarms, beacons,
clusters, signed commands); Yocto images; firmware updates over the air for radio nodes; FIBER
revision 1.3 (D9); TLS or a broker inside the agent (D19); a full-screen terminal interface.

## Tasks done

### T1. Foundations (M0)

Done 2026-11-17, commit `b440fe2`. Checked: `just check` passes on the maintainer's computer,
and the first CI run on GitHub is green (run 44175588049). Not checked, because it needs a
board: nothing here has run on a FIBER (T2).

- [x] The rules: `AGENTS.md`, this file, `LICENSE`, the pinned toolchain (1.99.1, the one the
      pinned TOWER crates need), `rustfmt.toml`, the workspace lints
- [x] The host workspace and the TOWER pin (`Cargo.toml`, revision `cb3b25c`): `tower-net` and
      `tower-radio` build for the computer and for `thumbv7em-none-eabi` through
      `fiber-tower-air`; `tower-protocol` and `tower-hmi` build into the `fiber` program and its
      tests
- [x] `fiber version` (what FIBER Setup's first-start script records) and `fiber version --all`
      (the pinned TOWER revision, read from `Cargo.lock`, and the radio schema tag)
- [x] Tests that hold the pin: the nonce layout, a frame's time on air, the LCD's frame type
- [x] `just check`: fmt, clippy, tests, the `thumbv7em` build, rustdoc, the bench scripts'
      self-test, no long dashes, `cargo audit`
- [x] CI (`.github/workflows/ci.yml`) and Dependabot, every action pinned by its commit
- [x] `docs/`: the hardware map, the architecture, the provisioning design, the bench checks
- [x] `hil/bootloader_probe.py`: the first bench check as one command; its protocol code tested
      against a simulated bootloader, not yet run on a FIBER
- [x] InfluxDB, Grafana and sinqtt taken out of the plan and the documents (D13)
- [x] This file in its five sections, with the whole plan as numbered tasks
- [x] The writing style taken from FIBER Setup; `README.md` and `AGENTS.md` written to it
- [x] The whole plan carried into this file: every task with its steps, what to watch out for,
      what it needs and when it is done; the plan's decisions that were not written down yet
      (D14 to D19), and one new question (D20)
- [x] The first CI run on GitHub is green

### T3. Does the south bridge firmware fit? (M0)

Done 2026-11-17, commit `586569c`. Checked: the skeleton builds at the release profile and
takes 15348 bytes of flash (15.0 KB) of the 24 KB budget; `cargo bloat` and the disassembly say
what weighs most; `just check` passes. Not checked, because it needs a board: the skeleton was
never flashed (T16 runs the real firmware on a FIBER).

The STM32C031C6 has 32 KB of flash. A skeleton with everything the real firmware will link
answers whether Rust with Embassy fits, before T16 builds on it
([`docs/bench-checks.md`](docs/bench-checks.md), check 5). Needs no board. The estimate was
14 to 20 KB; the measure is 15.0 KB.

- [x] A throw-away crate for `thumbv6m-none-eabi`: `embassy-stm32` 0.6.0 with `stm32c031c6`,
      `memory-x`, `time-driver-any`, `unstable-pac`; `embassy-executor` 0.10.0; `embassy-time`
      0.5.1 with `tick-hz-1_000`; the `flip-link` linker (the versions TOWER's firmware uses)
- [x] In it: USART1 on PA9/PA10 at 115200; a COBS frame with a CRC-32 and a postcard message
      each way; the ADC on PA11, PA12 and the internal reference; 18 LED outputs and 8 port
      outputs; a 50 Hz tick; the watchdog
- [x] The release profile: `opt-level = "z"`, fat LTO, one codegen unit, `panic = "abort"`
- [x] Measure it with `cargo size`, and see which parts weigh most
- [x] Write the result into `docs/bench-checks.md`, with what to leave out if it is close

Watch out: what makes such a firmware big is formatting code (`{:?}`, `unwrap` on a `Result`),
floats, 64-bit division at run time, and `defmt` left in a release build.

- Done when: the size is written down. The budget is 24 KB.

### T4. The platform file and its commands (M1)

Done 2026-11-17, commits `2fe8334` and `bf69fda`. Checked: the CI run of `bf69fda`
(44182504508) is green: 41 unit tests and 11 tests that run the program against a temporary
root; the static arm64 build (`file`: "statically linked"); and every command run in a bare
Debian arm64 container with no broker, no hardware and no settings (job `container`). Not
checked, because it needs a board: nothing of it has run on a FIBER. Settled on the way:
`serde-saphyr` reads the YAML; `components.yaml` is compiled into the program; `enable` and
`disable` change only the lines under `components:`; `--root` (hidden) is for the tests;
`status` reads `/var/lib/fiber/platform/results.json`, which T6 writes. `just check-container`
passes on the maintainer's computer too (2026-11-17).

The file that holds the choice, and the first commands that read and change it. No Ansible yet.

- [x] `provision/components.yaml` (`schema: 1`): for each component its title, what it brings,
      its roles, what it requires and suggests, its services and its ports. The components:
      `base`; `mosquitto`; `agent` (requires `base`, `mosquitto`); `node-red` (requires
      `mosquitto`); `chirpstack` (requires `mosquitto`). Its sizes have a place in the file but
      stay empty until they are measured on a FIBER (T12)
- [x] The program reads `components.yaml` compiled into it; the archive carries the same file
      for FIBER Setup (T5, T12)
- [x] `/etc/fiber/platform.yaml` (`schema: 1`): `components` as a map of `true` (bring it up),
      `false` (stop and disable it, keep its data) or absent (leave it alone), and `settings`
      (`mqtt.lan`, `lorawan.region`, `hardware.minipcie`), each with a default. Reading it gives
      plain messages with the line: an unknown component is named, a wrong value says what is
      allowed. No file, or a file with nothing in it, means nothing is chosen
- [x] Requirements are worked out in the `fiber` program, from `components.yaml`: a component
      that is on brings up what it requires; one that is on while something it requires is
      `false` is an error that names both
- [x] One module for the paths (`/etc/fiber`, `/var/lib/fiber`, `/run/fiber`, `/opt/fiber`),
      with a root directory in front of them for the tests (`--root`, hidden)
- [x] The program finds its release from its own path (the directory above its `bin/`, when it
      has a `provision/`), so it never runs another release's roles
- [x] `fiber platform list`: every component, on, off or left alone, and what it requires
- [x] `fiber platform status`: the release, whether an apply is waiting, and the last result of
      each component, from `/var/lib/fiber/platform/results.json` (written by T6). Exit status
      0 when every component that is on or off has been brought there with `ok` and no apply is
      waiting, 3 when not
- [x] `fiber platform enable <component>...` and `disable <component>...` change the file:
      `enable` turns on what is required too; `disable` is refused while a component that is on
      needs it, with its name. Only the lines under `components:` change; the rest, comments
      too, stays. The new text is read back before it is written, and written by a rename. A
      new file starts with a line that says so, and with the settings at their defaults
- [x] The static build for the FIBER (`aarch64-unknown-linux-musl`), built in CI
- [x] `docs/cli.md`: the `fiber platform` commands and their exit statuses
- [x] Tests: reading, checking and resolving the file; the line editor; every command against a
      temporary root
- [x] The static build runs every command in a bare Debian arm64 container (CI, run
      44182158761, job `container`)

- Done when: the commands run in a bare container, with no broker, no hardware and no settings.

### T5. The release archive and the installer (M1)

Done 2026-11-17, commit `4430086`. Checked: the CI run of `4430086` (44186517138) is green: the
four installer tests in a container like Raspberry Pi OS, on the arm64 runner (an install piped
into `bash` with a seed; a second run that changes nothing; a hard stop while unpacking, then a
clean run; a wrong checksum refused; also an unstamped installer and a user who is not root are
refused); the archive built twice is the same; `shellcheck`. Not checked: the installer's apply
step (it needs `fiber platform apply`; now a step of T6), and nothing has run on a FIBER.

How the platform gets onto a FIBER, and how two releases live side by side (D2).

- [x] `fiber platform seed <file>`: the seed's components and settings become
      `/etc/fiber/platform.yaml` (only when there is none), its login goes to
      `/etc/fiber/secrets/login.json` (root only, set once); checked like `platform.yaml`
- [x] The archive `fiber-platform-<version>-linux-arm64.tar.gz`, built the same byte for byte
      every time (sorted, fixed dates, owned by root): `bin/fiber`, `provision/` (what exists of
      it), and `manifest.json` (version, commit, the sha256 of every file, the Ansible version it
      works with). `provision/release.py`, `just release`
- [x] `provision/install.sh`, stamped at release time with its version and the archive's sha256.
      It checks that it runs as root on an arm64 Raspberry Pi OS trixie; downloads the archive
      and checks the sha256; unpacks into `/opt/fiber/releases/<version>.partial`, checks every
      file against the manifest, syncs, renames, and writes `.complete` last
- [x] The switch: `current` moves by renaming a temporary link, `previous` points at the release
      before, `/usr/local/bin/fiber` points into `current`. Two releases are kept
- [x] Ansible: `apt-get install --no-install-recommends ansible-core python3-debian`, with a
      lock timeout of ten minutes and `dpkg --configure -a` first
- [x] The seed: `/boot/firmware/fiber-seed.yaml` goes through `fiber platform seed`, and the copy
      on the boot partition is overwritten and then deleted. Without a seed, `--components
      a,b,c` turns those components on
- [x] It then starts the apply (T6), unless `FIBER_NO_APPLY=1`. `FIBER_NO_REBOOT=1` is
      honoured. Exit status 0: done; 10: done, restart needed; 20: partly done, the platform
      tries again by itself; anything else: failed. Written; tested in T6, which makes the apply
- [x] Settings for the tests: where the release comes from (`FIBER_RELEASE_URL`), a root
      directory to install into (`FIBER_ROOT`)
- [x] `shellcheck` on every shell script in `just check`
- [x] Tests in a container like Raspberry Pi OS (Debian trixie arm64 with what the image has):
      install from a local server; run it again (nothing changes); stop it hard in the middle of
      unpacking and run it again; a wrong checksum is refused. `just check-install`, and a CI job
- [x] `docs/provisioning.md`: the seed, the installer's settings and exit statuses; `docs/cli.md`:
      `fiber platform seed`

Watch out: `ansible-core` recommends the `ansible` bundle, 205 MB, which apt installs unless
told not to. The stock image has no `python3-debian`, which the module for package sources
needs. The checksum in a script fetched with `curl` proves the download is whole, not who made
it: the documentation has to say so. A script piped into `bash` is read as it runs: a command in
it that reads the standard input eats the rest of the script.

- Needs: T4.
- Done when: the four container tests pass.

### T6. `fiber platform apply` (M1)

Done 2026-11-17, commit `9f9dc19`. Checked: the CI run of `9f9dc19` (44190793281) is green: the
converge test in a container with systemd on the arm64 runner (install and apply through the
installer; a second apply with nothing changed; an apply without network; a role failing on
purpose, with the others ending ok and the service trying again until it ends ok; the installer
ending with 20 and with 10; Mosquitto turned off), the role lint, the installer tests, and the
unit tests of the plan, the skips and Ansible's output. Settled on the way: one `ansible-playbook`
run per step, the result being its exit status; the `mosquitto` role's first part as the
example role; `python3-apt` installed with Ansible; the lint in a Debian container. Not checked:
the installer's exit 10 without `FIBER_NO_REBOOT=1` (it restarts the machine), and nothing has
run on a FIBER.

The one way the system is brought to what the file says, whoever asks for it.

- [x] `fiber platform apply --foreground` does the work: it reads the file, turns off what is
      `false` (in the reverse order of `components.yaml`), then brings up what is on (in its
      order), each with one `ansible-playbook` run on the playbook of the program's own release,
      with what it worked out as variables (the component, its roles, on or off, the settings).
      A component whose requirement failed in this run is skipped. One apply at a time
- [x] `provision/ansible/ansible.cfg`: facts gathered only when asked for, one fork, temporary
      files under `/run`, the collections path pointed at an empty directory in the release, and
      a lock timeout of ten minutes as the default of every `apt` task
- [x] `site.yml`: a first task checks the Ansible version and what the program handed over, then
      the component's roles run, their `main` (install, then bootstrap) or their `disable`
- [x] The `common` role: the directories, `/etc/fiber/secrets` (root only); the rule that a
      password is set once and never set again, written down
- [x] The shape every role has, shown by the `mosquitto` role's first part and written into
      `docs/provisioning.md`: `install` (packages, files, enabling; it must work in a chroot, so
      that T32 can build an image from it), `bootstrap` (what needs the running service: tokens,
      passwords) and `disable`; a drop-in file where the service takes one, never an edit of the
      package's own settings; every version pin in one file. The other roles fail with "not
      written yet" until their tasks
- [x] `fiber-platform-apply.service`: a oneshot that runs only while
      `/var/lib/fiber/platform/pending` exists, tries again after a failure with growing pauses,
      and sets `LC_ALL=C.UTF-8` and `HOME=/root`. The program writes it from its release
- [x] `fiber platform apply` writes `pending`, starts the unit and follows it: one line per
      component, and at the end whether a restart is needed. Exit status 0, 10 or 20 as in T5
- [x] `/run/fiber/platform/status.json` (state, step n of m, the result of each component,
      restart needed) and `/var/log/fiber/platform.log`; both are copied to the boot partition
      at the end
- [x] `/var/lib/fiber/platform/results.json`, written as each component finishes, in the form
      `fiber platform status` already reads (`agent/src/platform/results.rs`,
      `docs/provisioning.md`): what it was brought to, `ok`, `failed` or `skipped`, when, why
- [x] An Avahi service file (`_fiber._tcp`) whose TXT records carry the state, the step, the
      component and the release, so that FIBER Setup can follow the first start without a login
- [x] `fiber platform enable` and `disable` end with the line that says to run
      `fiber platform apply`
- [x] The installer's last step, without `FIBER_NO_APPLY` (in the converge test): exit status 0,
      10 with `FIBER_NO_REBOOT=1`, and 20. Without `FIBER_NO_REBOOT=1` it restarts the machine,
      which the test does not do
- [x] The converge test (`provision/tests/`): an arm64 Debian trixie container with systemd. It
      applies twice (the second run reports no change), applies with the network taken away
      (passes when nothing needs installing), and applies with a role made to fail (the others
      end `ok`, the unit tries again)
- [x] `yamllint` and `ansible-lint` with its `only-builtins` rule (`just lint-ansible`, in a
      Debian container) and in CI

Watch out: Ansible refuses to run without a UTF-8 locale, which neither systemd nor cloud-init
gives it. Roles have no `meta` dependencies: the program resolves what a component requires.
Every `command` task has `creates` or a real `changed_when`, or the second run is never clean.

- Needs: T5.
- Done when: the three converge tests pass in CI.

### T7. The `base` role and `fiber doctor` (M1)

Done 2026-11-19, on a FIBER: the role wrote the block into the
stock `config.txt`, byte for byte the fixture's expected file; after the restart `i2cdetect -y 10`
shows 18 and 51 in use, both serial ports, `/dev/spidev6.0`, `/dev/rtc0` and eight 1-Wire masters
are there, and `fiber doctor` finds nothing wrong. The stock file has `otg_mode=1` under `[cm4]`;
the kernel sets the clock from the RTC 19 s into the start, with no `fake-hwclock`; a kernel
package installed again leaves `fiber-ds2482.dtbo` in place. The questions under "Watch out": with
`dtparam=audio=on` left in the stock file the backlight works on PWM channel 1, and with
`camera_auto_detect=1` and `display_auto_detect=1` the board's three chips answer on their bus.
And the time held across a power cycle with NTP off: the kernel set it from the RTC 6.7 s into
the start, within a second of the computer's clock.
Earlier (2026-11-17): everything that needs no board is done, commit `81f5cf4`, and its CI run (44192840178) is green: the fixture test on a real FAT
filesystem in the converge test, `fiber doctor`, the board module and its check against the
block. Settled: the stock `config.txt` is `pi-gen`'s at the tag of the pinned image
(`provision/tests/fixtures/`), not one taken out of the image; the role uses `blockinfile`;
`release.py` compiles the overlay, so `just check` and CI need `dtc`; turning `base` off with no
`config.txt` is not an error.

What makes the board's hardware work under plain Raspberry Pi OS. After `base` alone the clock,
both serial links, the 1-Wire ports and the display's bus are there for anyone who knows Linux.

- [x] `provision/boot/config-fiber.txt`: the block as `docs/provisioning.md` has it, under
      `[cm4]`, comments on their own lines. Its GPIO numbers are checked against
      `agent/src/board.rs` by a test, so that the two places cannot drift apart
- [x] `provision/boot/fiber-ds2482.dts`: the DS2482 on the `i2c_csi_dsi` bus (`i2c0if` and
      `i2c0mux` enabled, then `ds2482@18` with `compatible = "maxim,ds2482"`, as the kernel's
      `i2c-rtc` overlay does it), compiled with `dtc -@` by `provision/release.py`, shipped in
      the archive
- [x] The role writes the block between `# BEGIN FIBER PLATFORM` and `# END FIBER PLATFORM`
      only when it differs, keeps a copy of the file as it was before the first change, and
      syncs. It copies the overlay to `/boot/firmware/overlays/` without owner or mode (the
      partition is FAT). It writes `/etc/modules-load.d/fiber.conf` with `i2c-dev`, and makes
      the groups. A changed `config.txt` or overlay ends the apply with "restart needed".
      Turning `base` off takes the block, the overlay and the module file away again
- [x] `settings.hardware.minipcie` (`none`, `lte`, `rak5146`): nothing drives GPIO10 and 11 at
      start; the agent later sets them for the card that is fitted
- [x] `fiber doctor`: fails on a line that breaks this board (`dtparam=spi=on`,
      `dtoverlay=w1-gpio`, `ctsrts`, a bare `dtoverlay=pwm`), and checks what should be there
      (the block, `/dev/ttyAMA3`, `/dev/ttyAMA4`, `/dev/spidev6.0`, `/dev/rtc0`, 0x18 and 0x51
      taken by their drivers and 0x19 answering on bus 10, eight 1-Wire masters). Each finding
      says what to do. Tested against a temporary root
- [x] The fixture test, in the converge test: the stock `config.txt` becomes the expected file,
      byte for byte; a second run changes nothing; a file edited by hand gets the block back
- [x] On a FIBER: the list under "Done when" (but the power cycle); that the stock file already
      has `otg_mode=1` under `[cm4]`; that the clock is set from the RTC at start without
      `fake-hwclock`; that a kernel update leaves `fiber-ds2482.dtbo` in place (2026-11-19)
- [x] On a FIBER: the time survives a power cycle, with no network (2026-11-19: NTP off, the
      power off for about a minute; the kernel set the clock from the RTC 6.7 s into the start,
      within a second of the computer's)

Watch out: `dtparam=spi=on` turns on SPI0, which takes GPIO7 to 11: the south bridge's reset,
its serial link and the card's lines. A bare `dtoverlay=pwm` takes GPIO18, the LCD's chip
select. The firmware applies `gpio=` lines a few seconds after power-on, not at once. The stock
file has `dtparam=audio=on`: whether the CM4's analogue audio takes PWM channel 1 from the
backlight is for the bench (T2), and so are `camera_auto_detect=1` and `display_auto_detect=1`,
which probe the same I2C bus as the board's own chips.

- Needs: T6; T2 for the part on a FIBER.
- Done when: the fixture test passes in CI; on a FIBER `i2cdetect -y 10` shows 18 and 51 in use,
  both serial ports and `/dev/spidev6.0` exist, eight 1-Wire masters exist, and the time
  survives a power cycle.

### T8. The `mosquitto` and `node-red` roles (M1)

Done 2026-11-19, on a FIBER: Mosquitto and Node-RED installed by the installer with the rest;
`mosquitto_sub` works on the FIBER; from another computer only port 1884 answers, and refuses
without the login and serves `fiber/#` with it (1883 and Redis's 6379 do not answer); Node-RED's
editor and its dashboard answer 401 without the login and 200 with it, and a wrong password gets
403. The dashboard has still not been looked at in a browser. Found there: npm kept 217 MB of
downloads in two caches; they go now (T36).
Earlier (2026-11-18): the dashboard (D20) is in (commit `71321b0`, CI run 44241706037 green).
Everything else that needs no FIBER is done, commit `14e348f`, and its CI run
(44194596621) is green: the network listener with the seed's login; a password the owner changes
stays; Node-RED behind the login, with the flow to start from; a second apply with nothing changed;
only ports 1880, 1883 on loopback and 1884 listening. Settled: the password file is
`/etc/mosquitto/fiber-passwd`, made once by `mosquitto_passwd` (the file is its own stamp);
Node-RED's `settings.js` is written once, when there is none, with the login's bcrypt hash made by
the `bcryptjs` Node-RED brings; the NodeSource key is vendored in the role (fingerprint checked
2026-11-17). Found on the way: the converge test's network test took the default route away with the
link; it puts it back now.

The broker everything else talks through, and the first thing a user sees working.

Mosquitto:

- [x] Debian's `mosquitto` and `mosquitto-clients`. The package's `mosquitto.conf` stays as it
      is; ours is one file, `/etc/mosquitto/conf.d/fiber.conf` (done in T6, as its example role)
- [x] `mqtt.lan: false`: no listener is written. The broker then serves port 1883 on loopback
      only, without a login, which is what is wanted (T6; the converge test checks it)
- [x] `mqtt.lan: true`: `per_listener_settings true` first; port 1883 on loopback without a
      login; port 1884 for the network with a login, its password file owned by `mosquitto` and
      readable by nobody else. The password is made with `mosquitto_passwd`, once, behind a
      stamp file (its output differs every time)
- [x] Turning it off: the service stopped and disabled, the settings and data kept (T6)

Node-RED:

- [x] Node.js 24 from NodeSource (source, key and its fingerprint in `docs/provisioning.md`),
      with an apt preference that keeps `nodejs` on NodeSource
- [x] `npm install -g --omit=dev node-red@<pinned>`, run only when that version is not there
- [x] A system user `nodered` in the groups for the serial ports, GPIO, I2C and SPI; its
      directory `/var/lib/node-red`; our own `nodered.service`, with Node's memory capped at
      256 MB
- [x] The login (`adminAuth`, a bcrypt hash made with the bcrypt that Node-RED brings) written
      before the first start
- [x] The starter flow, copied only when there is none: a broker node on `localhost:1883` and a
      debug node on `fiber/#`
- [x] The dashboard nodes (D20): `@flowfuse/node-red-dashboard`, pinned, and a page with the
      thermometers in the starter flow. Version 1.34.1, behind the same login as the editor
      (`httpNodeAuth`); the converge test checks the version, the page in the flow, and the
      login. Not looked at in a browser yet
- [x] In CI: after a converge only the expected ports listen; the editor asks for the login; a
      second apply changes nothing

Watch out: the installer script the old FIBER documentation names is gone (its address returns
404), and its successor installs again on every run. Debian's own Node.js 20 is too old for
Node-RED 5. What npm pulls in below Node-RED is not pinned, and the install takes minutes on
eMMC.

- Needs: T6.
- Done when: `mosquitto_sub -t '#'` works on the FIBER; with the LAN listener only port 1884
  answers from outside and asks for the password; Node-RED's editor asks for the login.

### T10. Update, rollback, uninstall (M1)

Done 2026-11-18, commits `a054cde` and `256b9e4`. Checked: the CI run of `256b9e4` (44200429368) is
green with the converge test: the footprint accounts for every new path of a converge with every
component; an update to a test release, a rollback and an update to the current release; after
an uninstall only what packages own, and data, is left. The weekly canary ran it against the live
sources too (44200466387). Settled on the way: `update` fetches with `curl`; releases are found
where GitHub serves their files, `FIBER_RELEASES_URL` for the tests; a test release comes from
`release.py --version`; the footprint check knows what packages' scripts and services make. Found
on the way: Debian trixie mounts a new `/tmp` while systemd starts; the installer test copied the
update test's releases too, which it no longer does. Not run on a FIBER yet.

What a package manager would do for us, done by hand and tested.

- [x] `provision/footprint.yaml`: every file the platform writes outside `/opt/fiber`,
      `/etc/fiber` and `/var/lib/fiber`, with the component that writes it, data marked as data
- [x] The footprint test, in the converge test: the filesystem before the install and after a
      converge with every component; anything new that neither a Debian package owns nor the
      footprint lists fails the test
- [x] `fiber platform update [--to version]`: finds the release, fetches its `install.sh`,
      checks it against the release's checksums and runs it. There is one installer
- [x] `fiber platform rollback`: `current` goes back to `previous`, then an apply. It brings
      back our program, roles and units; it does not put third-party packages or data back, and
      says so
- [x] `fiber platform uninstall`: every component off, each role's `disable` run, the footprint
      and `/opt/fiber` removed; the commands that would purge the third-party packages and their
      data are printed, not run. It asks for `--yes`
- [x] `docs/cli.md` and `docs/provisioning.md`
- [x] In the converge test: an update to a test release and back; after an uninstall only files
      that a Debian package owns, and data, are left

- Needs: T6.
- Done when: an update to a test release and back works in the container; after an uninstall
  only files that a Debian package owns, and data, are left.

### T11. Releases and the weekly canary (M1)

Done 2026-11-18, commits `a303713` and the earlier ones of T11. Checked: a dry run of `release.yml`
passed (44200461354); the canary ran once and passed (44200466387); the publish and report jobs pass
`actionlint` and CI (44241706037); immutable releases are on. Not run yet: the publish job (nothing
is tagged) and the report job (the canary has not failed). The first tag will run the publishing for
the first time; run the workflow by hand on `main` before it.

- [x] `.github/workflows/release.yml`: on a tag `vX.Y.Z` that matches the version, on a commit
      of `main` with green CI. It builds the static `fiber`, the archive, the stamped
      `install.sh`, `SHA256SUMS` and a provenance attestation, tries the archive in a container,
      and publishes a release that cannot be changed afterwards (everything is uploaded to a
      draft first). The `publish` job (D22) checks the files against `SHA256SUMS`, attests them
      with `actions/attest`, and publishes; not run yet, as nothing is tagged
- [x] The repository's *immutable releases* setting turned on (2026-11-18, with `gh api`; read
      back as enabled)
- [x] The same workflow run by hand builds everything and publishes nothing
- [x] `.github/workflows/canary.yml`, weekly: converge every component against the live package
      sources, open an issue when one breaks (one job with `issues: write`, D22), and warn when
      a key we carry is about to expire. The `report` job has not run yet: it runs only when a
      Monday's run fails
- [x] `docs/release.md`: the steps of a release

Watch out: a tag cannot be moved, so a release whose build fails uses up its version. Run the
workflow by hand first after any change to the build.

### T18. The `agent` role (M2)

Done 2026-11-19, on a FIBER: the installer's seed turned the agent on, the role installed it, and
after the restart `fiber/status` came up online, with the south bridge flashed by the agent. An
update to the next release restarted it on the new program (and it flashed the new firmware).
Earlier (2026-11-18): everything that needs no board is done and checked (commit `94dea9b`, CI run
44210054395 green): `just check`, and the converge test with the service in it (the agent runs as
`fiber-agent` without capabilities, holds the port, feeds a watchdog of 3 s, is restarted on the new
program by an update and a rollback, and goes offline when turned off). On the way the converge test
found that the agent served nothing on MQTT without the south bridge's port; fixed
(`run_without_link`). The port lock is taken read-only when its file is there (another user's file
in the sticky `/run/lock`). Waits for a FIBER (T2) for the last step.

- [x] `docs/provisioning.md` first: the agent role (the user, its groups, the unit, the settings
      file), and the unit's lines in `docs/architecture.md`
- [x] `fiber-agent.service`: the user `fiber-agent`; `Type=notify` with a watchdog that the agent's
      loop feeds (also while the port is missing and while it flashes); the system read-only
      but for `/var/lib/fiber/agent` and `/run/lock`; only the device classes it needs; no
      capabilities
- [x] The role: the user and its groups, the unit, `/etc/fiber/agent.toml` written once with the
      defaults in comments, enabled and started; restarted when the program changed (an update
      or a rollback)
- [x] Turning it off stops and disables the service; both microcontrollers keep their firmware
- [x] In the converge test: the service against the south bridge simulator; it runs as
      `fiber-agent`, holds the port (`fiber firmware chip` refuses), its watchdog does not fire, an
      update restarts it on the new program, and turning it off takes `fiber/status` offline
- [x] On a FIBER: `fiber platform enable agent` brings up `fiber/status` (2026-11-19, through the
      installer's seed)

- Needs: T6, T17.
- Done when: `fiber platform enable agent` on a FIBER brings up `fiber/status`.

### T20. LCD, backlight, buttons, buzzer (M4)

Done 2026-11-19, on a FIBER, seen and heard by the maintainer: the LCD shows the agent's pages,
upright; the backlight lights; the buttons work; the buzzer sounds (a hold of *enter* through the
example flow, then `fiber buzzer beep`); `fiber display` answers. The panel is mounted upside
down, so the driver turns the picture by 180 degrees, as the earlier software did; the first look
("the lcd module view is flipped") said so and was misread, and for one build (`20bdbb2`) the turn
was out. The driver (`agent/src/agent/lcd.rs`): ST7920 packets over `/dev/spidev6.0` in mode 3 at
400 kHz, reset on GPIO16, only the rows that changed, set up again after an error. The backlight
dims after 60 s without a press, as it should. Not run: `fiber buttons watch` itself (the flow
read the same `buttons/event`). Commits `f68b657`, `20bdbb2`, `7cb8657`, pushed with the rest of the bench's
commits; CI run 44516902698 green.
Earlier (2026-11-18): all but the display driver is written and tested on the computer: the docs
(drafts), the buttons (`agent/src/agent/buttons.rs`, `tower-hmi`'s recognizer on a thread that waits
for the lines' edges), the buzzer's patterns, the backlight through sysfs PWM with its dimming, the
`display` topic, a message drawn into a `Frame<16, 64>` and compared with stored pictures, `fiber
display`, `fiber buzzer beep`, `fiber buttons watch` (commit `a59cf8c`, CI run 44213917356 green).
The unit no longer has `ProtectKernelTunables=yes`: the backlight is written through
`/sys/class/pwm`. `just check` and the converge test pass (it asks for a message, the backlight and
a beep through the service). The display driver waits for T2 (the LCD's bus); nothing of this has
run on a FIBER.

- [x] `docs/mqtt-api.md` and `docs/cli.md` first: `buttons/event`, `display`, and the methods
      `buzzer.beep`, `display.message`, `display.backlight`
- [x] The display driver over `/dev/spidev6.0`, with what T2 found (the mode, the speed, the
      bytes before a command and before data), reset on GPIO16. Seen on the LCD (2026-11-19);
      the picture turned by 180 degrees, as the panel is mounted upside down
- [x] The picture is a `tower_hmi::Frame<16, 64>`; only the rows that changed are sent
- [x] The backlight on PWM channel 1, with a brightness and a time after which it dims; the unit
      leaves `/sys` writable as its permissions have it
- [x] The buttons on GPIO23, 24 and 25 through `gpiocdev`, with the pull-ups and the kernel's
      debounce; click, hold and repeat from `tower-hmi`
- [x] The buzzer on GPIO17 (low = sound): a beep, and a few patterns
- [x] Tests: the frames a drawing produces, compared with stored ones; made-up button presses

- Needs: T2 (the LCD's bus), T17.
- Done when: `fiber display`, `fiber buzzer beep` and `fiber buttons watch` work on a FIBER.

### T21. Pages and the menu (M4)

Done 2026-11-19, on a FIBER: the pages show on the LCD and the buttons move through them (the
maintainer). The stored pictures match in the tests.
Earlier (2026-11-18): written and tested on the computer: `docs/front-panel.md` (a draft),
`agent/src/agent/screen.rs` (the four pages, the menu with About, the apply's progress and a
client's message over the pages, the addresses from `/proc/net/fib_trie`), the agent drawing the
picture every second and on every gesture, the first press after the backlight dimmed only lighting
it; a stored picture of every page. The picture goes nowhere until T20's driver. The menu's
"Restart" is written as D23 says: a confirmation, then `/run/fiber-agent/restart`, which
`fiber-restart.path` hands to `fiber-restart.service`; the converge test checks the chain, with the
restart made a mark, and passes (commit `b9c9bbb`, CI run 44241706037 green). `just check` and the
converge test pass on the computer (commit `ab9671d`). Its CI run 44215459393 failed in the converge
test's cleanup, not in the agent: `rm -r /run/fiber-test` raced with `socat` taking its links away
as it ended. The next run, on `5d3fbad` with the same code (44215720443), is green. The test now
waits for `socat` first; pushed with T34's fixes (`2a04dff`, CI run 44219055224 green).

- [x] `docs/front-panel.md` first: the pages, what the buttons do, the menu, what covers the pages
- [x] The pages: the name, the addresses and the time; the components and their state; the power
      source and the voltages; the thermometers. The radio's page comes with T26
- [x] The menu, with `tower-hmi`'s menus: about (the versions); restart (D23:
      `fiber-restart.path` and `.service` in the agent role). Pairing comes with T26, setup from
      the phone with T29
- [x] While an apply runs, its progress (from `/run/fiber/platform/status.json`); a client's
      message over the pages
- [x] A stored picture of every page, compared in the tests
- [x] On a FIBER, with T20's driver: the buttons move through the pages (2026-11-19)

- Needs: T20.
- Done when: the pictures match, and on a FIBER the buttons move through the pages.

### T34. Review of the night's code (M2 to M4)

Done 2026-11-18, commit `2a04dff`. Checked: `just check`, the roles' lint and the converge test
on the computer, and its CI run (44219055224) is green. An independent review read the code of
T17 to T21 (`54cb19e..ab9671d`) for defects that show at run time. It found nine; all are fixed:

- [x] **The agent role took over the owner's login.** FIBER Setup suggests the login user name
      `fiber`, and the role's user task would have given that account `nologin` and another
      home, and the uninstall would have said `userdel fiber`. The agent's user is
      `fiber-agent` now; the converge test makes a login `fiber` first and checks it is left
      alone
- [x] **The command line reused request ids** within one command (`fiber leds red`,
      `fiber ports off 1 2`), and the agent dropped the repeats: each request has its own id
      now; the converge test switches two ports with one command
- [x] **A long broker outage could leave the agent without its subscription to `cmd`:**
      rumqttc queued the publishes made while the broker was down, and a full queue kept the
      subscription from going out. Publishes while disconnected are dropped now, as the comments
      said they were
- [x] **A probe unplugged while the agent was not running stayed retained:** the agent reads
      the retained `thermometers/+` back and clears those no round finds
- [x] **A failed backlight write was not tried again** (udev may give the PWM's files to the
      group `gpio` only after the export): it is tried every second
- [x] **A probe moved to a lower port was reported on its old one** for about 100 s: it counts
      on the port where it reads
- [x] **The apply's status named the next component with the number of the finished steps:**
      it names the step that runs, from 1, as its doc comment says; a `running` status older
      than 2 hours (an apply that was killed) no longer covers the display
- [x] **Requests wait while a chip is flashed,** which `docs/mqtt-api.md` did not say: it says
      so now
- [x] **`firmware/north` was listed but is never sent:** the docs say it comes with T26
- [x] The converge test and CI pass with the fixes

### T35. The Rust toolchain off Dependabot's list (D21) (M0)

Done 2026-11-18, commit `1998121`. Checked: its CI run (44239942412) is green, and Dependabot ran
twice under the new settings, green (44239953437, 44239953577), with no run for the toolchain.
Pull request #1 is closed with a comment that names D21, and its branch deleted.

The toolchain moves with the TOWER pin and nothing else (D21).

- [x] `.github/dependabot.yml`: the `rust-toolchain` entry taken out, and its comment says why
- [x] Pull request #1 (1.99.1 to 1.100.0) closed, with a comment that names D21

### T36. What the first bench run found (M1, M2)

Done 2026-11-19: all five fixes, found on the FIBER of T2 and tried there with releases built by
hand (`0.1.0-bench.2` and `.3`). Checked: `just check`, the roles' lint and the converge test on the
computer, the FIBER, and CI run 44516902698, green. Commit `cee666c`, with the averaging and the
apply's hand-over in it; pushed with the rest of the bench's commits (`a3ea469..2bdc6fd`).

- [x] **The south bridge read the wrong calibration word** for its voltages: `VREFINT_CAL` is at
      0x1FFF 756A; 0x1FFF 7568 is the temperature sensor's `TS_CAL1` (1662 and 1048 on the
      chip, read through its bootloader). Every voltage was 1.59 times too low (T16). On the
      FIBER since: input 13.6 V, the source `poe`
- [x] **ChirpStack refuses a password shorter than 8 characters,** and FIBER Setup suggests
      `fiber`: the component failed at every try with "non-zero return code". The role now fails
      before it sets anything, saying what is wrong and what to do; the converge test tries it
      with `fiber` first (T9). What FIBER Setup should do: D25
- [x] **npm kept everything it downloaded:** 127 MB in `/root/.npm` and 90 MB in
      `/var/lib/node-red/.npm`, on 8 GB of eMMC. Each npm call gets a cache of its own, removed
      after it; the converge test checks that none is left (T8)
- [x] **After an update, a waiting apply ran the old release's roles:** the apply service's
      process, started by release 0.1.0 and waiting to try again, served the request of the
      next release's installer with its own roles. It now hands the process over to the current
      release's program (`exec`) before it runs again for a new request; a test for which release
      takes over (T6, T10)
- [x] **A single voltage reading is too noisy for the 0.05 V rule:** `power` went out 38 times
      in 2 minutes. The agent averages the last 8 readings (T17)
- [x] The converge test passes with them, on the computer (2026-11-19)
- [x] CI passes with them (44516902698)
- [x] On the FIBER (`0.1.0-bench.3`): 3 messages on `power` in 2 minutes (the retained one, then
      once a minute), where there were 38; the apply with the new npm tasks changes nothing. That
      a fresh install leaves no npm cache is for the converge test to show

- Done when: the converge test and CI pass, and the step before is seen on the FIBER.

### T37. No battery: `power` says `null` (M2)

Done 2026-11-19, commit `4c1a735`, with T38; CI run 44532823673 green. On the FIBER, with no cell fitted, the battery read
1.7 V and was shown as a voltage; a Li-Ion cell that works never reads under 2.5 V, so below 2 V
there is none. On the FIBER since (`0.1.0-bench.7`): `"battery": null` on `power`, "no battery"
from `fiber power`. On the way: whether `power` moved compared the input exactly once the battery
was `null`, which would have published every 0.01 V; it compares each voltage by 0.05 V now.

- [x] The agent: a battery below 2 V is `null` on `power` (the topic already allows it), the
      source not changed by it (`Volts::cell`)
- [x] `fiber power`, `fiber status` and the power page say "no battery"
- [x] `docs/mqtt-api.md` and `docs/cli.md` say so; tests

- Done when: the tests pass, and on the FIBER `power` has `"battery": null`.

### T38. The front panel: faster to answer, and drawn with symbols (M4)

Done 2026-11-19, commit `4c1a735`, with T37 (CI run 44532823673 green): the maintainer looked at the pages on the FIBER
(`0.1.0-bench.7`) and pressed the buttons, and said good. Measured first: a full redraw takes 91 ms
the agent's way (a write per 3-byte packet at 400 kHz), where the ST7920's datasheet allows no less
than about 83 ms (72 us an instruction, 1152 of them); a write per row took 79 ms and a whole
picture at 1 MHz 40 ms, both by packing packets closer than 72 us, so they were not taken. The
slowness was mostly that a click is recognized on the release: the page turns on the press now.
`just check` passes.

- [x] *Up* and *down* turn the page on the press (and on the repeats of a hold), not on the
      click; *enter* stays on the click, to tell it from a hold. The first press after the
      backlight dimmed still only lights it, and nothing else of that press acts
- [x] The LCD driver sends only the 16-pixel words of a row that changed
- [x] The pages drawn again: icons of our own (`tower_hmi::Image`), page dots in the title bar,
      the clock and the readings in bigger fonts, a mark for each component's result, the
      battery's level, a progress bar for an apply; stored pictures for each, looked at as PNGs
      on the computer before they went on the FIBER
- [x] `docs/front-panel.md` as it is now
- [x] On the FIBER: the maintainer looks, and presses (2026-11-19: good)

- Done when: the pictures match, and on the FIBER the pages look right and turn as the button
  goes down.

### T39. The front panel shows local time (M4)

Done 2026-11-19, commit `4e513a2`, CI run 44538912753 green: on the FIBER (`0.1.0-bench.8`) the maintainer looked and
asked for it to be pushed. The FIBER page showed the time in UTC; FIBER Setup sets a time zone (Europe/Prague on the
bench), in `/etc/localtime`, which the agent's user can read. Read with `tz-rs` 0.7 (pure Rust, no
`unsafe`, no dependencies), which also follows the TZif file's closing rule
(`CET-1CEST,M3.5.0,M10.5.0/3`). `just check` passes. Seen on the way, not from this change: four
"unsound" advisories against `heapless` 0.9.3 (2026-11-18), through `cobs`, `postcard` and
TOWER's crates; allowed as warnings by the audit; 0.9.4 is out.

- [x] The agent reads the zone from `/etc/localtime` (TZif, with its rule for the years after its
      last transition), again when the link or its file changes; UTC when there is none
- [x] The FIBER page: the local time and the zone's abbreviation (`CET`, `CEST`); the day too
- [x] Tests (the FIBER's own Europe/Prague file: winter, summer, 2040 from the rule, a link to
      nothing), the stored picture, `docs/front-panel.md`
- [x] On the FIBER: the local time (the maintainer, 2026-11-19)

- Done when: the tests pass, and the FIBER shows its local time.

### T40. `heapless` 0.9.4: the advisories of 2026-11-18 (M0)

State (2026-11-19): started, on the maintainer's word. RustSec published four "unsound" advisories
against `heapless` 0.9.3 on 2026-11-18 (RUSTSEC-2026-0576, -0578, -0579, -0581: `IndexMap::truncate`,
`SortedLinkedList::zeroize`, `Deque` from `TryFrom`, `Deque::make_contiguous`). It comes in through
`cobs`, `postcard` and TOWER's crates; the audit lets such warnings pass, and 0.9.4 is out.
Done 2026-11-19, commit `0dec9af`, CI run 44541931261 green. The south bridge's image changed with it
(`0x66695049` to `0x71f9caeb`, still 16224 bytes): a FIBER that gets this agent writes it to its
south bridge. Not installed on the bench's FIBER, whose south bridge the CM4 cannot reach until
its power cycle (T2): install it there after that.

- [x] Every lock file that has `heapless` 0.9.3 moved to 0.9.4, and nothing else moved (the
      host workspace's and the south bridge's; the examples and the radio spike have none)
- [x] `cargo audit` names it no more; `just check` passes, with the south bridge's size
- [x] CI (44541931261)

- Done when: the audit is clean of these four, and CI passes.

### T23. The radio's command layer and the packet codec (M5)

Done 2026-11-19: the packet codec in `crates/tower-air` (`packet`), the radio's commands in a new
crate, `crates/subghz` (`fiber-subghz`, `no_std`, no dependencies, over a bus trait the firmware
fills with the SUBGHZSPI), the `Radio` trait for T24, and the spike on the two crates as the test
firmware, with T23's receive flow. Checked: `just check` (the codec against every capture, the
commands' bytes against the spike's, the thumbv7em builds), and on the bench frames both ways with
a Core Module r2 through the crates. Commits `082a451`, `91ef7db`, `c43abf4`; CI run
44550531048 green. Found on the way: the bit rate's word is 53 287, not the spike's 53 286.

- [x] The packet codec in `fiber-tower-air`, as pure functions tested against the captures of
      T22. To send: length, frame and CRC, whitened. To receive: the length byte as soon as it
      is in (how many bytes follow), then the rest, de-whitened and checked (`packet::encode`,
      `packet_len`, `decode`; every capture passes, and the encoder makes a captured packet byte
      for byte)
- [x] `crates/subghz`: the radio inside the STM32WL over a bus trait: standby, the regulator,
      the TCXO, the calibrations, packet type, frequency, GFSK modulation and packet settings,
      the sync word, the buffer, interrupts, signal strength of a packet and of the channel, the
      low-power PA (+14 dBm at most, the limit of the 25 mW band), receive and send; the RF
      switch as a trait of its own. Laid out so that LoRa is another packet type and not a
      rewrite (D4). Tests: the bytes of each command, against the spike's, which the bench
      proved. One difference found: the bit rate's word is 53 287 (32 x 32 MHz / 19 216.6,
      rounded); the spike sent 53 286, a slip of 0.002 % in its comment's arithmetic
- [x] The spike on the two crates: the receive flow (the sync word, the length byte two byte
      times later, the rest when it is in, read, receive again), and sending; frames both ways
      with the Core Module r2 (`radio_beacon` as node, then as sniffer, flashed over its FTDI):
      every frame of the node passed the codec (20 in 20 s, none missed, -75 to -77 dBm), and the
      sniffer took every frame sent (14 of 14: -100, -87 and -83 dBm from -3, +10 and +14 dBm),
      the FIBER's antenna still off
- [x] The `Radio` trait the gateway is written against: send a frame, receive one with its
      signal strength, set the channel (`fiber_tower_air::radio`, async; its first user is T24)
- [x] `docs/firmware/radio.md` and `docs/architecture.md` (the new crate) as it is then

Watch out: this radio gives a signal strength and nothing like the SPIRIT1's link quality
numbers, so our messages carry the strength alone.

- Needs: T22.
- Done when: the codec passes every capture, and the test firmware exchanges frames with a Core
  Module through it.

### T41. A click on every press of a button (M4)

Done 2026-11-19, commit `a3acab3`, CI run 44577573971 green. Checked: `just check`, and on the FIBER (bench release
`0.1.0-bench.9`, after the power cycle) the maintainer hears a click on each press.

- [x] A short click of the buzzer as a button goes down, the press that only lights the backlight
      too; never over a pattern that sounds (a press must not cut an alarm short): 25 ms,
      `Pattern::Click`, not one a client can ask for
- [x] Tests; `docs/front-panel.md`
- [x] On the FIBER: heard (the maintainer, 2026-11-19)

- Done when: the tests pass, and on the FIBER each press clicks.

### T43. The address with its network size on the first page (M4)

Done 2026-11-19, commit `d9d44a6`, CI run 44577573971 green. Checked: `just check`, and on the bench's FIBER (release
`0.1.0-bench.11`) the maintainer saw `10.0.0.108/24` on page 1. The size comes from the same
`/proc/net/fib_trie`: the longest `link UNICAST` route that holds the address.

- [x] `docs/front-panel.md` first
- [x] Each local address with the size of the network it is in; an address in no such network
      stays bare. `+N` only when it fits the line (19 characters after the icon)
- [x] Tests: the FIBER's own `fib_trie` as a vector (`agent/src/agent/fib-trie-2026-11-19.txt`),
      two networks of other sizes, the `+N` rule, the stored picture
- [x] On the FIBER: seen on the display (the maintainer)

- Done when: the tests pass, and the FIBER's first page shows `10.0.0.108/24`.

### T44. A press acts at once, also on a dimmed display (D27) (M4)

Done 2026-11-19, commit `0d543dd`, CI run 44577573971 green. Checked: `just check`, and on the bench's FIBER (release
`0.1.0-bench.12`): dimmed, the backlight's PWM reads 20 % (200 000 of 1 000 000 ns, 1 kHz); a
message lit it to 80 % at once, and one press of *enter* sent it away; the first press on a
dimmed display acts ("it works well", the maintainer).

- [x] `docs/front-panel.md`, `docs/mqtt-api.md` first
- [x] The backlight dims to 20 % (or stays at its brightness, when that is lower)
- [x] A press acts at once, dimmed or not, and lights the backlight; a client's message lights it
      as it arrives; a press sends a message away, dimmed or not
- [x] Tests: the agent's front panel and press tests, the display's duty
- [x] On the FIBER: a press on a dimmed display acts; a message lights the backlight

- Done when: the tests pass, and on the FIBER the first press on a dimmed display acts.

### T46. A client's message shows at once (M4)

Done 2026-11-19, commit `a62cbbd`, CI run 44590725201 green. Checked: `just check` (a test that fails without the fix), and
on the FIBER (release `0.1.0-bench.15`) the alarm's message comes with its beep (the maintainer,
19:41:49). The cause: the agent drew the display on its tick, once a second, and on a press, but
not after a request, so a message waited up to 1 s (and about 90 ms more to reach the LCD) while
the buzzer started at once.

- [x] The agent draws after every request, as after a press
- [x] A test: the picture shows the message as soon as the request is handled, between two ticks
      (it fails without the fix)
- [x] `just check`
- [x] On the FIBER: the alarm's message and its beep together

- Done when: the test passes, and on the FIBER the message comes with the beep.

### T24. The frame codec and the gateway's flow (M5)

Done 2026-11-19 (the frame codec `8ec3c7b`, `17ca877`; the flow `7338409`, CI run 44615462418
green).
Checked: `just check`, the 11 flow tests and the 32 frames of a real Core Module through the
gateway (each message taken in once, each copy answered). Not run on a FIBER: that needs the
north bridge's firmware (T25). Pairing over the air is T47. Settled on the way: `recv` receives
until its deadline and passes over what it does not take in (TOWER's takes one frame a call); a
`Clock` trait beside `Radio`, a receive with a deadline being a race of the two; the watermark
behind a `Watermark` trait (T25 keeps it in flash); the counter locked until the agent gives the
floor the firmware last reported (no other record is kept on the north bridge, so a lost
watermark looks like a first start); the agent adds each node with its key and last counter
(D8).

What TOWER's own gateway does inside its firmware, written again over our radio. Every decision
about security stays TOWER's code (D1).

- [x] The frame, as TOWER's `firmware/src/radio/frame.rs` has it: `fmt_type | flags | src(4) |
      dest(4) | counter(4) | [bulk_idx(3)] | payload | tag(8)`; the clear header is
      authenticated, the payload encrypted; the nonce from `tower-net`; AES-128-CCM from
      `tower-radio`, over an AES in software (`fiber_tower_air::frame`, `SoftAes` over `aes` 0.9;
      RFC 3610's packet vector 1 passes through it; the 74-byte MTU checked against
      `tower_protocol` at compile time)
- [x] The vectors: frames of `net_confirmed` from a real Core Module, captured, opened with its
      known key (32 frames, 2026-11-19, `crates/tower-air/tests/net-confirmed-2026-11-19.txt`):
      each opens, `msg NNN\0`, the counters one after another from 0xB801, each message sent 3
      times as the same frame (no acknowledgement); none opens under another key
- [x] `docs/firmware/radio.md`: the gateway's flow, before its code
- [x] The gateway's flow over `tower-net` (`fiber_tower_air::gateway`): only nodes it knows; the
      replay rule; confirmed delivery with an acknowledgement (the counter it answers, the signal
      strength, the "more waiting" flag); the transmit counter with its reserve, locked until the
      agent's floor; one waiting message for each sleeping node, sent when the node next speaks
- [x] The duty-cycle budget for EU 868 from `tower-radio`; the channel as a setting (the band is
      EU 868 only: US 915 needs hopping, not planned)
- [x] Messages to nodes (`Set`, `Time`, `Describe`) and from them (`Info`, `Report`, `Event`)
      pass through untouched: the agent reads them, the firmware does not
- [x] Tests on the computer (`crates/tower-air/tests/gateway.rs`, 11): the flow against a made-up
      radio and clock sharing one time (a lost acknowledgement, a replay, a node the agent keys
      again, unknown and forged frames, a waiting downlink heard and not heard, the watermark,
      the duty cycle), and the 32 captured `net_confirmed` frames through it

Watch out: the parts copied in spirit from TOWER (`radio/frame.rs`, `radio/net/mod.rs`,
`radio/net/pairing.rs`) are what the drift report of T27 watches. Left for later: frequency
hopping (US 915), the remote shell, bulk transfer.

- Needs: T23.
- Done when: the vectors pass, and the flow tests cover the three cases above.

### T47. Pairing over the air (M5)

Done 2026-11-19 (the commit that closes it). Checked: `just check`, and 7 tests against a
joining node made up from `tower_net::handshake` (as a TOWER node runs it): the whole handshake
(the same link key and code at both ends, the answers 20 ms after each frame), a wrong
confirmation, a lost nonce (a fresh key pair for the next try), who may join (a known node, a
reserved address, the gateway's own, another handshake version), the window, nothing answered
before the agent's floor, and the keyed join (answered, a replay refused, switched off, an
unknown node). Not run with a real Core Module: that is T26's check (M5: TOWER nodes pair with
the FIBER). In `crates/tower-air`, `gateway::pairing`: the agent opens a window
(`Gateway::open_window`), the firmware calls `Gateway::pair` in slices, and adds the node it
returns; the random numbers through an `Entropy` trait (the STM32WL's generator in T25); the
refusals for the agent to tell (`take_join_refusal`), as TOWER logs them. The test air moved to
`crates/tower-air/tests/common/`. `zeroize` is a direct dependency now (to wipe the window's key
pair slot, as TOWER does; already in the lock through `tower-net`); the spike's lock followed.

Split off from T24 (2026-11-19). TOWER's `firmware/src/radio/net/pairing.rs` written again over
the gateway of T24: the window the agent opens, the X25519 key agreement, the commitment round,
the six-digit code, and the keyed join of a known node. Every decision stays `tower-net`'s
(`pairing`, `handshake`).

- [x] Read `pairing.rs` and `tower-net`'s `pairing` and `handshake`; write the flow into
      `docs/firmware/radio.md` first ("Pairing a node")
- [x] The pairing host over the T24 gateway, and the keyed join inside `recv`; the random numbers
      from the STM32WL's generator through a trait
- [x] Tests: a node joins against a made-up radio; a wrong confirmation tag pairs nobody;
      pairing again (removed first, a new key, the lane started again)

- Needs: T24.
- Done when: the tests pass; a real Core Module pairing with a FIBER is T26's check (M5).

## Decisions ahead

- **D25. Which password does FIBER Setup let through, now that it opens services?** Found on the
  bench (2026-11-19, T36): ChirpStack refuses a password shorter than 8 characters, and FIBER
  Setup suggests `fiber` and checks only that the field is not empty. The same password opens
  Node-RED, its dashboard and MQTT on port 1884 from the network (D16), and AGENTS.md says no
  service may answer with a default password. Choices:
  1. **(Recommended)** When any component is ticked, FIBER Setup asks for at least 8 characters
     and suggests none: the password opens services on the network, so it should not be one
     everybody knows. With nothing ticked, as today.
  2. Only when ChirpStack is ticked, at least 8 characters; `fiber` stays the suggestion
     otherwise.
  3. As it is: the platform's role says what to do, and the person puts a longer password into
     `/etc/fiber/secrets/login.json`.
  The platform's side is done either way (the role's message). T12 waits for it.

## Decisions done

- **D1. How is TOWER reused?** Own it here (2026-11-17). The `tower` repository is not changed.
  This repository writes its own STM32WL55 radio firmware and its own radio interface on MQTT,
  and pins only TOWER's hardware-free crates. Tests keep the two radio stacks together (T27).
- **D2. How does the software get onto a FIBER?** As a checksummed release archive from GitHub
  Releases, unpacked under `/opt/fiber` (2026-11-17). No apt repository, no Debian packages of
  our own.
- **D3. The Bluetooth interface?** A new, smaller GATT profile; HARDWARIO Manager is changed to
  match (2026-11-17). Nothing of the earlier service `0xFB00` is kept.
- **D4. What does the radio do?** TOWER FSK first, STICKER LoRa later (2026-11-17). The north
  bridge firmware, its link and the MQTT topics are made for two personalities, one at a time.
- **D5. Ansible on the FIBER?** Yes (2026-11-17, with the plan), with one correction to the
  idea it came from: Ansible does not keep the state, it brings the system to a state that we
  keep, in `/etc/fiber/platform.yaml`, and it remembers nothing of what it installed.
  `ansible-core` from Debian, `ansible.builtin` modules only, one small role per component, run
  on the FIBER itself. Not chosen: cloud-init alone (it runs once), shell installers (what there
  was), Ansible run from the desktop app (it cannot run from Windows). If a role ever needs a
  module of its own, that component moves into the `fiber` program.
- **D6. How many programs?** One, `fiber`: the platform's commands, the agent and the command
  line (2026-11-17, with the plan). The archive always carries it; only the `agent` component
  starts the service.
- **D7. Which InfluxDB?** Version 2 (2026-11-17, with the plan). Set aside the same day by D13:
  no InfluxDB is installed for now.
- **D8. Where does the radio keep its nodes?** In the agent (2026-11-17, with the plan). The
  north bridge keeps only a transmit-counter watermark, so it may restart at any time, and
  TOWER's storage code, made for another kind of memory, is not needed.
- **D9. Which hardware?** FIBER-R2 revision 2.1 only (2026-11-17, with the plan). Revision 1.3
  has no microcontrollers; the board map is by revision, so it could be added.
- **D10. Anything kept from the earlier on-device software?** No (2026-11-17, with the plan).
  Not its MQTT topics, its serial protocols, its settings or its Bluetooth service. A FIBER moves
  over by being written again with FIBER Setup.
- **D11. Does FIBER Setup write the `config.txt` block too?** No: one writer, the `base` role
  (2026-11-18). During the first installation the GPIO lines sit at their power-on levels, and
  one restart follows it. Looked at again once the first installation has been watched on the
  bench.
- **D12. What happens to `hardwario/fiber-agent`?** It is left as it is, and archived once a
  FIBER Setup with the new installer is out (2026-11-18). An old FIBER Setup keeps installing
  what it was tested with (T33).
- **D13. InfluxDB, Grafana and sinqtt?** Left out for now (2026-11-17). The components are
  `base`, `agent`, `mosquitto`, `node-red` and `chirpstack`; nothing stores or charts history.
  What was learned, for the day they come back: InfluxDB 3 Core limits a query to 432 files
  (about 72 hours) and only its Enterprise edition compacts, so version 2 is the one that charts
  months; `sinqtt` writes through `/api/v2/write`, so it works with version 2 as it is; Grafana is
  347 MB to download and 1.33 GB installed, on a module with 8 GB of eMMC.
- **D14. Containers?** No (2026-11-17, with the plan). Every service is installed from its own
  package source, as a Raspberry Pi user would expect to find it. Containers are heavier, and
  they hide the system from someone who is learning embedded Linux.
- **D15. Whose rules does the repository follow?** TOWER's for the layout and the checks (the
  workspaces, `just check`, pinned actions, a pinned toolchain), FIBER Setup's for the writing,
  and this file in five sections (2026-11-17).
- **D16. How many passwords?** One (2026-11-17, with the plan). The user name and password
  typed into FIBER Setup also open Node-RED, ChirpStack and the MQTT listener for the network.
  They travel once, in the seed file on the boot partition, and are deleted from there. Each
  service gets them once; a password changed later stays changed.
- **D17. Who reads the thermometers?** The kernel's 1-Wire driver first (2026-11-17, with the
  plan): it worked on this board before, and the probes are then there for anyone, with or
  without the agent. A driver of our own is the fallback, behind the same trait (T19).
- **D18. Does FIBER Setup follow the newest release?** No, it names one (2026-11-17, with the
  plan): the installer's version and sha256 sit in its catalog, as the system image's do. What a
  new FIBER installs then changes only with a release of FIBER Setup.
- **D19. How is the agent reached?** Over MQTT, and only over MQTT (2026-11-17, with the plan).
  The command line is a client of the same topics, so anything it can do, a user's program can
  do. The agent needs the local broker; it has no broker of its own and no TLS.
- **D20. Does the Node-RED component bring the dashboard?** Yes (2026-11-18):
  `@flowfuse/node-red-dashboard`, pinned, and a page in the starter flow with the thermometers,
  so that the first minute with a FIBER shows something (T8).
- **D21. Who moves the Rust toolchain?** The TOWER pin, and not Dependabot (2026-11-18): the
  channel is taken off Dependabot's list, so that TOWER's crates always build here on the
  compiler TOWER uses, and its pull request #1 is closed.
- **D22. May the release and the canary workflows write?** Each through one job of its own
  (2026-11-18), as TOWER does it: the publishing job of `release.yml` gets `contents: write`,
  `id-token: write` and `attestations: write`, runs only on a tag that passed the gate, and
  uploads to a draft first; one job of `canary.yml` gets `issues: write`. `AGENTS.md` names the
  two as the only exceptions to "the workflows may only read" (T11).
- **D23. How does the menu restart the FIBER?** Through a path unit (2026-11-18):
  `fiber-restart.path` in the agent role watches a file that only the agent may write, and
  starts `fiber-restart.service`, which runs `systemctl reboot` as root. Nothing else is
  installed, and the agent keeps no right of its own (T21).
- **D24. When is the first release made, for FIBER Setup to pin?** After the bench (2026-11-18):
  no tag until T2 has run the platform on a FIBER from a release built by hand (`just release`,
  served to it). Until then FIBER Setup's work on T12 stays on this computer, as its catalog names
  an installer that is not published (T12).
- **D26. How does a person at the FIBER get rid of a client's message, and do alarms belong in
  the agent?** Alarms stay in the flows; a press takes a client's message off the display and
  stops a pattern that sounds, and that press does nothing else; `display` then says
  `"message": null` (the maintainer, 2026-11-19, after the example's alarm could not be sent
  away on the bench). The other choices were: the message only stepping aside, a parameter that
  keeps a message whatever is pressed, and alarms in the agent. Built in T42. Its rule for a
  dimmed display (the first press only lights it, the message goes with the next) was changed by
  D27.
- **D27. What does a press do while the backlight is dimmed?** What any press does: it acts at
  once, and lights the backlight on the way; a client's message lights the backlight as it
  arrives; any press sends a message away, dimmed or not (this changes D26's rule for a dimmed
  display). The backlight dims to 20 %, not to a fifth of its brightness (16 % of 80 %). (The
  maintainer, 2026-11-19: the press that only lit the backlight felt like nothing happening, on a
  display that stays readable when dimmed.) The other choices were: a wake-only press made
  audible, and only *enter* waking without acting. Built in T44.
