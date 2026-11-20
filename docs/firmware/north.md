# The north bridge's firmware

What runs on the north bridge (STM32WL55, its Cortex-M4 core): the FIBER's TOWER gateway on the
radio, on the agent's word. The link it speaks is in [`north-link.md`](north-link.md); what it
does on air in [`radio.md`](radio.md); the board's pins in [`../hardware.md`](../hardware.md).

**Status:** built (62.8 KB of flash, 4.8 KB of static memory) and checked in CI (T25). Not run on
a FIBER yet: it has not said `Hello` to anyone.

## Where it is

| Path | What |
|---|---|
| `crates/tower-air` | The gateway without the hardware: TOWER's packet, its frame, the gateway's flow and pairing over a radio trait. Tested on the computer |
| `crates/subghz` | The radio's commands |
| `crates/protocol` | The frames and the messages of the link, shared with the agent |
| `firmware/north` | The board: a workspace of its own, for `thumbv7em-none-eabi` |

```sh
just north          # build it
just check-north    # fmt, clippy, rustdoc and the TOWER pin, as CI runs them
```

Like the south bridge's, it is built with `flip-link`, fat LTO, one codegen unit and
`panic = "abort"`; `opt-level = "s"`, as the flash has room (256 KB) and pairing's arithmetic
wants the speed.

## What it does

- **At start:** a `Hello` (its version, the CRC-32 of its own image, its unique id, why it
  started, a fault kept from before the reset, its personalities, the counter's floor), then it
  waits, silent on the radio, for `Settings`.
- **The first `Settings`** makes the gateway at that address, raises the transmit counter to the
  agent's floor, and starts the radio. The address is then fixed until the next reset.
- **The loop:** the agent's messages, a `Status` every 10 s, then 100 ms of the radio: uplinks
  passed on as they come, and what waits for a node sent after its uplink. While a pairing window
  is open, 500 ms of listening for nodes that want to join instead.
- **One personality,** `TowerFsk` (D4); `SetPersonality` is there for the next.
- **The watchdog** (8 s) resets it when its loop stops: longer than the longest turn, a pairing
  slice with a whole handshake (under 5 s).
- **A fault** (a panic, a hard fault) is kept across the reset in memory a reset leaves alone, and
  reported once in the next `Hello`, as on the south bridge.

## The board

- **The clock:** 48 MHz from the TCXO (32 MHz, the HSE in bypass, which powers it; the PLL / 2
  x 6 / 2). The random number generator from the PLL's Q output, also 48 MHz.
- **USART1** on PA9 and PA10, 115200 baud, 8N1, to the CM4's `uart3`.
- **The radio:** the SUBGHZSPI at 12 MHz, its busy line and chip select through the power
  controller; the RF switch on PC3, PC4 and PC5 (as ST's HAL for the module sets it); the TCXO at
  1.7 V for the radio; the switching regulator; the settings of [`radio.md`](radio.md).
- **Embassy** for the STM32WL55's M4 core, started with `init_primary`. The second core, the
  Cortex-M0+, never runs; the data the two would share is an ordinary static.
- **The "empty" flag** is cleared at every start, as on the south bridge, so that a reset after a
  first write through the bootloader starts the firmware.

## The watermark

The one thing it keeps (D8): every transmit counter it has used is below it. In the last two
pages of the flash, 0x0803F000 and 0x0803F800, which `memory.x` leaves out of the image (the
image may take 252 KB):

- **A record is 8 bytes,** the flash's smallest write: the value and its complement. A record
  whose halves do not match was cut short by a reset and counts for nothing.
- **Records are appended in one page;** when it is full, the other page is erased and written
  next, so one of the two always holds the last value. The highest record of the two is the
  watermark (the counter only grows).
- **Each value is read back** after it is written. When that fails the gateway sends nothing more:
  a counter used twice under one key would break the encryption. One record covers 1024 counters,
  so a page lasts 262 144 frames sent, and the flash's 10 000 erases a page far longer than the
  product.

## Not done yet

- On a FIBER: `Hello`, `Settings`, a Core Module's frames taken in (T25's "Done when").
- The agent carries this firmware (`just build-fiber` builds it in) and writes it when the north
  bridge runs another image or says nothing (T26); not run on a FIBER yet.
