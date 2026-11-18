# Provisioning

How a vanilla Raspberry Pi OS becomes a FIBER: what is installed, how the choice is stored, and how
it is applied again later.

**Status: partly built** (`TASKS.md`, T4 to T12, T18). Built so far: `provision/components.yaml`;
the `fiber platform` commands that read and change `/etc/fiber/platform.yaml` and take the seed
([`cli.md`](cli.md)); the release archive and the installer; the roles `base`, `mosquitto`,
`node_red`, `chirpstack` and `agent`, tested in containers and not run on a FIBER yet. The
package sources and versions below were checked against the upstream documentation and
repositories on 2026-11-17; check them again when each role is changed.

## The idea

- **The state is one file**, `/etc/fiber/platform.yaml`: which components are wanted, and a few
  settings. FIBER Setup writes the first version; a person can edit it. `fiber platform enable`
  and `disable` change only the lines under `components:`, so comments and everything else stay
  ([`cli.md`](cli.md)).
- **Ansible converges the system to that file.** It runs on the device itself. It keeps no memory
  of what it installed, so turning a component off is an explicit path in its role.
- **Everything arrives in one release archive**, unpacked under `/opt/fiber`; nothing is spread
  over `/` by hand.

```yaml
# /etc/fiber/platform.yaml
schema: 1
components:          # true: converge, false: stop and disable (data kept), absent: untouched
  base: true
  mosquitto: true
  agent: true
  node-red: true
settings:
  mqtt: { lan: false }
  lorawan: { region: eu868 }
  hardware: { minipcie: none }       # none | lte | rak5146
```

- **A component that is on brings up what it requires,** whether the file names it or not. A
  component that is on while something it requires is `false` is an error, and the file is
  refused with both names.
- **Every setting has a default:** `mqtt.lan: false`, `lorawan.region: eu868` (the only region the
  `chirpstack` role sets up so far), `hardware.minipcie: none`.
- **Strict values:** `true` and `false` only, not `yes` or `on`. An unknown key or component is
  named, with its line.
- **`schema: 1`** must be there. A file for another schema is refused as a whole.

## On the device

| Path | What |
|---|---|
| `/opt/fiber/releases/<version>/` | One unpacked release: `bin/fiber`, `provision/`, `manifest.json`. Unpacked as `.partial`, verified against the manifest, renamed, marked `.complete` last. Never edited in place. |
| `/opt/fiber/current`, `previous` | Symlinks. A switch is one rename; the previous release stays for a rollback. |
| `/usr/local/bin/fiber` | Symlink into `current` |
| `/etc/fiber/platform.yaml` | The declared state |
| `/etc/fiber/secrets/` | Root only: the seeded login (`login.json`), generated tokens |
| `/opt/fiber/downloads/` | An archive while it is downloaded and checked; empty otherwise |
| `/var/lib/fiber/` | State |
| `/var/lib/fiber/platform/results.json` | The last result of each component: what it was brought to (`on` or `off`), `ok`, `failed` or `skipped`, when, and why. Written by `apply` as each component finishes; read by `fiber platform status` |
| `/var/lib/fiber/platform/pending` | There while an apply is asked for and not finished |
| `/var/log/fiber/platform.log` | Every apply |
| `/run/fiber/platform/status.json` | Live progress: state, step, result per component, reboot needed |

**The footprint rule** stands in for a package manager's file list: every file written outside
`/opt/fiber`, `/etc/fiber` and `/var/lib/fiber` is named `fiber-*` or sits in a drop-in directory,
and is listed in `provision/footprint.yaml`, with the component that writes it and its kind:
removed by an uninstall, or kept as data, or a package's own file the platform rewrote. The
converge test compares the filesystem before the install and after a converge with every
component, and fails on any new path that neither a Debian package (its file list, or what its
scripts make) nor the platform's own places nor the footprint accounts for. After an uninstall
it checks that only packages' paths and data are left. Two exceptions are written in the
footprint as they are: Node-RED, installed by npm into `/usr/lib/node_modules`, and the
`nodered.service` unit, named as Node-RED's own installer names it.

## Commands

`fiber platform list | status | apply | enable <component> | disable <component> | update [--to
version] | rollback | uninstall`. The ones that exist are described in [`cli.md`](cli.md).

- `apply` starts `fiber-platform-apply.service` and follows it (below, "How an apply runs").
- `disable` stops and disables the services and keeps the data. Nothing is purged.
- `update` fetches the target release's `install.sh`, verifies it and runs it: there is one
  installer. The new release is unpacked beside the old one and `current` moves only on success.
  It finds the releases where GitHub serves their files (`releases/latest/download/`,
  `releases/download/v<version>/`); `FIBER_RELEASES_URL` points elsewhere, for the tests.
- `rollback` restores our binary, roles and units. It does not downgrade third-party packages.
- `uninstall` turns every component off and removes the platform's files; data stays.
- All three are tested in the converge test: an update to a test release
  (`release.py --version`; the program in it still says its own version), a rollback, an update
  to the current release (nothing happens), and an uninstall. Not run on a FIBER yet.

### How an apply runs

`fiber platform apply` writes the id of its request into `/var/lib/fiber/platform/pending`,
puts `fiber-platform-apply.service` in place from its release (when it differs), starts it, and
prints each step as it ends. The service runs `fiber platform apply --foreground`, which does the
work:

1. **The plan:** `common` first; then every component that is `false`, in the reverse order of
   `components.yaml`; then every component that is on (named or required), in its order.
2. **One `ansible-playbook` run per step,** `-c local`, on `provision/ansible/site.yml` of the
   release the program itself belongs to, so a program never runs another release's roles. The
   program hands over what it worked out in one variable, `fiber`: the component, its roles, `on`
   or `off`, the settings, the release and the paths. No role reads `platform.yaml` itself.
3. **A step's result is its run's exit status.** A component whose requirement did not come up in
   this run is skipped, and so is everything when `common` fails. One unreachable package source
   stops only the components that need it.
4. **As each step ends:** its result goes to `results.json`, the progress to
   `/run/fiber/platform/status.json` and to the Avahi record, and Ansible's output to
   `/var/log/fiber/platform.log`. At the end the status and the log are copied to the boot
   partition (`fiber-platform-status.json`, `fiber-platform.log`).
5. **The end:** 0 when every step is `ok`; 10 when also a role asked for a restart (it touches
   `/run/fiber/platform/restart-needed`); 20 otherwise. On 0 and 10 `pending` is removed; on 20 it
   stays, and the service tries again after 10 s, then with longer and longer pauses, up to
   30 minutes. A request that comes in while an apply runs makes it run once more.

The service runs only while `pending` exists, at every start too, so an apply cut by a power cut
goes on after the next start. Two never run at once. An apply that has nothing to install needs
no network: a role updates the package lists only when a package is missing.

**The Avahi record** (`/etc/avahi/services/fiber-platform.service`, `_fiber._tcp`) carries the
progress in its TXT records (`state`, `step`, `component`, `release`, `restart`), so that FIBER
Setup can follow the first start without a login. It is written only where Avahi is installed.

**For the converge test,** the components named in `/run/fiber/platform/fail-on-purpose` fail on
purpose. The file is in `/run`, so it is gone after a restart.

## Components

`provision/components.yaml` is the one manifest: ids, titles, what each requires and suggests,
its roles, services and ports, and its sizes once they are measured. The `fiber` program is
built with it inside, so the two never disagree; FIBER Setup's picker is built from a copy of
it.

| Id | What it installs | Requires |
|---|---|---|
| `base` | The `config.txt` block below, the DS2482 overlay, the `i2c-dev` module, groups | |
| `mosquitto` | The broker, loopback only | |
| `agent` | The agent's service and settings | `base`, `mosquitto` |
| `node-red` | Node.js, Node-RED, a starter flow | `mosquitto` |
| `chirpstack` | ChirpStack, Redis, the concentrator daemon and the MQTT forwarder | `mosquitto` |

A database and charts (InfluxDB, Grafana) are left out for now (`TASKS.md`, D13).

After `base` alone the board is usable with plain Linux knowledge: `/dev/rtc0`, DS18B20 probes in
the 1-Wire sysfs tree (`w1_bus_master1` to `8`, one per port), `/dev/ttyAMA3` and `4`,
`/dev/spidev6.0`.

### Rules for every role

A role is `provision/ansible/roles/<name>/`. The `mosquitto` role shows the shape:

| File | What it does |
|---|---|
| `tasks/install.yml` | Packages, files, enabling. Nothing that needs the service running, so that an image build (T32) can run it in a chroot |
| `tasks/bootstrap.yml` | What needs the running service: starting it, tokens, passwords |
| `tasks/main.yml` | `install`, then `bootstrap`: what `on` runs |
| `tasks/disable.yml` | What `off` runs: the services stopped and turned off, the settings and the data kept |
| `handlers/main.yml`, `templates/` | As Ansible has them |

- **Packages** come through the `common` role's `packages` tasks (`common_packages: [...]`): they
  update the package lists only when a package is missing, and install without starting the
  service (`policy_rc_d: 101`).
- **Configure a service before its first start.** No service may ever answer with a default
  password.
- **Drop-in files** in place of edits to a package's own settings, named `fiber.conf` or
  `fiber-*`, and on the footprint list.
- **A password is set once and never set again,** so that one the owner changed stays changed.
  The task that sets it runs only while its stamp `/var/lib/fiber/platform/set-once/<name>` does
  not exist (`creates:`), and makes the stamp when it is done. Every other task can run again and
  again.
- **Every `command` task** has `creates` or a real `changed_when`, or a second apply is never
  clean.
- **Version pins** live in one file, `provision/ansible/versions.yml`.
- **`ansible.builtin` modules only,** and no `meta` dependencies: the program works out what a
  component requires. `just lint-ansible` runs `yamllint` and `ansible-lint` (its `production`
  profile and `only-builtins` rule) in a Debian container.
- **A role that is not written yet** fails with a message that names its task, and its `disable`
  does nothing, as nothing of it can have been installed.

### Mosquitto and Node-RED

**Mosquitto** (the `mosquitto` role) is Debian's. Its own `mosquitto.conf` stays as it is; the
platform writes `/etc/mosquitto/conf.d/fiber.conf`:

- `mqtt.lan: false`: no listener there, so Mosquitto serves port 1883 on loopback only, without a
  login.
- `mqtt.lan: true`: `per_listener_settings true`, port 1883 on loopback without a login, and port
  1884 on every interface with the login of `/etc/mosquitto/fiber-passwd` (owned by `mosquitto`,
  readable by nobody else). That file is made once by `mosquitto_passwd`, from the login typed
  into FIBER Setup, and never again: the owner changes the password with
  `sudo mosquitto_passwd /etc/mosquitto/fiber-passwd <user>`.

**Node-RED** (the `node_red` role, component `node-red`):

- **Node.js 24 from NodeSource,** with its key vendored in the role
  (`/etc/apt/keyrings/fiber-nodesource.asc`) and an apt preference that keeps `nodejs` on
  NodeSource. **Node-RED from npm,** `npm install -g --omit=dev node-red@<pin>`, run only when that
  version is not the one installed.
- **The user `nodered`,** in the groups `dialout`, `gpio`, `i2c` and `spi`, with
  `/var/lib/node-red` as its directory. **Our own `nodered.service`,** with Node's memory capped at
  256 MB.
- **`settings.js` is written once,** before the first start, with the login typed into FIBER Setup
  as a bcrypt hash made by the `bcryptjs` that Node-RED brings. The platform never writes it
  again, so the owner's changes, and a changed password, stay.
- **A flow to start from,** written only when there is no `flows.json`: a broker node on
  `localhost:1883`, everything under `fiber/#` in the debug panel, and a line for each reading
  of the thermometers.
- Port 1880 on every interface; the editor asks for the login.

Both are tested in the converge test: the login from the seed works and nothing works without it;
a password the owner changes stays changed; a second apply changes nothing; only ports 1880, 1883
(loopback) and 1884 listen. Not run on a FIBER yet.

### ChirpStack

The `chirpstack` role brings a LoRaWAN network server, and the gateway for a RAK5146 USB card in
the mini-PCIe slot:

- **From ChirpStack's package source** (key vendored, fingerprint checked; the source is signed by
  the primary key, so the signing subkey that expired on 2025-12-21 does not matter):
  `chirpstack-sqlite` and `chirpstack-mqtt-forwarder`; Redis from Debian, on loopback. Nothing
  starts when it is installed.
- **Never with admin/admin.** The package enables ChirpStack; the role turns it off again until
  the admin password is set. Before ChirpStack first starts, as the user `chirpstack`,
  `chirpstack set-password --email admin --password-file ...` takes the password typed into FIBER
  Setup from a root-only file in `/run` that is deleted at once (`--stdin` wants a terminal). A
  stamp keeps it from ever being set again. The converge test asks the API with admin/admin all
  through the apply, and it never answers.
- **ChirpStack's user is `admin`,** with the FIBER's password: its command line can only set the
  password of a user that exists. The other services take the user name typed into FIBER Setup.
- **An API key** for the FIBER, made once (`create-api-key --name fiber`; every call makes a new
  one), kept in `/etc/fiber/secrets/chirpstack-api-key`. **The API's secret** is the FIBER's own,
  made once, in `/etc/fiber/secrets/chirpstack-api-secret`.
- **`/etc/chirpstack/chirpstack.toml`** is the platform's: SQLite at
  `/var/lib/chirpstack/chirpstack.sqlite` with a write-ahead log and a busy timeout of 5 s, the
  API and the web interface on port 8080, the region from `lorawan.region`, device events on the
  local broker in JSON. The region's own file (`region_eu868.toml`) stays as the package has it:
  it has the eight EU 868 channels and listens on MQTT under `eu868/`.
- **The concentrator daemon** (`chirpstack-concentratord-sx1302`, a tarball checked by its
  sha256) goes to `/opt/fiber/concentratord/<version>/`, with `/etc/fiber/concentratord.toml`
  (`rak_5146`, `USB`, `/dev/rak5146`, its sockets in `/run/fiber-concentratord/`) and our own
  `fiber-concentratord.service`, bound to the card's device. Over USB the daemon sets up no reset
  line; its default for SPI would be GPIO17, the buzzer (read in its source, v4.7.1).
- **A udev rule** names a USB `0483:5740` serial port `/dev/rak5146` and starts the daemon and the
  MQTT forwarder; the forwarder runs only with the daemon. Both are always installed with the
  component, as the card can appear long after the start. `0483:5740` is any STM32 serial port,
  not this card alone.

Tested in the converge test, without a card. Not run on a FIBER yet.

### The agent

The `agent` role runs `fiber agent` ([`cli.md`](cli.md)) as a service:

- **The user `fiber`,** a system user without a login, in the groups `dialout`, `gpio`, `i2c`
  and `spi`: the serial ports, the GPIO lines, the I2C and SPI buses.
- **`fiber-agent.service`,** the role's own unit. `Type=notify`: the agent tells systemd when it
  runs, and its loop feeds a watchdog of 30 s, also while the south bridge's port is missing and
  while it flashes. When the loop stops, systemd restarts it after 5 s.
- **What the unit allows:** the system read-only but for `/var/lib/fiber/agent` (the flash tries),
  `/run/lock` (the lock on the south bridge's port) and `/sys` as its permissions have it (the
  backlight's PWM); of the devices only the classes `ttyAMA`, `gpiochip`, `i2c` and `spi`; no
  capabilities; IPv4, IPv6 and Unix sockets only.
- **`/etc/fiber/agent.toml`** is written once, with every setting at its default in a comment,
  and never again: the agent needs no settings, and the owner's changes stay.
- **A new program is started:** after an update or a rollback the role restarts the agent when
  the program it runs is not the one `/usr/local/bin/fiber` leads to.
- **Turned off,** the service is stopped and does not start with the FIBER. Both
  microcontrollers keep their firmware; the south bridge shows its fallback pattern after 5 s.

Tested in the converge test against the simulated south bridge. Not run on a FIBER yet: the
device classes and the groups of the devices are Raspberry Pi OS's, not seen on a FIBER yet.

### Sources (checked 2026-11-17)

Third-party repositories are written in deb822 form with a vendored armored key
(`Signed-By: /etc/apt/keyrings/fiber-<name>.asc`).

| Component | Source | Notes |
|---|---|---|
| Mosquitto | Debian: `mosquitto`, `mosquitto-clients` 2.0.21 | With no listener configured it serves port 1883 on loopback only, anonymously, which is the default wanted. `mqtt.lan: true` adds port 1884 with a login (`per_listener_settings true` must come first in the file). One drop-in in `/etc/mosquitto/conf.d`. |
| Node.js | NodeSource: `https://deb.nodesource.com/node_24.x`, suite `nodistro`. Key `https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key`, fingerprint `6F71F525282841EEDAF851B42F59B5F99B1BE0B4` | Debian's Node.js 20 is too old for Node-RED 5. An apt preference keeps `nodejs` on NodeSource. |
| Node-RED | `npm install -g --omit=dev node-red@<pinned>` (5.0.10, the newest on 2026-11-17; it asks for Node.js 22.9 or newer, and brings `bcryptjs` 3.0.3) | The installer script the old FIBER documentation names is gone (its URL returns 404), and its successor is not idempotent. Own `nodered.service` as a system user `nodered`, port 1880, `adminAuth` written before the first start, a starter flow copied only if none exists. |
| ChirpStack | `https://artifacts.chirpstack.io/packages/4.x/deb`. Key `https://artifacts.chirpstack.io/packages/chirpstack.key`, fingerprint `E078226D4144665AD8793A601CE2AFD36DBCCA00`. Packages `chirpstack-sqlite`, `chirpstack-mqtt-forwarder`; Debian `redis-server` | **The package starts on install, on `0.0.0.0:8080` with admin/admin.** Install it with `policy_rc_d: 101`, then `set-password --password-file` and `create-api-key` before the first start. The repository keeps one version per package, so pin the major only. A region file is mandatory. The unit is `chirpstack-sqlite.service`, and since 4.20.0 the package replaces the placeholder API secret with a random one on install (both read from the package, 2026-11-17). |
| Concentrator daemon | Tarball `chirpstack-concentratord-sx1302_4.7.1_linux_arm64.tar.gz` from `artifacts.chirpstack.io/downloads`, sha256 `328faf7e6fe6fcc9f5a50f8368fd520807ac2939214c607593318e017e857345` | Model `rak_5146`, flag `USB`. Always installed with `chirpstack`, started by a udev rule when the card (USB `0483:5740`) appears: the card can show up long after boot. |
| Ansible | Debian: `ansible-core` 2.19 and `python3-debian`, installed with `--no-install-recommends` | Without that flag apt also installs the `ansible` collection bundle (205 MB). `python3-debian` is what the deb822 module needs. |

One credential for all of it: the user name and password typed in FIBER Setup also log in to
Node-RED, ChirpStack and the LAN MQTT listener. They are handed over once in the seed
file on the boot partition, moved to `/etc/fiber/secrets`, and removed from the boot partition.

## The `base` role's `config.txt` block

Overlay names and pins were checked against the Raspberry Pi kernel tree (`rpi-6.18.y`). The
block is `provision/boot/config-fiber.txt`, with a comment above each line; without the comments:

```
# BEGIN FIBER PLATFORM
[cm4]
otg_mode=1
dtparam=i2c_arm=on
dtoverlay=i2c-rtc,pcf85063a,i2c_csi_dsi
dtoverlay=fiber-ds2482
dtoverlay=uart3
dtoverlay=uart4
dtoverlay=spi6-1cs
dtoverlay=pwm,pin=13,func=4
gpio=17=op,dh
gpio=7,22=ip,pu
gpio=26,27=ip,pd
gpio=23-25=ip,pu
[all]
# END FIBER PLATFORM
```

- `fiber-ds2482.dtbo` is the only overlay of our own (no stock overlay describes a DS2482): the
  chip's node on the `i2c_csi_dsi` bus, enabled as the kernel's `i2c-rtc` overlay enables that
  bus (`provision/boot/fiber-ds2482.dts`). `provision/release.py` compiles it with `dtc -@` into
  the archive; the role copies it to `/boot/firmware/overlays/`.
- The reset lines (GPIO7, 22) are inputs with pull-ups, so each microcontroller can still pull its
  own reset; the agent later requests them as open-drain outputs. The BOOT0 lines (GPIO26, 27) are
  held low. GPIO17 high keeps the buzzer silent.
- GPIO10 and 11 are left alone: the right level depends on the card in the mini-PCIe slot
  (`settings.hardware.minipcie`).
- A bare `dtoverlay=pwm` would take GPIO18, the LCD's chip select: the pin must be named.
- **Never** `dtparam=spi=on`, `dtoverlay=w1-gpio` or `ctsrts` on this board
  ([`hardware.md`](hardware.md)). `fiber doctor` fails when it finds one of them.
- **The GPIO numbers of the block are checked** against the agent's board module by a test, so
  the two places cannot drift apart.

What the role does:

- **The block goes at the end of `config.txt`,** between its two marks, and only when it differs;
  a block edited by hand is put back as it was, and the rest of the file is left alone. Before
  the first change it keeps a copy of the file, `config.txt.fiber-original`.
- **The boot partition is FAT:** files are written without owner or mode, and synced.
- **A changed `config.txt` or overlay ends the apply with "restart needed"** (exit status 10).
- It writes `/etc/modules-load.d/fiber.conf` (`i2c-dev`) and makes the groups `gpio`, `i2c` and
  `spi`.
- **Turned off,** it takes the block, the overlay and the module file away again (a restart is
  needed then too), and keeps the copy and the groups.

The converge test runs it on a real FAT filesystem: the stock `config.txt` of the image FIBER
Setup pins becomes the expected file byte for byte (`provision/tests/fixtures/`), a second
apply changes nothing, and a file edited by hand gets its block back. Not run on a FIBER yet.

## The release archive and the installer

A release holds four things. `just release` builds the first three into `dist/`
(`provision/release.py`); the release workflow (T11) adds the fourth and publishes them. A
release is never changed after it is published.

| File | What it is |
|---|---|
| `fiber-platform-<version>-linux-arm64.tar.gz` | `bin/fiber` (static, arm64), `provision/` (without its build and test tools), `manifest.json` |
| `install.sh` | The installer, with the release's version and the archive's sha256 written in |
| `SHA256SUMS` | The sha256 of the archive and of the installer |
| A provenance attestation | For people and CI; the FIBER does not check it |

**The archive is the same byte for byte in every build** from the same commit: its entries sorted,
every date the commit's, everything owned by root, gzip without a name or a date. `just check`
builds it twice and compares.

**`manifest.json`** names the release and lists every other file with its sha256:

```json
{
  "ansible_core": "2.19",
  "commit": "370c2ae...",
  "files": {"bin/fiber": "...", "provision/components.yaml": "..."},
  "schema": 1,
  "version": "0.1.0"
}
```

### The installer

```sh
curl -fsSL https://github.com/hardwario/fiber-platform/releases/download/v<version>/install.sh | sudo bash
```

It is the one installer: FIBER Setup's first start runs it, `fiber platform update` (T10) runs
it, and so can someone who wrote a plain Raspberry Pi OS with Raspberry Pi Imager. In six steps:

1. **The machine:** root, arm64, Raspberry Pi OS (`/etc/rpi-issue`) trixie, and the tools it
   uses. One installation runs at a time.
2. **The download:** the archive, checked against the sha256 written into the installer. A
   wrong one is deleted and nothing else happens.
3. **Unpacking:** into `/opt/fiber/releases/<version>.partial`; every file is checked against the
   manifest; then it is renamed and `.complete` is written last. `current` and `previous` move
   by renaming a new link over the old one, `/usr/local/bin/fiber` points into `current`, and
   only those two releases are kept. A release that is unpacked already is used as it is.
4. **Ansible:** `ansible-core`, `python3-apt` (for its `apt` modules) and `python3-debian` (for
   its `deb822_repository` module) from Debian, with `--no-install-recommends` (otherwise apt
   brings the `ansible` bundle, 205 MB), after `dpkg --configure -a`, waiting up to 10 minutes for
   the package lock.
5. **The choice:** the seed (below) through `fiber platform seed`, then the seed is overwritten
   and deleted. Without a seed, `--components a,b,c` turns those components on.
6. **The apply:** `fiber platform apply` (T6).

| Setting | Meaning |
|---|---|
| `FIBER_NO_REBOOT=1` | Do not restart at the end, even when the apply asks for it |
| `FIBER_NO_APPLY=1` | Install the release, but do not apply it |
| `FIBER_RELEASE_URL` | Where the archive is downloaded from (default: the release on GitHub) |
| `FIBER_ROOT` | A directory to install into instead of `/` (for the tests) |

| Exit status | Meaning |
|---|---|
| 0 | Done |
| 10 | Done; the FIBER needs a restart (it restarts by itself unless `FIBER_NO_REBOOT=1`) |
| 20 | Partly done; the platform tries again by itself |
| anything else | Failed; the message says why |

**What the sha256 proves.** Through FIBER Setup the chain is: the signed desktop application
pins the sha256 of `install.sh`, which pins the archive. With `curl | sudo bash` the same check
proves that the archive is the one the installer was made for, not who made the installer:
take the installer from the release page, over HTTPS.

**Run again, it changes nothing.** **Stopped hard at any point,** it starts over cleanly the
next time: a half unpacked release stays `.partial` and is removed. Both are tested in a container
like Raspberry Pi OS (`just check-install`, `provision/tests/`), not yet on a FIBER.

### The seed

FIBER Setup writes `/boot/firmware/fiber-seed.yaml` beside its first-start script (T12). It is
read as strictly as `platform.yaml`, plus the login:

```yaml
schema: 1
components: {base: true, mosquitto: true, agent: true, node-red: true}
settings: {mqtt: {lan: true}}
login: {user: pi, password: "the password from FIBER Setup"}
```

`fiber platform seed` writes `/etc/fiber/platform.yaml` from it, in the form `enable` makes,
and `/etc/fiber/secrets/login.json` (the directory and the file root only). Each only when it is
not there yet: a `platform.yaml` that exists is the person's, and a password is set once
(D16). The installer then overwrites the seed with zeros and deletes it.

A weekly canary workflow (T11) converges the whole stack against the live package sources and
opens an issue when one of them breaks.

## What changes in FIBER Setup

- The one switch "install the FIBER software" becomes a component map, with requirements ticked
  automatically and download sizes shown.
- It writes `fiber-seed.yaml` beside `fiber-install.sh` on the boot partition. The script keeps
  its waiting, retrying, logging and restarting; its last step fetches a **pinned** `install.sh`
  (version and sha256 in the catalog, like the system image) instead of a script from a branch.
- **A fault to fix:** the script tests `https://github.com` before the clock is right. On a
  network that blocks NTP, TLS then fails and the script reports "no internet" without reaching its
  clock fallback, which itself uses HTTPS. The date must come over plain HTTP first, and the clock
  may only be stepped forward.
- The retry unit gets `Restart=on-failure`, so plugging in the Ethernet cable later works without
  a restart.
- cloud-init sets the final `fiber-<last 8 characters of the serial number>` name with
  `manage_etc_hosts: true`; nothing on the device renames it afterwards.
- Progress without a login: the platform publishes an Avahi service (`_fiber._tcp`) whose TXT
  records carry the state and the step.
- Nothing ticked: no script is written, the FIBER gets a plain Raspberry Pi OS (as today).

## Keeping Ansible small

- `ansible-core` from Debian, nothing from Galaxy or pip. ansible-lint's `only-builtins` rule is
  on, and the collections path points at an empty directory in the release.
- The wrapper sets `LC_ALL=C.UTF-8` and `HOME=/root` (Ansible refuses to run without a UTF-8
  locale), `gathering = explicit`, `forks = 1`, and an apt lock timeout of ten minutes.
- No role `meta` dependencies: the binary resolves requirements from `components.yaml`.
- Every `command` task has `creates` or a real `changed_when`.
- If a role ever needs a module of its own, that component moves into the Rust binary instead.

## Not yet verified

- Time and memory of a full converge on a CM4, and of the whole stack on a 1 GB module.
- Whether a kernel update leaves a foreign `.dtbo` in `/boot/firmware/overlays/` alone.
