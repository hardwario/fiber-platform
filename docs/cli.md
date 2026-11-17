# The `fiber` command line

`fiber` is the one program of the FIBER platform. This page describes its commands. What exists
so far: `fiber version` and the `fiber platform` commands that read and change the choice of
components. `apply` and the rest come with their tasks (`TASKS.md`, T6 and on).

**Status:** tested on a computer, and in a bare Debian arm64 container in CI (no broker, no
hardware, no settings). Not run on a FIBER yet.

## Exit statuses

Every command ends with one of these:

| Status | Meaning |
|---|---|
| 0 | Done |
| 1 | Failed; the message says why and, where it can, what to do |
| 2 | The command was not understood (a wrong or missing argument) |
| 3 | `fiber platform status` only: a component that is on or off has not been brought there yet |

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
