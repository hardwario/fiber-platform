# Provisioning

How a vanilla Raspberry Pi OS becomes a FIBER: what is installed, how the choice is stored, and how
it is applied again later.

**Status: mostly design** (`TASKS.md`, T4 to T12). Built so far: `provision/components.yaml`, and the
`fiber platform` commands that read and change `/etc/fiber/platform.yaml` ([`cli.md`](cli.md)).
Nothing installs anything yet. The package sources and versions below were checked against the
upstream documentation and repositories on 2026-11-17; check them again when each role is
written.

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
| `/etc/fiber/secrets/` | Root only: the seeded password, generated tokens |
| `/var/lib/fiber/` | State |
| `/var/lib/fiber/platform/results.json` | The last result of each component: what it was brought to (`on` or `off`), `ok`, `failed` or `skipped`, when, and why. Written by `apply` as each component finishes; read by `fiber platform status` |
| `/var/lib/fiber/platform/pending` | There while an apply is asked for and not finished |
| `/var/log/fiber/platform.log` | Every apply |
| `/run/fiber/platform/status.json` | Live progress: state, step, result per component, reboot needed |

**The footprint rule** stands in for a package manager's file list: every file written outside
`/opt/fiber`, `/etc/fiber` and `/var/lib/fiber` is named `fiber-*` or sits in a drop-in directory,
and is listed in `provision/footprint.yaml`. A CI test compares the filesystem before and after a
converge and fails on anything that neither a Debian package nor the footprint accounts for.

## Commands

`fiber platform list | status | apply | enable <component> | disable <component> | update [--to
version] | rollback | uninstall`. The ones that exist are described in [`cli.md`](cli.md).

- `apply` starts `fiber-platform-apply.service`, a oneshot with retry and backoff, and follows
  it. An apply therefore survives a dropped SSH session and a power cut, and two never run at once.
  The unit runs `ansible-playbook -c local` on the playbook of the release the binary itself
  belongs to, so a binary never runs another release's roles.
- Each component runs in its own block: one unreachable upstream does not stop the others, and
  the result is recorded per component. Exit status 0: converged; 10: converged, restart needed;
  20: partly (it retries by itself).
- `disable` stops and disables the services and keeps the data. Nothing is purged.
- `update` fetches the target release's `install.sh`, verifies it and runs it: there is one
  installer. The new release is unpacked beside the old one and `current` moves only on success.
- `rollback` restores our binary, roles and units. It does not downgrade third-party packages.

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

- Three phases: `install` (packages, files, enabling; safe in a chroot, so an image build can
  reuse it), `bootstrap` (needs the running service: tokens, passwords), `disable`.
- Configure a service before its first start. No service may ever answer with a default password.
- Drop-in files in place of edited package configuration.
- Credentials are seeded once and never re-asserted: a password changed by the owner stays changed.
- Version pins live in one file.
- `ansible.builtin` modules only.

### Sources (checked 2026-11-17)

Third-party repositories are written in deb822 form with a vendored armored key
(`Signed-By: /etc/apt/keyrings/fiber-<name>.asc`).

| Component | Source | Notes |
|---|---|---|
| Mosquitto | Debian: `mosquitto`, `mosquitto-clients` 2.0.21 | With no listener configured it serves port 1883 on loopback only, anonymously, which is the default wanted. `mqtt.lan: true` adds port 1884 with a login (`per_listener_settings true` must come first in the file). One drop-in in `/etc/mosquitto/conf.d`. |
| Node.js | NodeSource: `https://deb.nodesource.com/node_24.x`, suite `nodistro`. Key `https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key`, fingerprint `6F71F525282841EEDAF851B42F59B5F99B1BE0B4` | Debian's Node.js 20 is too old for Node-RED 5. An apt preference keeps `nodejs` on NodeSource. |
| Node-RED | `npm install -g --omit=dev node-red@<pinned>` (5.0.10 at the time) | The installer script the old FIBER documentation names is gone (its URL returns 404), and its successor is not idempotent. Own `nodered.service` as a system user `nodered`, port 1880, `adminAuth` written before the first start, a starter flow copied only if none exists. |
| ChirpStack | `https://artifacts.chirpstack.io/packages/4.x/deb`. Key `https://artifacts.chirpstack.io/packages/chirpstack.key`, fingerprint `E078226D4144665AD8793A601CE2AFD36DBCCA00`. Packages `chirpstack-sqlite`, `chirpstack-mqtt-forwarder`; Debian `redis-server` | **The package starts on install, on `0.0.0.0:8080` with admin/admin.** Install it with `policy_rc_d: 101`, then `set-password --password-file` and `create-api-key` before the first start. The repository keeps one version per package, so pin the major only. A region file is mandatory. |
| Concentrator daemon | Tarball `chirpstack-concentratord-sx1302_4.7.1_linux_arm64.tar.gz` from `artifacts.chirpstack.io/downloads`, sha256 `328faf7e6fe6fcc9f5a50f8368fd520807ac2939214c607593318e017e857345` | Model `rak_5146`, flag `USB`. Always installed with `chirpstack`, started by a udev rule when the card (USB `0483:5740`) appears: the card can show up long after boot. |
| Ansible | Debian: `ansible-core` 2.19 and `python3-debian`, installed with `--no-install-recommends` | Without that flag apt also installs the `ansible` collection bundle (205 MB). `python3-debian` is what the deb822 module needs. |

One credential for all of it: the user name and password typed in FIBER Setup also log in to
Node-RED, ChirpStack and the LAN MQTT listener. They are handed over once in the seed
file on the boot partition, moved to `/etc/fiber/secrets`, and removed from the boot partition.

## The `base` role's `config.txt` block

Overlay names and pins were checked against the Raspberry Pi kernel tree (`rpi-6.18.y`).

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
  chip's node on the `i2c_csi_dsi` bus, compiled in CI, copied to `/boot/firmware/overlays/`.
- The reset lines (GPIO7, 22) are inputs with pull-ups, so each microcontroller can still pull its
  own reset; the agent later requests them as open-drain outputs. The BOOT0 lines (GPIO26, 27) are
  held low. GPIO17 high keeps the buzzer silent.
- GPIO10 and 11 are left alone: the right level depends on the card in the mini-PCIe slot
  (`settings.hardware.minipcie`).
- A bare `dtoverlay=pwm` would take GPIO18, the LCD's chip select: the pin must be named.
- **Never** `dtparam=spi=on`, `dtoverlay=w1-gpio` or `ctsrts` on this board
  ([`hardware.md`](hardware.md)). `fiber doctor` fails when it finds one of them.
- The boot partition is FAT: files are copied without owner or mode, `config.txt` is rewritten
  only when it differs, with a backup, and synced.

## The release archive and the installer

- A release holds `fiber-platform-<version>-linux-arm64.tar.gz` (built reproducibly), `install.sh`
  stamped with the archive's sha256, `SHA256SUMS`, and a build-provenance attestation. Releases
  are immutable.
- `install.sh` is also the one-command path for someone who used Raspberry Pi Imager. It checks
  the machine and the system, downloads and verifies the archive, unpacks it beside older
  releases, installs Ansible, moves the seed file into `/etc/fiber` and starts the apply unit. It
  honours `FIBER_NO_REBOOT=1`.
- Through FIBER Setup the chain is: the signed desktop application pins the sha256 of
  `install.sh`, which pins the archive. With `curl | bash` the same check gives integrity, not
  authenticity; the documentation must say so. The provenance attestation is for people and CI: the
  device does not verify it.
- A weekly canary workflow converges the whole stack against the live upstream repositories and
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

- The unit name of `chirpstack-sqlite`.
- The path of Node-RED's bundled bcrypt for hashing the admin password.
- Time and memory of a full converge on a CM4, and of the whole stack on a 1 GB module.
- Whether a kernel update leaves a foreign `.dtbo` in `/boot/firmware/overlays/` alone.
