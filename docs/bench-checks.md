# Bench checks

Six questions that documents cannot answer. Each takes minutes on a real FIBER and each removes a
risk from the plan, so they come before the work that depends on them (`TASKS.md`: T2 for checks
1 to 3 and 6, T22 for check 4, T3 for check 5).

**Status: checks 5 (2026-11-17) and 1 (2026-11-19) are answered; checks 2, 4 and 6 in part (2026-11-19); 3 has not been run yet.** Fill in the
result and the date as they are answered.

What is needed: a FIBER-R2 rev 2.1 reachable over SSH, written by FIBER Setup with the FIBER
software turned off (a plain Raspberry Pi OS); for check 4 also a TOWER Core Module.

## 1. Do both ROM bootloaders answer from the CM4?

Everything about flashing and recovery rests on this. The schematic shows the four lines fitted
(BOOT0 and NRST of each chip, [`hardware.md`](hardware.md)); the authors of the earlier software
believed the north bridge had none and built a boot stub around that.

On the FIBER:

```sh
# once: enable the two UARTs, then restart
printf 'dtoverlay=uart3\ndtoverlay=uart4\n' | sudo tee -a /boot/firmware/config.txt
sudo reboot

# copy hil/bootloader_probe.py to the FIBER, then
sudo python3 bootloader_probe.py
```

The script raises BOOT0, pulses NRST, and asks each bootloader for its version, product id and
option bytes. It only reads; each chip restarts into its firmware afterwards.

| Chip | Expected | Result |
|---|---|---|
| North bridge | ACK, product id `0x497`, `nSWBOOT0` = 1 | **Yes** (2026-11-19): ACK, bootloader 3.3, product id `0x497`, option bytes `0x3FFFF0AA` (the factory value), `nSWBOOT0` = 1 |
| South bridge | ACK, product id `0x453`, and the option bytes of a production unit (`nBOOT_SEL`) | **Yes** (2026-11-19): ACK, bootloader 3.1, product id `0x453`, option bytes `0xFEFFFEAA`, `nBOOT_SEL` = 0 |

**Result (2026-11-19): the CM4 reaches both bootloaders.** One unit, FIBER-R2 rev 2.1 with a CM4
rev 1.1 (1 GB), on a plain Raspberry Pi OS of 2026-10-06 with kernel 6.18.60. Readout protection is
off on both chips. Neither chip sent anything on its own in 4 s before the probe.

The south bridge of this unit was not factory-fresh: its option bytes are those of the earlier
production script (`0xEEFFFEAA`) but for the reset pin's mode (`NRST_MODE` 3, the factory's
input and output, where the script set 1, input only). Bit 8 (`BOREN`) is 0: the brown-out reset is
off, as `hardware.md` warns about that script. `nBOOT_SEL` is already 0, so the new firmware's
one-time fix changes nothing on such a unit. Only one unit was at hand: what other production units
carry is not known.

If the south bridge does not answer on a unit whose firmware runs, its `nBOOT_SEL` bit is
probably still 1 (the BOOT0 pin is ignored): that unit can only be reached over SWD, and the new
firmware's one-time option-byte fix matters for every unit like it.

## 2. Does the LCD work on `/dev/spidev6.0`?

Add `dtoverlay=spi6-1cs` and check that `/dev/spidev6.0` appears and that the display accepts data
with chip select on GPIO18 (through the inverter). The earlier software drove GPIO12 by hand; on
rev 2.1 that link is not fitted. Also confirm that nothing objects to the overlay claiming GPIO19.

**Result (2026-11-19): half answered.** `/dev/spidev6.0` appears, GPIO18 becomes its chip select
and GPIO19 to 21 its pins (`SPI6_MISO`, `SPI6_MOSI`, `SPI6_SCLK`), and nothing in the kernel's log
objects. A test pattern was sent as an ST7920 takes it (SPI mode 3, 400 kHz, `0xF8` before a
command, `0xFA` before data, reset on GPIO16): four lines of text from the controller's own font
and a frame with a checkerboard. Since then the agent's driver sends its pages, without an error.
**Nobody has looked at the LCD yet.** What to look for: whether anything shows at all; whether the
text is upside down (the earlier software turned its picture by 180 degrees, and the agent does the
same, so its pages should be upright while the controller's own text of the test pattern would be
upside down).

## 3. Which level does the RAK5146 USB card need on mini-PCIe pin 22?

GPIO11 drives that pin. On the LTE modem it is PERST# (low = reset). RAK documents it as an
active-high reset on the SPI variants of the card; the USB variant is not documented. Until this is
known, nothing drives GPIO11 at boot.

Result: not run. The unit on the bench (2026-11-19) has no card in the slot: `lsusb` shows only
the root hub.

## 4. Radio: can the STM32WL55 exchange frames with a TOWER Core Module?

The spike that opens the north bridge work (T22). With a Core Module running TOWER's
`radio_beacon` example at the pinned revision:

1. Receive in raw mode (sync word `0xDB624715`, fixed length, no whitening, no CRC) and capture the
   bytes after the sync word. Settle the whitening sequence and the CRC start value; both are
   inferred from documents so far. Commit the captures as test vectors.
2. Transmit frames built in software back to the Core Module and read its frequency-offset report.
3. Measure the packet error rate at 117, 156 and 187 kHz receive bandwidth, at several signal
   levels.

**Result (2026-11-19): the radio is up; nothing heard from TOWER yet, as no Core Module was at
hand.** A bench firmware (`hil/radio-spike/`, plain registers), written to the north bridge
through its bootloader with `hil/north_flash.py` after a copy of its earlier firmware was read out
(256 KB, sha256 `1690ce2d...`, kept on the FIBER and on the maintainer's computer). On the radio,
from RM0453: the TCXO at 1.7 V and every calibration (the image one for 863 to 870 MHz) without a
device error; the radio sets `XOSC_START_ERR` at its start, before it is told about the TCXO, and
it stays clear once cleared. GFSK as TOWER sends it on 868.1 MHz (19 216.6 bit/s, 19.8 kHz
deviation, BT 1, sync word `0xDB624715`, a preamble detector of 16 bits), received raw (fixed 64
bytes, no whitening, no CRC); the RF switch to receive (PC3 to PC5 high); the radio stays in
receive. The core runs on MSI at 16 MHz, not on the TCXO.

The noise floor, as the radio's own RSSI reads it: 500 samples in each setting, the FIBER and its
antenna as they were on the bench (whether its cover was on and an antenna fitted was not looked
at). The average in dBm; the lowest reading was -127 dBm everywhere, the highest between -107 and
-80 dBm (short spikes, from something else on the band).

| Receive filter | LDO, normal gain | LDO, boosted | SMPS, normal | SMPS, boosted |
|---|---|---|---|---|
| 117.3 kHz | -116 | -118 | -116 | -117 |
| 156.2 kHz | -115 | -116 | -114 | -116 |
| 187.2 kHz | -114 | -116 | -114 | -116 |

The floor rises with the filter about as thermal noise does (1.25 dB and 0.8 dB from one to the
next), and is about what thermal noise and a receiver's usual noise figure give (-117 dBm at 117
kHz). The switching regulator adds nothing measurable; the boosted gain reads 1 to 2 dB lower. In
6 minutes no sync word was heard on channel 0. What this does not answer: the receiver's
sensitivity (the earlier firmware measured a weak one; that takes a known signal), and everything
that needs a Core Module (the whitening, the CRC, frames both ways, the error rates).

## 5. Does the south bridge firmware fit?

Needs no board: a skeleton firmware for the STM32C031C6 with the UART link, the ADC and the LED
engine, built at the release profile. The budget is 24 KB of the 32 KB.

**Result (2026-11-17): it fits.** The skeleton takes **15348 bytes (15.0 KB)** of the 24 KB
budget, which leaves about 9 KB for the real logic. Static RAM: 720 bytes of the 12 KB. It was
built and measured, not flashed: nothing of it has run on a FIBER.

What was built (a throw-away crate, T3, not kept in the repository):

- **Everything the real firmware links:** USART1 on PA9/PA10 at 115200 (`BufferedUart`); a COBS
  frame with a CRC-32 and a postcard message each way (`Desired` in, `Hello` and `Status` out);
  the ADC on PA11, PA12 and the internal reference; the 18 LED lines and the 8 port lines; a
  50 Hz `Ticker`; the independent watchdog; the 32.768 kHz crystal; two tasks.
- **And what T16 adds around it:** the CRC-32 over the firmware's own image, the reset cause, the
  option bytes read, and a fault code with an address kept across a reset (panic and HardFault).
- **The versions TOWER's firmware uses:** `embassy-stm32` 0.6.0 (`stm32c031c6`, `memory-x`,
  `time-driver-any`, `unstable-pac`), `embassy-executor` 0.10.0 (`platform-cortex-m`,
  `executor-thread`), `embassy-time` 0.5.1 (`tick-hz-1_000`), `embassy-sync` 0.8.0, `postcard`
  1.1.3, `cobs` 0.5.1; Rust 1.99.1.
- **The profile:** `opt-level = "z"`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`.
- **The linker:** `flip-link`, with `--nmagic` and `-Tlink.x`, as in TOWER's firmware.

| Section | Bytes |
|---|---|
| `.vector_table` | 180 |
| `.text` | 12316 |
| `.rodata` | 2732 |
| `.data` | 120 |
| Flash in all | 15348 |

What weighs most:

- **The two tasks, with everything inlined into them:** 4.6 KB (`cargo bloat` puts it under
  `embassy_executor`, but it is our code and the HAL's set-up).
- **The panic locations: 3.2 KB.** Every panic in the HAL carries its source file's full path
  (about 1.6 KB of strings) and its line. A panic handler that does not read the location lets
  the linker drop them all: 12140 bytes (11.9 KB). Shorter paths (`--remap-path-prefix`) give
  14296 bytes (14.0 KB).
- **64-bit division: 0.9 KB,** and it cannot be left out: the ADC's start-up delay in
  `embassy-stm32` (`block_for_us`) divides a 64-bit number. Taking our own one out
  (`Instant::as_secs`) saved 12 bytes.
- **`postcard`: 0.6 KB; `memcpy` and `memclr`: 0.9 KB.**
- **No floats and no formatting code** were linked (no `core::fmt`, no `__aeabi_f*`).

`opt-level = "s"` gives 16080 bytes (15.7 KB), so `"z"` stays.

**If it gets close:** drop the panic locations first (3.2 KB), and keep the HardFault's
address: its handler and the store take 82 bytes. Nothing else was tried.

**The firmware itself (T16, 2026-11-18):** 16224 bytes (15.8 KB), with the panic locations left
out and everything of the skeleton plus the core's LED engine, the frames of `fiber-protocol`, the
reset cause, the option bytes and the empty flag. `just south-size` checks it against the budget
in CI. Not run on a chip yet.

## 6. Is the rest of the board as the map says?

Quick looks while the FIBER is on the bench, each against [`hardware.md`](hardware.md):

| What | How | Result |
|---|---|---|
| The I2C bus | `i2cdetect -y 10` (after `dtparam=i2c_vc=on`) shows 0x18, 0x19 and 0x51. The schematic leaves the bus's pull-up resistors unfitted | **Yes** (2026-11-19): all three answer, at 97.5 kHz. The LIS2DH12 answers `0x33` to `WHO_AM_I`, the DS2482 reads `0x18` after a reset, the PCF85063A holds the right time in UTC with its oscillator flag clear. With the platform's block, 0x18 and 0x51 show `UU` (taken by their drivers) |
| The backlight | `dtoverlay=pwm,pin=13,func=4`, then PWM channel 1 dims it | Half (2026-11-19): `pwmchip0` with two channels; channel 1 on GPIO13 (`PWM0_1`) takes a period and a duty cycle, and the agent sets it to 80 %. Whether the backlight lights and dims: not seen |
| The buttons | GPIO23, 24 and 25 read low while pressed | Half (2026-11-19): all three read high, not pressed, against the CM4's own pull-downs, so the board's pull-ups are there. Pressed: not tried, nobody at the bench |
| The buzzer | GPIO17 low sounds it. Listen at power-on too: until `config.txt` sets the line, only the Raspberry Pi's weak pull-down holds it | Half (2026-11-19): on a plain Raspberry Pi OS the line reads high at start, against the CM4's pull-down, so the driver's pull-up keeps it quiet before `config.txt` sets it. Not heard, nobody at the bench |

More that the bench showed (2026-11-19), with the platform installed:

- **The clock is set from the PCF85063A at start,** by the kernel itself when the driver registers
  `rtc0` (19 s after the start; before that the time comes from `systemd-timesyncd`'s file). No
  `fake-hwclock` is installed. Seen across restarts; a power cycle without network: not tried.
- **The stock `config.txt`** of the image of 2026-10-06 is byte for byte the converge test's
  `provision/tests/fixtures/config.txt.stock`, and the file the `base` role wrote from it is byte
  for byte `config.txt.expected`. A kernel package installed again leaves `fiber-ds2482.dtbo` in
  place (its hook copies with `rsync` and deletes nothing).
- **Eight 1-Wire bus masters** appear with the DS2482 overlay. With no probe plugged in, which
  master is which port is not checked yet.
- **The south bridge's `VREFINT_CAL` is at 0x1FFF 756A,** not 0x1FFF 7568 as the firmware first
  read it (that is `TS_CAL1`, the temperature sensor's): 1662 and 1048 on this chip, read through
  its bootloader. The first firmware's voltages were 1.59 times too low.
- **The south bridge's pins follow `Desired`,** read over SWD with a J-Link while the firmware
  ran (the output registers of GPIOA, GPIOB and GPIOD): the ports, the LEDs set by the agent and
  by a client, and the fallback 5 s after the agent stopped (the LEDs dark but the power LED's
  yellow, briefly once a second; the ports as they were). Not seen by eye.
- **After SWD the CM4 cannot reach the south bridge's bootloader:** `fiber firmware chip south`
  got `0x00` where the bootloader's ACK should be, and the firmware started instead. It stays so
  until the next power-on (to be checked). Keep a debug probe off the south bridge on a FIBER
  whose firmware the agent should still be able to write.
- **The voltages** with that fixed: input 13.6 V, battery 1.7 V (no cell fitted, by the look of
  it). The input's readings spread over 0.13 V. Not checked with a meter.
