# The south bridge's firmware

What runs on the south bridge (STM32C031C6): the status lights, the power of the 1-Wire ports and
two voltages, on the agent's word. The link it speaks is in [`south-link.md`](south-link.md); the
board's pins in [`../hardware.md`](../hardware.md).

**Status:** built, 16096 bytes (15.7 KB) of flash of the budget of 24 KB, and checked in CI. Not
run on a chip yet: there is no FIBER on the bench (T2, T16).

## Where it is

| Path | What |
|---|---|
| `crates/south-core` | The logic without the hardware: the LED engine, what a `Desired` does, the start and fallback patterns, the ports at start. Tested on the computer |
| `crates/protocol` | The frames and the messages, shared with the agent |
| `firmware/south` | The board: a workspace of its own, for `thumbv6m-none-eabi` |

```sh
just south          # build it
just south-size     # and measure it against the budget
just check-south    # fmt, clippy, rustdoc and the size, as CI runs them
```

It is built with `flip-link` (`cargo install flip-link`), which puts the stack below the static
memory, so an overflow faults at once (the chip has no MPU), and with the profile measured in
[`../bench-checks.md`](../bench-checks.md), check 5: `opt-level = "z"`, fat LTO, one codegen unit,
`panic = "abort"`.

## What it does

- **At start:** it sends a `Hello` (its version, the CRC-32 of its own image, its unique id, why it
  started, a fault kept from before the reset, its option bytes), blinks the power LED green, and
  switches the ports on one after another, 100 ms apart.
- **On a `Desired`:** every LED line and the ports, from the next tick on, and a `Status` back
  with the three raw ADC counts. On a `HelloRequest`: a `Hello`.
- **No `Desired` for 5 s:** the power LED blinks yellow, the port LEDs are dark, the ports stay.
- **50 ticks a second.** Lines that blink alike blink together: they count from the same tick.
- **The watchdog** (2 s) resets it when its loop stops.

## The board

- **USART1** on PA9 and PA10, 115200 baud, 8N1.
- **18 LED lines,** lit when low: PA0 and PA1 (the power LED, green and yellow), then green and red
  of ports 1 to 8 (PA2 to PA7, PB0, PB1, PB2, PB10 to PB15, PA8).
- **8 port lines,** on when high: PB9, PB8, PB7, PB6, PB5, PB4, PB3, PD3.
- **The ADC:** PA11 (the battery, 68 k / 68 k), PA12 (the 12 V rail, 330 k / 68 k) and the internal
  reference, with the longest sample time, for the dividers' resistance. Raw counts only: no
  floats in the firmware.
- **The 32.768 kHz crystal** (LSE).

## Faults

A panic or a hard fault is written into memory that a reset leaves alone (`.uninit`), and the chip
resets. The next `Hello` reports it, once: a panic as code 1, without its place (the places of
every panic in the HAL would cost 3.2 KB); a hard fault as code 2 with the address it happened at.

## The option bytes

The firmware changes one bit of them, once, and nothing else: where `nBOOT_SEL` is 1 (the factory
setting), BOOT0 is taken from an option bit, and once the flash is written the CM4's BOOT0 line
would no longer reach the bootloader. So it clears that bit and reloads the options, which resets
the chip; on a factory part the register then reads `0xDEFFE1AA`. A chip with readout protection
is left alone. It also clears the flash's "empty" flag at every start, so that a reset after a
first write starts the firmware, not the bootloader.

**Not tried on a chip yet.** A write of the option bytes that is cut short can leave the chip with
readout protection on and its bootloader out of reach, which only SWD can undo: try it on the
bench first, with a debug probe at hand (T2, T16).
