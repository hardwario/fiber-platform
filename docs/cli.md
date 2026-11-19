# The `fiber` command line

`fiber` is the one program of the FIBER platform. This page describes its commands. What exists
so far: `fiber version`, `fiber doctor`, and the `fiber platform` commands that read and change
the choice of components, take the seed, apply the choice, and update, roll back and uninstall
the platform. `apply` and the rest come with their tasks (`TASKS.md`, T6 and on).

**Status:** tested on a computer, and in a bare Debian arm64 container in CI (no broker, no
hardware, no settings). On a FIBER (2026-11-19): `version`, `doctor`, `platform status`, `seed`
and `apply` (through the installer), and an update to the next release; not `rollback` and
`uninstall`.

## Exit statuses

Every command ends with one of these:

| Status | Meaning |
|---|---|
| 0 | Done |
| 1 | Failed; the message says why and, where it can, what to do |
| 2 | The command was not understood (a wrong or missing argument) |
| 3 | `fiber platform status` only: a component that is on or off has not been brought there yet |
| 10, 20 | `fiber platform apply` only: done but a restart is needed; partly done |

A message about a mistake goes to the standard error and starts with `error:`. A mistake in a
file names the file and the line, and shows the line.

## `fiber version`

Prints the version, on one line. FIBER Setup's first-start script records it as it is.

```
$ fiber version
0.1.0
```

`fiber version --all` adds what the build is made of: the TOWER revision its radio is compatible
with, and the schema tag of the TOWER radio messages.

## The board: the agent and the commands that talk to it

**Built on the drafts of T13, which the maintainer has yet to read (T17).** Tested in a container
against a simulated south bridge, through Mosquitto; flashing and `fiber firmware chip` are tested
against a simulated bootloader. On a FIBER (2026-11-19): `status`, `power`, `ports`, `leds`,
`display`, `buzzer beep`, `firmware status`, `firmware flash` and `firmware chip`; not `buttons
watch` itself (the buttons work, read by a Node-RED flow) and not `thermometers` with a probe. These commands, but for `fiber firmware
chip`, are clients of the agent's MQTT API ([`mqtt-api.md`](mqtt-api.md)): each one reads its
retained topics or sends a request on `fiber/cmd`, so whatever they do, a program of your own can
do too. They connect to
the broker on the FIBER (`127.0.0.1:1883`); `--broker HOST[:PORT]`, `--user` and the password in
`FIBER_MQTT_PASSWORD` reach another FIBER, through its listener for the network.

Their exit statuses, beside the ones at the top of this page:

| Status | Meaning |
|---|---|
| 4 | The agent does not answer: its `status` is `offline`, or not there |
| 125 | The agent speaks another schema of the API ([`mqtt-api.md`](mqtt-api.md), "The rules") |

### `fiber agent`

The agent itself: the one process that owns the board. `fiber-agent.service` runs it, as the user
`fiber` (the `agent` component, [`provisioning.md`](provisioning.md), "The agent"); it is not
meant to be started by hand while the service runs. It needs no settings; `/etc/fiber/agent.toml`
changes the defaults (the role writes it once, with each default in a comment):

```toml
[mqtt]
host = "127.0.0.1"   # the broker on the FIBER itself
port = 1883

[board]
dividers = true      # false on the board variant without the two voltage dividers
```

It keeps working while the broker is down, and sends every retained topic again when it is back.
When it cannot open the south bridge's port, it tries again every 5 s. It carries the south
bridge's firmware (`just build-fiber` builds it in) and writes it to the chip when the chip runs
another image, speaks another schema, or says nothing for 10 s after the start: at most three
tries an hour, kept in `/var/lib/fiber/agent/flash-tries.json` across restarts.

### `fiber status`

```
$ fiber status
FIBER       fiber-abcdef12, FIBER-R2 2.1, serial 10000000abcdef12
Agent       online, 0.2.0, schema 1
Power       PoE, input 12.08 V, battery 4.11 V
Ports       1 2 3 4 5 6 7 8 on
South       running 0.2.0, image 0x6f1c2a90 (the one carried)
North       running 0.2.0, image 0x1a2b3c4d (the one carried)
```

### `fiber power`

The power source and the two voltages, as `power` has them; "no battery" when `battery` is `null`
with the input known. `--follow` then prints a line on every change, and each `power/event`,
with the time in UTC, until Ctrl+C.

```
$ fiber power
Power       PoE, input 13.62 V, no battery
```

### `fiber ports [on|off|cycle] [PORT...] [--seconds N]`

Without an action, the power of every port. With one, it switches the ports named (1 to 8; all
when none is named) and prints them all after it. `cycle` switches them off and on again after
5 s, or `--seconds N` (1 to 60).

```
$ fiber ports off 3
Ports       1 2 4 5 6 7 8 on, 3 off
$ fiber ports cycle --seconds 3
Ports       1 2 3 4 5 6 7 8 on
```

### `fiber leds [auto | COLOUR [--blink ON_MS OFF_MS]] [LED...]`

Without an action, how each LED is set (`auto`, or the colour a client set). With a colour
(`off`, `green`, `red`, `amber`, `yellow`), it sets the LEDs named (`power`, `port1` to `port8`;
all port LEDs when none is named); `auto` gives them back to the agent.

### `fiber thermometers [watch]`

The DS18B20 probes on the ports, as `thermometers/...` has them, one line each, by port:

```
$ fiber thermometers
Port 3      28-0000034a5b1c  21.44 °C
Port 3      28-0000034a77e0  21.50 °C
Port 5      28-00000b1c2d3e  no reading
```

It prints `No thermometers.` when there are none, and says so when the 1-Wire buses were not
found. `watch` prints a line for every change, with the time in UTC, until Ctrl+C:

```
$ fiber thermometers watch
14:02:11  port 3  28-0000034a5b1c  21.44 °C
14:02:51  port 5  28-00000b1c2d3e  gone
```

### `fiber display [message TEXT [--seconds N] | clear | backlight LEVEL]`

Without an action, the backlight and the message, as `display` has them. `message` puts a message
on the display, over the pages (`\n` starts a new line); `clear` takes it away; `backlight` sets
the brightness, 0 to 100 %.

```
$ fiber display message "Back in\n5 minutes" --seconds 300
$ fiber display
Backlight   80 %
Message     Back in / 5 minutes
```

### `fiber buzzer beep [beep | double | long | alarm]`

Sounds the buzzer once, in one of its patterns (`beep` when none is named).

### `fiber buttons watch`

A line for each gesture of the three buttons, with the time in UTC, until Ctrl+C:

```
$ fiber buttons watch
14:02:11  enter  click
14:02:13  down  hold
14:02:13  down  repeat
```

### `fiber firmware [status | flash CHIP | chip CHIP]`

- `status` (the default): both chips, as `firmware/south` and `firmware/north` have them.
- `flash south` or `flash north`: writes the image the agent carries to the chip, through the
  agent (`firmware.flash`), and waits for it, up to 2 minutes.
- `chip south` or `chip north`: talks to the chip's ROM bootloader directly and prints what it
  is (product id, option bytes, readout protection), then starts its firmware again. It needs
  root, and it refuses while the agent holds the serial port: stop the agent first. It ends
  with 1 when the chip is not the one the board should have.

```
$ sudo systemctl stop fiber-agent
$ sudo fiber firmware chip south
Chip        product id 0x453 (the STM32C031 expected)
Bootloader  3.1, commands 00 01 02 11 21 31 44 63 73 82 92
Options     0xfffffeaa
Protection  off
$ sudo systemctl start fiber-agent
```

This is what the simulated bootloader answers; a real chip's answer has not been seen yet (T2).

## `fiber doctor`

Checks that the board's hardware is there as Linux should see it, and says what to do where it
is not. It needs no agent and changes nothing.

```
$ fiber doctor
ok    config.txt has the FIBER block
FAIL  config.txt, line 52: `dtparam=spi=on` turns on SPI0, which takes GPIO7 to 11: the south bridge's reset and serial link, and the card's lines
      remove line 52 from config.txt, then restart
ok    /dev/ttyAMA3 (the north bridge's serial link)
ok    /dev/ttyAMA4 (the south bridge's serial link)
ok    /dev/spidev6.0 (the LCD)
ok    /dev/rtc0 (the clock)
ok    the DS2482 1-Wire master at 0x18 on I2C bus 10 has its driver
ok    the PCF85063A clock at 0x51 on I2C bus 10 has its driver
ok    the LIS2DH12 accelerometer at 0x19 on I2C bus 10 answers
ok    8 1-Wire bus masters, one for each port
1 of 10 not as it should be.
```

What it looks at:

- **`config.txt`:** the lines that break this board (`dtparam=spi=on`, `dtoverlay=w1-gpio`,
  `ctsrts`, a `dtoverlay=pwm` without its pin), and whether the FIBER block is there.
- **The devices the block makes:** both serial links, the LCD's SPI device, the clock.
- **The I2C bus:** the 1-Wire master and the clock taken by their drivers, and the accelerometer
  answering (with `i2cget`, from `i2c-tools`; that needs root).
- **The eight 1-Wire bus masters,** one for each port.

Exit status 0 when everything is as it should be, 1 when not. Tested against prepared files,
and in a container for the `config.txt` part. On a FIBER with the platform (2026-11-19):
everything as it should be.

## `fiber platform`

The platform is a set of components, and one file, `/etc/fiber/platform.yaml`, says which of them
the FIBER brings up ([`provisioning.md`](provisioning.md)). For each component the file says
`true` (bring it up), `false` (stop it and turn it off; its data stays) or nothing (leave it
alone). A component that is on also brings up what it requires.

The components and what they require are fixed in the program: it carries
`provision/components.yaml` of its own release.

| Component | What it is | Requires |
|---|---|---|
| `base` | Basic system configuration: the `config.txt` block, the 1-Wire overlay, the groups | |
| `mosquitto` | Mosquitto, the MQTT broker | |
| `agent` | The FIBER agent | `base`, `mosquitto` |
| `node-red` | Node-RED | `mosquitto` |
| `chirpstack` | ChirpStack, a LoRaWAN network server | `mosquitto` |

### `fiber platform list`

Shows every component: on, off or left alone, and what it requires. A component that the file
does not name but that one which is on requires is shown as `on, for` that one.

```
$ fiber platform list
COMPONENT   STATE          REQUIRES         WHAT
base        on, for agent                   Basic system configuration
mosquitto   on, for agent                   Mosquitto
agent       on             base, mosquitto  FIBER agent
node-red    left alone     mosquitto        Node-RED
chirpstack  off            mosquitto        ChirpStack
```

Without `/etc/fiber/platform.yaml` every component is left alone, and the command says so.

### `fiber platform status`

Shows the release the program belongs to, whether an apply is waiting, and the last result of
each component. Exit status 0 when every component that is on or off has been brought there
and no apply is waiting; 3 when not.

```
$ fiber platform status
Release  1.2.0, in /opt/fiber/releases/1.2.0
Apply    nothing waiting

COMPONENT   STATE          LAST RESULT
base        on, for agent  ok, 2026-11-17T21:04:05Z
mosquitto   on, for agent  failed, 2026-11-17T21:04:05Z: no network
agent       on             not applied yet
node-red    off            ok, 2026-11-17T21:06:12Z
chirpstack  left alone     -
```

- **The release** is found from the program's own path (`/opt/fiber/releases/<version>/bin/`).
  A program started from anywhere else says it is not inside a release.
- **The last results** are in `/var/lib/fiber/platform/results.json`, written by `fiber
  platform apply`. A result counts only while it is for what the file wants now: a component
  turned on since its last apply is `not applied yet`.

### `fiber platform enable <component>...`

Turns one or more components on in `/etc/fiber/platform.yaml`, together with what they require.
It changes the file only; nothing is installed yet. It needs root.

```
$ sudo fiber platform enable node-red
mosquitto: on, node-red requires it
node-red: on
Changed /etc/fiber/platform.yaml.
```

A component that is on already is left as it is. Without a file, the command makes one, with the
settings at their defaults.

### `fiber platform disable <component>...`

Turns one or more components off. Their services are to be stopped and turned off; their data
stays. A component that another one needs is refused, with the other's name; turning them off
together works.

```
$ sudo fiber platform disable mosquitto
error: mosquitto is needed by agent and node-red, which are on. Turn them off with it: fiber platform disable agent node-red mosquitto
```

### `fiber platform apply`

Brings the FIBER to what `/etc/fiber/platform.yaml` says: installs, sets up and starts what is
on, stops and turns off what is off. It runs as the service `fiber-platform-apply.service`, so
it goes on when the SSH session drops; the command starts it and prints each step as it ends
([`provisioning.md`](provisioning.md), "How an apply runs"). It needs root.

```
$ sudo fiber platform apply
The apply runs as fiber-platform-apply.service; its log: /var/log/fiber/platform.log
common: up, nothing changed
chirpstack: off, nothing changed
mosquitto: up, 4 changes
Done.
```

| Exit status | Meaning |
|---|---|
| 0 | Every step ended `ok` |
| 10 | The same, and the FIBER needs a restart |
| 20 | Partly done: a step failed or was skipped. The service tries again by itself |
| 1 | The apply could not run (a mistake in the file, no release) |

`--foreground` does the work in this process instead of the service; the service runs that, and
so can a system without systemd.

### `fiber platform update [--to <version>]`

Installs a newer release with its own installer: there is one installer. Without `--to` it takes
the newest release. It fetches the release's `install.sh` and `SHA256SUMS`, checks the one
against the other, and runs the installer, which unpacks the release beside the current one,
moves `current` to it and applies. It needs root.

```
$ sudo fiber platform update
Updating from release 1.2.0 to 1.3.0, with its installer.
...
```

The check proves that the installer is whole, not who made it: both come from the same place,
GitHub's release page, over HTTPS. Its exit status is the installer's (0, 10, 20,
[`provisioning.md`](provisioning.md)). A release that is current already is left alone.

### `fiber platform rollback`

Goes back to the release before (`previous`), and applies. It brings back our program, our roles
and our units; it does not put packages from other sources or data back, and says so. The two
releases change places, so a second rollback goes forward again. Exit status: the apply's.

### `fiber platform uninstall --yes`

Takes the FIBER platform off the system: every component is turned off (each role's `disable`),
then every file of the footprint ([`provisioning.md`](provisioning.md)), `/etc/fiber` with the
login, `/var/lib/fiber`, `/var/log/fiber`, `/usr/local/bin/fiber` and `/opt/fiber` are removed.
The services' data and the users the roles made (`fiber-agent`, `nodered`) stay; the command lists the
data, and prints, without running them, the commands that would take the packages from other
sources, their data and the users off too. Without `--yes` it only says what it would do.

### `fiber platform seed <file>`

Takes the choice and the login from a seed file that FIBER Setup wrote
([`provisioning.md`](provisioning.md), "The seed"). The installer runs it; it needs root.

```
$ sudo fiber platform seed /boot/firmware/fiber-seed.yaml
Wrote /etc/fiber/platform.yaml.
Wrote /etc/fiber/secrets/login.json.
```

It writes each of the two only when it is not there yet, and says `Kept` otherwise: a
`platform.yaml` that exists is the person's, and the login is set once. A seed with a mistake in
it writes nothing. The seed itself is left alone; the installer deletes it.

### How `enable` and `disable` change the file

- **Only the lines under `components:` change.** A value is changed in place, and a new
  component goes on a new line after the last one. Everything else stays as it is: the
  settings, the order, blank lines, and comments, also the ones on a changed line. A file the
  commands make says so in its first line.
- **The components must be written one per line,** as in `  base: true`. In any other form (for
  example `components: {base: true}`) the commands refuse, and the file is to be changed by hand.
- **The new text is read back before it is written.** If it does not say exactly what was asked,
  nothing is written.
- **The file is replaced in one step:** the new text goes into a file beside it, which then takes
  its place. The owner and the permissions stay.
