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
| M4 | The front panel | T20, T21 | The LCD shows the name and the address; the buttons move through the pages |
| M5 | The north bridge | T22 to T27 | TOWER nodes pair with the FIBER and their events arrive on MQTT |
| M6 | Bluetooth | T28, T29 | A phone puts a FIBER on Wi-Fi |
| M7 | Documentation and polish | T30 to T33 | A new user gets from the box to a thermometer in a flow with the documentation alone |

**The order.** M1 and M2 do not depend on each other and can go side by side. The three tasks
that answer the biggest unknowns come first: T2 (do the bootloaders answer), T3 (does the south
bridge firmware fit) and T22 (does the radio hear a TOWER node), which can start as soon as T2
is done, long before the rest of M5.

## Tasks in progress

### T7. The `base` role and `fiber doctor` (M1)

State (2026-11-17): waiting for a FIBER (T2) for its last step. Everything that needs no board is
done, commit `81f5cf4`, and its CI run (44192840178) is green: the fixture test on a real FAT
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
- [ ] On a FIBER: the list under "Done when"; that the stock file already has `otg_mode=1`
      under `[cm4]`; that the clock is set from the RTC at start without `fake-hwclock`; that a
      kernel update leaves `fiber-ds2482.dtbo` in place

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

State (2026-11-18): waiting for a FIBER (T2) and for D20. Everything that needs neither is done,
commit `14e348f`, and its CI run (44194596621) is green: the network listener with the seed's
login; a password the owner changes stays; Node-RED behind the login, with the flow to start
from; a second apply with nothing changed; only ports 1880, 1883 on loopback and 1884 listening.
Settled: the password file is `/etc/mosquitto/fiber-passwd`, made once by `mosquitto_passwd`
(the file is its own stamp); Node-RED's `settings.js` is written once, when there is none, with
the login's bcrypt hash made by the `bcryptjs` Node-RED brings; the NodeSource key is vendored
in the role (fingerprint checked 2026-11-17). Found on the way: the converge test's network test
took the default route away with the link; it puts it back now.

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
- [ ] The dashboard nodes, if D20 says so (waiting for D20)
- [x] In CI: after a converge only the expected ports listen; the editor asks for the login; a
      second apply changes nothing

Watch out: the installer script the old FIBER documentation names is gone (its address returns
404), and its successor installs again on every run. Debian's own Node.js 20 is too old for
Node-RED 5. What npm pulls in below Node-RED is not pinned, and the install takes minutes on
eMMC.

- Needs: T6.
- Done when: `mosquitto_sub -t '#'` works on the FIBER; with the LAN listener only port 1884
  answers from outside and asks for the password; Node-RED's editor asks for the login.

### T9. The `chirpstack` role (M1)

State (2026-11-18): waiting for a FIBER with a card (and T2). Everything else is done, commit
`26a78d0`, and its CI run (44197050069) is green: the API never answers to admin/admin during the
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

### T10. Update, rollback, uninstall (M1)

State (2026-11-18): built, and the converge test passes on the maintainer's computer with all of
it: the footprint accounts for every one of the 20575 new paths of a converge with every
component; an update to a test release, a rollback and an update to the current release work;
after an uninstall only what packages own, and data, is left. Next: the push and CI. Settled:
`update` fetches with `curl` (no HTTP client in the program); releases are found where GitHub
serves their files, `FIBER_RELEASES_URL` points elsewhere for the tests; a test release with
another version number comes from `release.py --version`; the footprint check knows what
packages' scripts and services make (their `rc?.d` links, alias links, their own data). Found on
the way: Debian trixie mounts a new `/tmp` while systemd starts, so the test keeps its snapshots
in `/root`; the test image has `/etc/udev/rules.d`, as Raspberry Pi OS has.

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

State (2026-11-18): the read-only parts are written: `release.yml` (the gate, the build, the
second build compared, the program and the installer tried in containers, the files kept as an
artifact; a run by hand is the dry run), `canary.yml` (every component converged against the
live sources, and the keys checked: `just check-keys`) and `docs/release.md`. Publishing a
release and opening an issue need write rights, which `AGENTS.md` forbids: waiting for D22. Next:
the push, then a dry run and a canary run, started by hand.

- [ ] `.github/workflows/release.yml`: on a tag `vX.Y.Z` that matches the version, on a commit
      of `main` with green CI. It builds the static `fiber`, the archive, the stamped
      `install.sh`, `SHA256SUMS` and a provenance attestation, tries the archive in a container,
      and publishes a release that cannot be changed afterwards (everything is uploaded to a
      draft first). Written without the publishing and the attestation (waiting for D22)
- [ ] The same workflow run by hand builds everything and publishes nothing
- [ ] `.github/workflows/canary.yml`, weekly: converge every component against the live package
      sources, open an issue when one breaks (waiting for D22), and warn when a key we carry is
      about to expire
- [x] `docs/release.md`: the steps of a release

Watch out: a tag cannot be moved, so a release whose build fails uses up its version. Run the
workflow by hand first after any change to the build.

- Needs: T5, T6.
- Done when: a dry run of a release passes, and the canary has run once.

### T13. The interfaces, written first (M2)

State (2026-11-18): waiting for the maintainer to read the three drafts. Written: `docs/mqtt-api.md`,
the board's commands in `docs/cli.md`, `docs/firmware/south-link.md`. Settled for the drafts: a
request carries `op`, as TOWER's gateway API does, not `method` as this task first said, so that
one client shape fits both; the south link's `Desired` has a state for each of the 18 LED lines
(in ticks of 20 ms), the agent can ask for a `Hello` at any time (`HelloRequest`), and `Status`
counts the frames the firmware dropped.

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

State (2026-11-18): built on the draft of `docs/firmware/south-link.md`, and `just check` passes:
11 tests, and the builds for `thumbv6m` and `thumbv7em`. Next: the push and CI. It stays in
progress until T13's drafts are agreed: a change there is a change here. Settled: the schema tag
is worked out as TOWER's is (a structural hash of the postcard schema, names left out, folded
to 16 bits), by code of our own, so it does not move with the TOWER pin; the south link's tag is
`0x90d6` now; `serde` and `postcard` without their standard library.

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

## Tasks ahead

In the order of the plan. Every task has its steps already; when a task starts, they are read
again and brought up to date before the first one is ticked.

### M0. Foundations and bench checks

#### T2. Bench checks on a FIBER

What only a real board answers. Flashing, recovery, the LCD and the ChirpStack role all rest on
these answers, and each takes minutes ([`docs/bench-checks.md`](docs/bench-checks.md), checks 1 to
3 and 6).

- [ ] Write a FIBER-R2 rev 2.1 with FIBER Setup, with the FIBER software turned off, and log in
      over SSH
- [ ] Add `dtoverlay=uart3` and `dtoverlay=uart4` to `/boot/firmware/config.txt`, restart, and
      see `/dev/ttyAMA3` and `/dev/ttyAMA4`
- [ ] Run `sudo python3 bootloader_probe.py` (from `hil/`). North bridge: ACK, product id
      `0x497`, `nSWBOOT0` = 1. South bridge: ACK, product id `0x453`
- [ ] Write down the south bridge's option bytes as a production unit has them. `nBOOT_SEL` = 1
      on a unit whose firmware runs means the CM4 cannot reach that chip's bootloader at all
- [ ] If a second unit is at hand, run the probe on it too: the question is what production
      units carry, not what one board does
- [ ] `i2cdetect -y 10` (after `dtparam=i2c_vc=on`): 0x18 (DS2482), 0x19 (LIS2DH12) and 0x51
      (PCF85063A) answer. The schematic leaves the bus's pull-up resistors unfitted
- [ ] The LCD: add `dtoverlay=spi6-1cs`, see `/dev/spidev6.0`, and get a test pattern onto the
      display with chip select on GPIO18 and reset on GPIO16 (the earlier software used SPI mode
      3 at 400 kHz, with `0xF8` before a command and `0xFA` before data). Nothing may object to
      the overlay taking GPIO19
- [ ] The backlight on PWM channel 1 (`dtoverlay=pwm,pin=13,func=4`)
- [ ] The buttons on GPIO23, 24 and 25 read low while pressed
- [ ] The buzzer sounds with GPIO17 low. Listen at power-on too: until `config.txt` sets the
      line, only the Raspberry Pi's weak pull-down holds it
- [ ] With a RAK5146 USB card in the mini-PCIe slot: which level on GPIO11 (pin 22) lets the
      card appear on USB (`0483:5740`)
- [ ] Write every result, with the date, into `docs/bench-checks.md`, and correct
      `docs/hardware.md` where the board disagrees with it

Watch out: the probe resets both microcontrollers, and no other program may hold the two serial
ports (stop the earlier FIBER software if the unit has it). After any access over SWD an
STM32C0 keeps BOOT0 low until the next power-on, so do not mix a debug probe into this check.

- Needs: a FIBER-R2 rev 2.1 on the bench, reachable over SSH.
- Done when: every result is in `docs/bench-checks.md`.

### M1. Provisioning

Done when: a FIBER written with every box ticked reaches `state=ok` by itself, also on a network
that blocks NTP; a second `apply` changes nothing; no service ever answers with a default
password; update and rollback work.

#### T12. FIBER Setup: the component picker

In `../fiber-setup`. The one place where a user makes the choice.

- [ ] `Settings.install_fiber` becomes a map of components: `firstboot.rs` (the struct, its
      checks, what it writes), `types.ts`, `wizard.svelte.ts`, `api.ts`, the command line's
      `FlashArgs`, the *Software* card in `Settings.svelte`, and every view that looks at the
      old switch (`Image`, `Write`, `FirstStart`, `Done`). The texts in all five languages
- [ ] The list comes from a copy of `components.yaml` at the pinned release: what a component
      requires is ticked with it, and its download size is shown
- [ ] `personalise()` writes `fiber-seed.yaml` beside `fiber-install.sh`: the components, the
      settings, and the user name and password for the services (D16). The file joins the list
      of old files that are cleared from the boot partition
- [ ] `catalog.rs`: the installer by its version and sha256, as the system image is, in place
      of an address on a `main` branch. `fiber-install.sh` fetches it and checks it; the test
      that keeps the two in step stays
- [ ] The clock: take the date over plain HTTP before anything over HTTPS, and only ever move
      the clock forward
- [ ] `fiber-install.service` tries again after a failure without waiting for a restart
- [ ] cloud-init sets the final name, `fiber-<last 8 characters of the serial number>`, always
      with `manage_etc_hosts: true`; the line that edits `/etc/hosts` goes
- [ ] *Follow the first start* reads the progress from the platform's Avahi record. The last
      step shows a link for every chosen component, and says in plain words that these services
      can be reached from the network with the password from the settings
- [ ] The choice "set Wi-Fi later from your phone" stays hidden until T29
- [ ] With nothing ticked no script is written, as today
- [ ] Its README: the FIBER software is this platform; "about 10 minutes" becomes what the
      bench shows
- [ ] On a FIBER: every box ticked; the same with NTP blocked; with the Ethernet cable plugged
      in only later; with the power cut in the middle of the installation; with a RAK5146 card
      and with the LTE modem in the slot; and on a module with 1 GB of RAM, with the time, the
      download size and the memory in use written down. Each component's download and installed
      size go into `size` in `provision/components.yaml`, which is empty until then

Watch out: today the script tests `https://github.com` before the clock is right. On a network
that blocks NTP, TLS then fails, the script says "no internet", and its clock fallback (also
HTTPS) is never reached. The password field is pre-filled with `fiber`, and from now on that
password also opens services on the network.

- Needs: T5 to T8 (a release to point at). D11 may change one step.
- Done when: a FIBER written by the new FIBER Setup comes up with the chosen components, and the
  last step shows a link for each.

### M2. The agent's core and the south bridge

Done when: a unit with the earlier south bridge firmware is flashed by the agent on its own; a
power cut in the middle of flashing recovers; stopping the agent shows the fallback LED pattern.

#### T15. `fiber-flasher`

Writing a microcontroller's firmware from the CM4, in a way that cannot leave it dead.

- [ ] The chips: product id `0x453` (STM32C031, 16 pages of 2 KB) and `0x497` (STM32WL55, 128
      pages of 2 KB). Any other id is refused before anything is erased
- [ ] The reset lines behind a trait: NRST is pulled low or let go, never driven high; BOOT0 is
      driven. Entering the bootloader is tried a few times, holding BOOT0 longer each time
- [ ] The serial port: 115200 baud with even parity for the bootloader, and back to no parity
      for the firmware
- [ ] The commands: Get, Get ID, Read Memory, Write Memory (at most 256 bytes, in multiples of
      8), Extended Erase by a list of pages, Go. No mass erase
- [ ] The image is checked before anything is erased: its size, where its stack starts, where
      its reset vector points
- [ ] The block with the vector table is written last, after everything else was read back and
      compared. A flash that is cut short then leaves a chip that waits in its bootloader, not
      one that starts half a program
- [ ] It leaves through `Go`, not through a reset: after a first flash the chip still believes
      its memory is empty and a reset lands in the bootloader again
- [ ] The option bytes are read and reported (`nBOOT_SEL`, `nSWBOOT0`, readout protection), and
      a chip with readout protection is refused. The flasher never writes them
- [ ] Progress is reported to the caller, nothing is printed; an error type of its own
- [ ] A simulated bootloader for the tests that models the two lines, the "empty" flag, and the
      refusals a real one gives
- [ ] A lock, so that a command run by hand keeps off a serial port the agent is using
- [ ] On a FIBER: both chips' identity is read; a test image is written to the south bridge and
      verified; the same with the power cut in the middle, then recovered

Watch out: a write of the STM32C0's option bytes that is cut short can leave the chip with
readout protection on and its bootloader out of reach, which only SWD can undo. That is why only
the south bridge firmware changes them, once (T16).

- Needs: T2 (the bootloaders answer).
- Done when: the three steps on a FIBER pass.

#### T16. The south bridge firmware

The status lights, the power of the 1-Wire ports and two voltages, in at most 24 KB.

- [ ] `crates/south-core`: the LED engine (off, on, blinking, by colour, on a 50 Hz tick), what
      happens with a `Desired`, the fallback pattern after 5 s without one, the ports switched
      on one after another at start. Tested on the computer, and built as a simulator on a
      pseudo-terminal for the agent's tests
- [ ] `firmware/south`: a workspace of its own for `thumbv6m-none-eabi`, with the linker and
      the profile from T3 (`docs/bench-checks.md`, check 5: `flip-link`, `opt-level = "z"`,
      fat LTO, one codegen unit, `panic = "abort"`)
- [ ] The board: USART1 on PA9/PA10 at 115200; the 18 LED lines (low = lit) and the 8 port
      lines (high = on) as `docs/hardware.md` has them; the ADC on PA11, PA12 and the internal
      reference, as raw counts; the watchdog; the 32.768 kHz crystal
- [ ] `Hello` at start, with the CRC-32 the firmware works out over its own image
- [ ] A start pattern on the LEDs until the agent sends its first `Desired`
- [ ] A panic stores a fault code and the address in memory that survives a reset, resets, and
      reports them in the next `Hello`
- [ ] The option bytes: read at start; if `nBOOT_SEL` is 1, bit 24 is cleared, once (the value
      then is `0xDEFFE1AA` on a factory part). Nothing else in them is ever touched. The stale
      "empty" flag is cleared at start
- [ ] `just` recipes for building and measuring; a CI job: build, clippy, rustdoc, and the size
      against the budget
- [ ] `docs/firmware/south.md`

Watch out: no formatting code, no floats: the counts are turned into volts by the agent. The
chip has no EEPROM and keeps nothing: after a reset the agent's next `Desired` puts everything
back. It is fed from the CM4's 3.3 V and is off whenever the CM4 is. T3's skeleton took 15.0 KB;
a panic handler that reads the panic's location costs 3.2 KB of it (every source path in the
HAL), so a panic may have to record its fault code without a location; the HardFault's address
costs 82 bytes.

- Needs: T3, T14.
- Done when: it builds under 24 KB, and on a FIBER the lights and the ports follow `Desired`.

#### T17. The agent's core

The one program that owns the board. This task gives it the south bridge and its first topics.

- [ ] The crates: `tokio` on one thread, `rumqttc` without TLS, `tokio-serial`, `gpiocdev`
- [ ] `agent/src/board.rs`: every pin, device path and I2C address of rev 2.1, from
      `docs/hardware.md`. A test that no GPIO is used twice
- [ ] The shape: one task holds the state; MQTT is an adapter around it (the LCD and Bluetooth
      will be two more). The board keeps working while the broker is down
- [ ] The south link: `Desired` every second, `Status` and `Hello` read; the counts turned into
      volts (battery: the pin times 2; input: the pin times 398 / 68); PoE or battery, with a
      margin so that it does not flap
- [ ] Firmware: both images are built into the program. A chip whose `Hello` names another
      image or another schema tag, or that says nothing, is flashed (T15). At most three tries
      for one image in an hour, remembered across restarts
- [ ] The mini-PCIe card's two lines (GPIO10 and 11), set for the card that
      `hardware.minipcie` names, with the levels T2 found
- [ ] MQTT: the last will on `status`, every retained topic sent again after a reconnect,
      requests and answers, the `schema` number
- [ ] `/etc/fiber/agent.toml`, with defaults that need no file
- [ ] The command line as a client of MQTT: `status`, `power`, `ports`, `leds`, `firmware
      status`. `firmware flash` and `doctor` talk to the hardware themselves and refuse while
      the agent holds it
- [ ] The service: the user `fiber`, read-only system directories, only the devices it needs, no
      capabilities, a watchdog
- [ ] Tests without a board: against the south bridge simulator, the simulated bootloader, and
      a broker inside the test

Watch out: the schematic has a variant of the board without the two voltage dividers. On such a
board the two voltages read nothing useful, and `power` has to say so.

- Needs: T14 to T16.
- Done when: the three lines under "M2" hold on a FIBER.

#### T18. The `agent` role

- [ ] The service unit, the user and its groups, `/etc/fiber/agent.toml`, enabled and started
- [ ] Turning it off stops and disables the service; both microcontrollers keep their firmware
- [ ] In the converge test: the agent starts against the simulators

- Needs: T6, T17.
- Done when: `fiber platform enable agent` on a FIBER brings up `fiber/status`.

### M3. Thermometers

#### T19. Thermometers

The first thing most users will plug in: a DS18B20 on one of the eight ports.

- [ ] Thermometers behind a trait. The first implementation reads the kernel's 1-Wire tree: one
      bus master for each port, found under the DS2482's own device and not by its number
      (numbers follow the order of registration); all probes converted at once; the reads kept
      off the main task (one read blocks for about 750 ms)
- [ ] What is not a reading: an empty read, the 85 °C a probe reports right after power-on, a
      probe that is gone (the kernel notices slowly)
- [ ] The port lights: what green, red and off mean for a port, written in `docs/mqtt-api.md`
      first
- [ ] `thermometers/{rom}` (retained; an empty message when a probe is gone) and the probes of
      each port in `ports/{n}`; `fiber thermometers` and `fiber thermometers watch`;
      `fiber ports cycle N` switches a port off and on
- [ ] Tests against a made-up 1-Wire tree under a root that the tests set
- [ ] The Node-RED starter flow shows the thermometers
- [ ] On a FIBER: plug a probe in and out; short a port; eight ports with probes

Watch out: if plugging in feels slow (the kernel searches a bus only every ten seconds), the
other implementation is a driver of our own for the DS2482 over `i2c-dev`, about 700 lines. The
trait is there so that this is a swap (D17).

- Needs: T7, T17.
- Done when: a DS18B20 shows up on `fiber/thermometers/...` and in the starter flow; plugging a
  probe in and out, and a shorted port, behave.

### M4. The front panel

Done when: the LCD shows the name and the address after the start, and the buttons move through
the pages.

#### T20. LCD, backlight, buttons, buzzer

- [ ] `docs/mqtt-api.md` and `docs/cli.md` first: `buttons/event`, `display`, and the methods
      `buzzer.beep`, `display.message`, `display.backlight`
- [ ] The display driver over `/dev/spidev6.0`, with what T2 found (the mode, the speed, the
      bytes before a command and before data), reset on GPIO16
- [ ] The picture is a `tower_hmi::Frame<16, 64>`; only the rows that changed are sent
- [ ] The backlight on PWM channel 1, with a brightness and a time after which it dims
- [ ] The buttons on GPIO23, 24 and 25 through `gpiocdev`, with the pull-ups and the kernel's
      debounce; click, hold and repeat from `tower-hmi`
- [ ] The buzzer on GPIO17 (low = sound): a beep, and a few patterns
- [ ] Tests: the frames a drawing produces, compared with stored ones; made-up button presses

- Needs: T2 (the LCD's bus), T17.
- Done when: `fiber display`, `fiber buzzer beep` and `fiber buttons watch` work on a FIBER.

#### T21. Pages and the menu

- [ ] The pages: the name, the addresses and the time; the components and their state; the power
      source and the voltages; the thermometers; the radio
- [ ] The menu, with `tower-hmi`'s menus: restart, about (the versions), and later pairing and
      setup from the phone
- [ ] While an apply runs, its progress (from `/run/fiber/platform/status.json`)
- [ ] A stored picture of every page, compared in the tests

- Needs: T20.
- Done when: the pictures match, and on a FIBER the buttons move through the pages.

### M5. The north bridge

Done when: two TOWER Core Modules at the pinned revision pair with the FIBER, their events arrive
on MQTT and a `set` reaches them; at least 99 % of frames are acknowledged at the first try;
replays are refused after a restart of either side; 24 hours pass without a fault.

#### T22. The radio spike

TOWER nodes use a SPIRIT1 radio; the north bridge has another one. Whether the two understand
each other is the biggest unknown of the plan, so it is answered before anything is built on it
([`docs/bench-checks.md`](docs/bench-checks.md), check 4). About two days.

- [ ] A small Embassy firmware for the STM32WL55's M4 core (`stm32wl55jc-cm4`; the M0+ core is
      never started), written to the chip through its bootloader. The released
      `embassy-stm32` 0.6.0 has no driver for the radio: it takes a little glue for its SPI and
      its interrupt
- [ ] The radio up: the TCXO switched on (1.7 V, 1 ms to settle) and the system clock taken from
      it; the calibration for 863 to 870 MHz; the RF switch on PC3, PC4 and PC5 (receive: all
      high; transmit on the low-power PA: PC3 and PC4 high, PC5 low)
- [ ] GFSK as TOWER sends it: 19 216.6 bit/s (the rate a SPIRIT1 really produces, not the round
      19 200), 19.8 kHz deviation, BT 1, sync word `0xDB624715`, a preamble detector of 16 bits,
      156 kHz receive bandwidth
- [ ] Receive raw: fixed length, the radio's own whitening and CRC off, an interrupt at the sync
      word. Capture the bytes after the sync word from a Core Module that runs TOWER's
      `radio_beacon` at the pinned revision
- [ ] From the captures settle the whitening (a 9-bit sequence that starts with all ones; in
      which order its bits fall into a byte) and the CRC-16 (polynomial `0x1021`; its start
      value; what it covers). Keep the captures as test vectors
- [ ] Try the shorter way too: the radio's own variable length and whitening, with a correction
      in software. It loses the two CRC bytes; every frame still carries an 8-byte tag
- [ ] Send frames built in software to the Core Module, and read the frequency offset it
      reports, on several modules
- [ ] The error rate at -60, -90, -100 and -105 dBm, for 117, 156 and 187 kHz bandwidth
- [ ] Inside the closed FIBER: the noise floor, with the linear regulator and with the switching
      one, with and without the boosted receive gain (the earlier firmware measured a weak
      receiver)
- [ ] The results into `docs/bench-checks.md` and `docs/firmware/radio.md`

Watch out: by the documents the radio's own whitening has the right polynomial and seed and the
wrong bit order, so it cannot be used as it is. That is read from documents and from a capture
of a related chip, not seen on a SPIRIT1. This radio has no automatic frequency correction, so
the receive filter has to be wide enough for both crystals' error.

- Needs: T2 (the bootloader answers, to write the test firmware); a TOWER Core Module.
- Done when: frames go both ways, and the captures are in the repository.

#### T23. The radio's command layer and the packet codec

- [ ] The command layer for the radio inside the STM32WL (about 500 lines; no maintained crate
      does GFSK): standby, packet type, frequency, modulation, packet settings, interrupts, the
      buffer, the signal strength of a packet and of the channel, the transmit power on the
      low-power PA (+14 dBm, the limit of the 25 mW band), the TCXO, the RF switch. Laid out so
      that LoRa is another packet type and not a rewrite (D4)
- [ ] The packet codec in `fiber-tower-air`, as pure functions tested against the captures of
      T22. To send: length, frame and CRC, whitened. To receive: the length byte as soon as it
      is in, then the rest, de-whitened and checked
- [ ] The receive flow in the firmware: the interrupt at the sync word, one byte time later the
      length, a timer for the rest, read, standby, receive again
- [ ] The `Radio` trait the gateway is written against: send a frame, receive one with its
      signal strength, set the channel

Watch out: this radio gives a signal strength and nothing like the SPIRIT1's link quality
numbers, so our messages carry the strength alone.

- Needs: T22.
- Done when: the codec passes every capture, and the test firmware exchanges frames with a Core
  Module through it.

#### T24. The frame codec and the gateway's flow

What TOWER's own gateway does inside its firmware, written again over our radio. Every decision
about security stays TOWER's code (D1).

- [ ] The frame, as TOWER's `firmware/src/radio/frame.rs` has it: `fmt_type | flags | src(4) |
      dest(4) | counter(4) | [bulk_idx(3)] | payload | tag(8)`; the clear header is
      authenticated, the payload encrypted; the nonce from `tower-net`; AES-128-CCM from
      `tower-radio`, over an AES in software
- [ ] The gateway's flow over `tower-net`: only nodes it knows; the replay rule; confirmed
      delivery with an acknowledgement (the counter it answers, the signal strength, the
      "more waiting" flag); the transmit counter with its reserve; one waiting message for each
      sleeping node, sent when the node next speaks; pairing over the air (the key agreement,
      the commitment round, the six-digit code)
- [ ] The duty-cycle budget for EU 868 from `tower-radio`; the band and the channel as settings
- [ ] Messages to nodes (`Set`, `Time`, `Describe`) and from them (`Info`, `Report`, `Event`)
      pass through untouched: the agent reads them, the firmware does not
- [ ] Tests on the computer: the vectors (a frame, its nonce, its encrypted form, from a real
      Core Module with a known key); the flow against a made-up radio (a lost acknowledgement, a
      replay, pairing again)

Watch out: the parts copied in spirit from TOWER (`radio/frame.rs`, `radio/net/mod.rs`,
`radio/net/pairing.rs`) are what the drift report of T27 watches. Left for later: frequency
hopping (US 915), the remote shell, bulk transfer.

- Needs: T23.
- Done when: the vectors pass, and the flow tests cover the three cases above.

#### T25. The north bridge firmware and its link

- [ ] `docs/firmware/north-link.md` first: `Hello` (who it is, its image's CRC, the
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
- [ ] `fiber radio status | config | nodes | pair | watch`
- [ ] Tests against the north bridge simulator

- Needs: T17, T25; T21 for the code on the LCD.
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

#### T28. The Bluetooth profile, written first

A new, small profile (D3), agreed with the people who make the app before any code.

- [ ] `docs/ble.md`: one service with three characteristics, `info` (read), `rpc_rx` (write)
      and `rpc_tx` (notify). They carry the agent's requests, and only these: `device.info`,
      `setup.state`, `wifi.scan`, `wifi.connect`, `net.status`
- [ ] Who may talk: pairing with a passkey that the FIBER shows on its LCD; the two request
      characteristics only on a link paired that way; advertising and pairing open only after a
      button on the FIBER was pressed
- [ ] What happens if phones pair unreliably: the same code used in a key exchange of our own,
      above Bluetooth's
- [ ] Agreed with the HARDWARIO Manager app

- Done when: the app's side has said yes to the document.

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

#### T31. Examples

- [ ] Node-RED flows: the thermometers, a button that sounds the buzzer, what a TOWER node sends
- [ ] MQTT from the command line, from Python and from Rust
- [ ] Each example tried on a FIBER, and in CI where it can run without one

#### T32. A ready-made image

- [ ] An image built in CI from the roles' `install` step, with every component in it and none
      turned on
- [ ] The first start then only does the roles' `bootstrap` step, and needs no internet
- [ ] FIBER Setup offers it in place of the stock image (its catalog already expects one)

- Needs: T11.

#### T33. Retire the earlier repositories

- [ ] `hardwario/fiber-agent` and `hardwario/fiber-platform-legacy`, as D12 says

- Needs: D12, T12.

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

## Decisions ahead

### D11. Should FIBER Setup write the `config.txt` block when it writes the system?

Today only the `base` role writes it, after the first start.

- (a) One writer, the `base` role. During the first installation the GPIO lines sit at their
  power-on levels, and one restart is needed afterwards.
- (b) FIBER Setup writes the same block and the overlay. Defined levels and a working clock from
  the first second and no restart, but two writers of one block.

Recommendation: (a), until the first installation has been watched on the bench. It changes one
step of T12 and blocks nothing.

### D12. What happens to `hardwario/fiber-agent` when FIBER Setup moves to this platform?

FIBER Setup 1.2.0, as released, fetches `bootstrap.sh` from that repository's `main` branch.

- (a) Leave it as it is, and archive the repository once a FIBER Setup with the new installer is
  out. An old FIBER Setup then keeps installing what it was tested with.
- (b) Put a forwarder to this platform's installer in place of its `bootstrap.sh`.

Recommendation: (a). Blocks T33.

### D20. Should the Node-RED component bring the dashboard nodes?

With InfluxDB and Grafana out (D13), nothing on a FIBER shows a value as a chart or a gauge.

- (a) Install `@flowfuse/node-red-dashboard`, pinned, and give the starter flow one page (a
  thermometer). The earlier student guide did this.
- (b) Node-RED alone. The starter flow shows messages in the debug panel; a user adds the
  dashboard from Node-RED's own palette.

Recommendation: (a): it is what makes the first minute with a FIBER show something. Changes one
step of T8 and blocks nothing else.

### D21. Who moves the Rust toolchain?

`rust-toolchain.toml` pins the channel, and `AGENTS.md` says it is the one the pinned TOWER
crates need, so it moves when the TOWER pin moves. Dependabot was set up to watch it anyway, and
on the first day it opened pull request #1 (1.99.1 to 1.100.0). Its CI fails, as it should: the
`audit` job checks that every workflow sets the same `RUST_TOOLCHAIN`, and Dependabot changes
only the one file.

- (a) Take the channel off Dependabot's list. It then moves only with the TOWER pin, as the rule
  says, and this platform always builds TOWER's crates on the compiler TOWER itself uses.
- (b) Keep it, and push the matching `RUST_TOOLCHAIN` change onto every such pull request by
  hand, as `.github/dependabot.yml` says today. Rust releases arrive every six weeks.

Recommendation: (a), and close pull request #1. Blocks nothing. Until it is answered, the pull
request stays open and red.

### D22. May the release and the canary workflows write?

`AGENTS.md` says that the workflows may only read the repository, and that no workflow gets more
rights. T11 needs two things that write: publishing a release (`contents: write`, and
`id-token: write` with `attestations: write` for the provenance attestation), and the canary
opening an issue when a package source breaks (`issues: write`).

- (a) As TOWER does it: one job of `release.yml` that gets those rights, runs only on a tag that
  passed the gate (the version, on `main`, CI green), and uploads to a draft first; and one job
  of `canary.yml` with `issues: write`. `AGENTS.md` names these two jobs as the exceptions.
- (b) No workflow writes. A person publishes a release by hand from the dry run's artifact
  (`gh release create`), and reads the canary's failures on the Actions page.

Recommendation: (a). Blocks the last step of T11. Everything else of T11 is written read-only.

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
