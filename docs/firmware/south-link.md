# The south link

The serial link between the agent and the south bridge (STM32C031C6): what goes over the wire,
field by field, and when. The agent and the firmware both build these messages from one crate,
`crates/protocol` (`fiber-protocol`, T14), so the two cannot disagree on a field; a frame from a
peer built from another source is refused by its schema tag.

**Status: a draft for the maintainer to read (T13).** Nothing of it is built yet.

## The wire

- **USART1** of the south bridge (PA9 TX, PA10 RX) to `uart4` of the CM4 (`/dev/ttyAMA4`),
  **115200 baud, 8 data bits, no parity, 1 stop bit**, no flow control. The ROM bootloader uses
  the same pins with even parity ([`../hardware.md`](../hardware.md), "Flashing and recovery").
- **Frames, not bytes.** Every message is one frame: COBS, then one `0x00`. A receiver collects
  bytes up to a `0x00`, decodes the COBS, and drops what does not check out. It never answers a
  frame it dropped; it counts it (`Status.rx_dropped`).

### The frame

Inside the COBS:

| Bytes | Field | Meaning |
|---|---|---|
| 1 | `type` | Which message (below) |
| 2 | `tag` | The schema tag of the south link, little-endian |
| 2 | `seq` | The sender's frame counter, little-endian; it wraps at 65535 |
| n | `payload` | The message, postcard-encoded |
| 4 | `crc` | CRC-32/IEEE (as in zlib) of everything before it, little-endian |

At most 128 bytes inside the COBS. A receiver checks, in this order: the CRC, then the tag, then
the type, then that the payload is exactly one message with no byte left over. So a damaged
frame always counts as damaged, and a tag mismatch always means a whole frame from another
build.

**The schema tag** is worked out from the message types when `fiber-protocol` is built: a hash
of their shapes (field order, integer widths, variants), folded to 16 bits. It is ours alone and
does not move with the TOWER pin. A firmware and an agent from different builds refuse each
other's frames; the agent then flashes the firmware it carries (T17).

## The messages

| `type` | Message | Direction | When |
|---|---|---|---|
| `0x01` | `Hello` | south to agent | At start, and when asked |
| `0x02` | `Status` | south to agent | In answer to every `Desired` |
| `0x10` | `Desired` | agent to south | Every second |
| `0x11` | `HelloRequest` | agent to south | When the agent starts, or wants to know |

Postcard writes integers as varints and structs field by field, in this order. No message has a
growing collection: every array has its length fixed.

### `Hello`

Who the firmware is, and how it came to be running.

| Field | Type | Meaning |
|---|---|---|
| `version` | `[u8; 3]` | The firmware's version: major, minor, patch |
| `image_crc` | `u32` | CRC-32/IEEE of its own image in flash, as it works it out at start. The agent compares it with the image it carries |
| `uid` | `[u8; 12]` | The chip's unique id |
| `reset` | `Reset` | Why it last started: `PowerOn`, `Pin` (NRST, the agent's reset), `Watchdog`, `Software`, `BrownOut`, `Other` |
| `fault` | `Option<Fault>` | What made it reset, when it was a fault it caught: `code` (`u8`: 1 a panic, 2 a hard fault) and `address` (`u32`: where; 0 when not known) |
| `option_bytes` | `u32` | `FLASH_OPTR` as it reads at start |

The fault is kept across the reset in memory that a reset leaves alone, reported once, then
cleared.

### `Desired`

Everything the agent wants, every time: the firmware keeps no state of its own that the agent
has not sent. A lost `Desired` costs nothing; the next one puts it right.

| Field | Type | Meaning |
|---|---|---|
| `leds` | `[Light; 18]` | Each LED line, in the order below |
| `ports` | `u8` | The power of the 1-Wire ports: bit 0 is port 1, bit 7 port 8; 1 = on |

`Light` is one of:

| Variant | Fields | Meaning |
|---|---|---|
| `Off` | | Dark |
| `On` | | Lit |
| `Blink` | `on: u8`, `off: u8`, `phase: u8` | Lit for `on` ticks, dark for `off` ticks, starting `phase` ticks into the cycle. A tick is 20 ms (the firmware's 50 Hz) |

Lines with the same `on`, `off` and `phase` blink together: that is how a colour that needs two
lines (amber is green and red) blinks as one.

The 18 lines, as `docs/hardware.md` has them (all are lit when their pin is low; the firmware
hides that):

| Index | Line | Index | Line |
|---|---|---|---|
| 0 | Power LED, green | 1 | Power LED, yellow |
| 2 | Port 1, green | 3 | Port 1, red |
| 4 | Port 2, green | 5 | Port 2, red |
| 6 | Port 3, green | 7 | Port 3, red |
| 8 | Port 4, green | 9 | Port 4, red |
| 10 | Port 5, green | 11 | Port 5, red |
| 12 | Port 6, green | 13 | Port 6, red |
| 14 | Port 7, green | 15 | Port 7, red |
| 16 | Port 8, green | 17 | Port 8, red |

### `Status`

The answer to a `Desired`: what is applied, and the three voltages as raw counts.

| Field | Type | Meaning |
|---|---|---|
| `applied` | `u16` | The `seq` of the `Desired` now applied |
| `ports` | `u8` | The port power as it is now (as `Desired.ports`) |
| `vbat` | `u16` | ADC count on PA11: the battery through 68 k / 68 k |
| `vin` | `u16` | ADC count on PA12: the 12 V rail through 330 k / 68 k |
| `vref` | `u16` | ADC count of the internal reference |
| `uptime` | `u32` | Seconds since the firmware started |
| `rx_dropped` | `u16` | Frames dropped since start (damaged, another tag, another type, bytes left over); it wraps |

The counts are 12 bits. The agent turns them into volts (`vref` gives the supply), so the
firmware has no floats: battery = pin x 2, input = pin x 398 / 68. On the board variant without
the two dividers, `vbat` and `vin` mean nothing, and the agent says so.

### `HelloRequest`

No payload. The firmware answers with a `Hello`. The agent sends it when it starts, so it never
has to wait for the firmware to restart to learn who it is.

## The timing

- **At start** the firmware sends a `Hello`, shows a start pattern on the LEDs, and switches the
  eight ports on one after another, 100 ms apart, so that 1-Wire works without the agent and the
  inrush stays small.
- **The agent sends a `Desired` every second.** The firmware applies it at its next tick (within
  20 ms) and answers with a `Status`.
- **No `Desired` for 5 s:** the LEDs show the "agent down" pattern (the power LED blinks yellow,
  the port LEDs are dark). The port power stays as it was. The next `Desired` ends it.
- **The watchdog** resets the firmware when its loop stops; the next `Hello` says `Watchdog`.

## What is not on this link

Flashing (the ROM bootloader, [`../hardware.md`](../hardware.md)), and the option bytes, which
only the firmware itself writes, once, at its first start (T16).
