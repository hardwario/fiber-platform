# The radio: TOWER on the north bridge

How the north bridge (an STM32WL55, its sub-GHz radio) speaks to TOWER nodes, whose radio is a
SPIRIT1. What a TOWER node sends on air, as the bench showed it, and how the STM32WL is set to
send and receive the same. The layers around it are in [`../architecture.md`](../architecture.md),
"North bridge"; the measurements in [`../bench-checks.md`](../bench-checks.md), check 4.

**Status:** settled on the bench (2026-11-19, T22) with the radio spike (`hil/radio-spike`) and
two TOWER Core Modules at the pinned revision running `radio_beacon`: frames go both ways. The
settings below are the spike's. The packet is made and checked by `fiber-tower-air`'s `packet`,
the commands by `fiber-subghz` (T23), and the spike runs on both: through them it received every
frame of a Core Module (20 in 20 s, none missed) and a Core Module received every frame it sent
(14 of 14), on 2026-11-19. The north bridge's firmware (T25) is not written yet. With the
FIBER's antenna fitted the levels rise, and depend on the antenna (see "Good to know"); the
settings were checked against ST's own code for the module and match it. Not measured yet: the error rates at several
levels, the frequency offset in Hz.

## What a TOWER node sends

TOWER's radio settings are in its `firmware/src/radio/config.rs`; what they produce on air was read
from 56 frames of `radio_beacon` (kept as test vectors in `crates/tower-air/tests/`):

| | On air |
|---|---|
| Channels (EU 868) | 868.1, 868.3 and 868.5 MHz (channels 0 to 2); a node and its gateway share one |
| Modulation | GFSK, BT 1, 19 216.6 bit/s (what a SPIRIT1 makes of 19 200), 19.8 kHz deviation |
| Preamble | 4 bytes, `0x55` alternating |
| Sync word | `15 47 62 DB`: TOWER's `SYNC4..SYNC1` (`0xDB624715`) in reverse, as the SPIRIT1 sends `SYNC1` first |
| Length | one byte: the payload's length, without itself and without the CRC |
| Payload | TOWER's network frame (its `docs/firmware/radio.md`); the length byte allows up to 255 bytes, TOWER's own limit is lower |
| CRC | CRC-16/CCITT-FALSE: polynomial `0x1021`, from `0xFFFF`, not reflected, nothing XORed at the end; over the length byte and the payload; high byte first |
| Whitening | PN9 (x^9 + x^5 + 1) from all ones, each byte filled from its least significant bit; over the length byte, the payload and the CRC |

The STM32WL's own whitening fills each byte from its most significant bit, so it cannot make or
undo this; its own CRC engine is not used either. The radio therefore runs raw: a fixed length and
the sync word, no whitening, no CRC, and the packet is built and checked in software, by one pair
of pure functions (`fiber-tower-air`'s `packet`).

**Receiving a packet of any length** with a fixed length set: the radio is set for the longest
(255 bytes) and listens for the sync word. Two byte times (834 us) after it, the first byte is in
the radio's buffer, and it says how long the packet is (`packet::packet_len`); one byte time after
the packet's last byte the radio is put in standby, the packet read from the buffer and checked
(`packet::decode`), and it listens again. So it is ready for the next packet as soon as this one
ends, and not 106 ms later. Seen working on the bench (`hil/radio-spike`, receive mode).

## How the STM32WL is set

From RM0453 (the STM32WL reference manual, section 5), as the spike sets it:

| What | Command | Value |
|---|---|---|
| The TCXO (inside the module) | `SetTcxoMode` | 1.7 V, 1 ms to settle; then `Calibrate` (all) and `CalibrateImage` for 863 to 870 MHz |
| Regulator | `SetRegulatorMode` | the switching regulator, as ST's code for the module (`HAL_RADIO_IsDCDC`) |
| Packet type | `SetPacketType` | GFSK |
| Frequency | `SetRfFrequency` | f x 2^25 / 32 MHz |
| Modulation | `SetModulationParams` | bit rate 53 287 (32 x 32 MHz / 19 216.6, rounded; the spike's 53 286 was a slip of 0.002 %), BT 1 (`0x0B`), receive filter 117.3, 156.2 or 187.2 kHz (`0x0B`, `0x1A`, `0x12`), deviation 20 762 (19.8 kHz x 2^25 / 32 MHz, rounded) |
| Packet | `SetPacketParams` | a preamble of 32 bits, a detector of 16 bits, a sync word of 32 bits, no address, fixed length, no CRC, no whitening |
| Sync word | registers `0x06C0` | `15 47 62 DB` |
| Receive gain | register `0x08AC` | `0x96` (boosted) |
| Transmit | `SetPaConfig`, `SetTxParams` | the low-power PA: `04 00 01 01` and 14 for +14 dBm, `01 00 01 01` and 13 for +10 dBm; a ramp of 200 us |
| RF switch | PC3, PC4, PC5 | receive: all high; transmit on the low-power PA: PC3 and PC4 high, PC5 low; off: all low (ST's HAL for the module, `HAL_RADIO_SetSwitchConfig`) |

Good to know:

- **The radio sets `XOSC_START_ERR` at its start,** before it is told about the TCXO; clear the
  device errors after the calibrations, and any error then is a real one.
- **The high-power PA is not wired** on this module's board: +14 dBm from the low-power PA is the
  most, and the limit of the 868.0 to 868.6 MHz band (25 mW, 1 % of the time).
- **The switching regulator** made no measurable difference to the noise floor (the boosted gain
  read 1 to 2 dB lower), nor to the levels sent and received: within 1 dB of the linear one,
  both ways, run back to back (2026-11-19).
- **The board wires the module for the low-power PA,** as the module's datasheet (DS14084,
  "Low output power configuration") shows it: `VDDPA` on `VDDRF1V55`, `VR_PA` on `BIAS_LP`,
  `BIAS_HP` to a test point. The module's `ANT` goes through `R63` (0 ohm) to the antenna jack
  `JP16`; the two shunt places beside it (`C56`, `C57`) are not fitted. The module's pin 27,
  `SW_CTL`, is not connected: the datasheet gives it no function beyond its name, and answers on
  ST's forum disagree on it (one, from a community member, says it is PC4, the switch's control
  line brought out). ST's board schematic for the module (MB1779) was not read yet.
- **The antenna is in the path** (the bench unit, 2026-11-19): with its tip 15 cm from a Core
  Module, +14 dBm arrived at -81 dBm and the Core Module's frames at -73 dBm with one 868 MHz
  antenna, and 8 to 9 dB more both ways with another (-71 to -74 dBm, -64 to -68 dBm). Whether
  that is as strong as it should be needs a reference (two Core Modules at the same distance);
  see [`../bench-checks.md`](../bench-checks.md), check 4.
- **No frequency correction:** this radio does not follow the sender's offset. The receive filter
  has to pass both crystals' error: TOWER's own receiver uses about 216 kHz. Between the north
  bridge (a TCXO) and one Core Module the SPIRIT1's `AFC_CORR` read -4 to 0, a small offset.
- **No link quality numbers:** this radio gives the signal strength of a packet (and of the
  channel) and nothing like the SPIRIT1's PQI and SQI, so the north bridge reports the strength
  alone.
