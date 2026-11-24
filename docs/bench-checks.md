# Bench checks

Six questions that documents cannot answer. Each takes minutes on a real FIBER and each removes a
risk from the plan, so they come before the work that depends on them (`TASKS.md`: T2 for checks
1 to 3 and 6, T22 for check 4, T3 for check 5).

**Status: checks 5 (2026-11-17), 1 and 2 (2026-11-19) are answered; checks 4 and 6 in part
(2026-11-19 to 21); 3 waits for a cell, without which the card has no power.** Fill in the result
and the date as they are answered.

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

**A J-Link on the south bridge's SWD header shuts the CM4 out** (seen 2026-11-19 and 2026-11-20):
the bootloader then answers the start byte with `0x00`, until a power cycle. On 2026-11-19 it
followed the J-Link's reads; on 2026-11-20 nothing had touched SWD since the last power cycle, the
J-Link was only wired to the header. Unplug it from the header before the CM4 writes the south
bridge. On 2026-11-21 the J-Link was taken off and the FIBER started without it: both
bootloaders answered (`fiber firmware chip south`, `chip north`). Not tried yet: a day of it.

**A write cut short recovers** (seen on this unit): the agent killed in the middle of the south
bridge's write (2026-11-19, page 22 of 64), and the FIBER's power cut in the middle of the north
bridge's (2026-11-21, about block 8 of 252). Each time the chip waited in its bootloader, and the
agent wrote it again at its next start: the north bridge 30 s after the agent started, when it had
stayed silent, and running 22 s later.

## 2. Does the LCD work on `/dev/spidev6.0`?

Add `dtoverlay=spi6-1cs` and check that `/dev/spidev6.0` appears and that the display accepts data
with chip select on GPIO18 (through the inverter). The earlier software drove GPIO12 by hand; on
rev 2.1 that link is not fitted. Also confirm that nothing objects to the overlay claiming GPIO19.

**Result (2026-11-19): yes, the LCD shows the agent's pages.** `/dev/spidev6.0` appears, GPIO18 becomes its chip select
and GPIO19 to 21 its pins (`SPI6_MISO`, `SPI6_MOSI`, `SPI6_SCLK`), and nothing in the kernel's log
objects. A test pattern was sent as an ST7920 takes it (SPI mode 3, 400 kHz, `0xF8` before a
command, `0xFA` before data, reset on GPIO16): four lines of text from the controller's own font
and a frame with a checkerboard. Since then the agent's driver sends its pages, without an error.
The maintainer looked: the pages show, so the bus, the mode, the speed and the packets are right.
Sent as drawn they are upside down: the panel is mounted upside down, and the agent turns the
picture by 180 degrees, as the earlier software did; turned, they are upright. (For one build the
turn was taken out, on a misreading of the first look.)

## 3. Which level does the RAK5146 USB card need on mini-PCIe pin 22?

GPIO11 drives that pin. On the LTE modem it is PERST# (low = reset). RAK documents it as an
active-high reset on the SPI variants of the card; the USB variant is not documented. Until this is
known, nothing drives GPIO11 at boot.

Result: not answered yet. A RAK5146 USB card in the slot (2026-11-19, evening), GPIO11 an input
with the CM4's pull-down (reads low), nothing on the USB-B socket (so the USB switch gives the bus
to the card): `lsusb` shows only the root hub. **The card has no power on this unit:** its 3.3 V
comes from its own converter (`U27`, TPS62933, at least 3.8 V in) whose input is `BAT+`, the
charger's battery node, and the unit has no cell (`fiber power`: input 13.62 V from PoE, no
battery). Pin 22 was not driven; next with a charged cell fitted.

2026-11-23, a second unit (`fiber-19cb4010`, serial `1000000019cb4010`) with a cell and a RAK5146
USB card, written by FIBER Setup with 1.0.1 and `hardware.minipcie: rak5146` (which drives nothing
yet), nothing on the USB-B socket. The cell read 3.75 to 3.76 V on PoE, charging; `U27` wants at
least 3.8 V. `lsusb` showed only the root hub in every case, each held 6 s: GPIO10 and 11 as found
(inputs with the pull-down, low); GPIO11 high; GPIO11 low for 200 ms, then high; GPIO10 high with
GPIO11 high, and with GPIO11 low (`pinctrl`, by hand). `config.txt` has `dtoverlay=dwc2,dr_mode=host`
and `otg_mode=1`. Both lines put back to inputs. Not answered: whether the card had power.

## 4. Radio: can the STM32WL55 exchange frames with a TOWER Core Module?

The spike that opens the north bridge work (T22). With a Core Module running TOWER's
`radio_beacon` example at the pinned revision:

1. Receive in raw mode (sync word `0xDB624715`, fixed length, no whitening, no CRC) and capture the
   bytes after the sync word. Settle the whitening sequence and the CRC start value; both are
   inferred from documents so far. Commit the captures as test vectors.
2. Transmit frames built in software back to the Core Module and read its frequency-offset report.
3. Measure the packet error rate at 117, 156 and 187 kHz receive bandwidth, at several signal
   levels.

**Result (2026-11-19): the north bridge hears a TOWER Core Module, and the air format is settled
from the captures.** The Core Module (r1) ran TOWER's `radio_beacon` at the pinned revision as the
node, on the maintainer's computer near the FIBER (the distance was not measured): a 16-byte frame every second on channel 0, `[seq:4 LE][0xA5 x
12]`. With TOWER's sync word in its register order (`DB 62 47 15`) nothing matched, while the
preamble detector fired once a second; without a sync word the radio handed over the 64 bytes after
each preamble, and in them:

- **The sync word is byte-reversed on air:** `15 47 62 DB` (the SPIRIT1 sends `SYNC1` first).
- **The whitening is PN9** (x^9 + x^5 + 1, from all ones), **least significant bit first** in each
  byte, over the length byte, the payload and the CRC. The radio's own whitening packs it the other
  way, as the documents said.
- **The CRC is CRC-16/CCITT-FALSE:** polynomial `0x1021`, from `0xFFFF`, not reflected, over the
  length byte and the payload, sent high byte first.
- **The length byte counts the payload only** (16).

All 56 strong frames of a 70-second capture agree, at -74 to -76 dBm (156 and 187.2 kHz filters,
the boosted gain): sequence numbers 142 to 198 with one missed. Weaker frames of something else on
the channel (-80 to -97 dBm) came too. The captures are kept as test vectors
(`crates/tower-air/tests/`).

**And the other way:** frames built on the north bridge from that description (`radio_beacon`'s
payload, the CRC, the whitening, a preamble of 32 bits and the sync word in its order on air), sent
raw from the low-power PA, are received by a second Core Module (an r2) running `radio_beacon` as
the sniffer at the pinned revision: its SPIRIT1 checks the CRC and passes them, with all 32 bits of
the sync word right (SQI 32) and an AFC correction of -4 to 0 (raw `AFC_CORR`, of +-127; TOWER does
not give its scale in Hz). The strength it reads follows the power asked of the PA:

| PA (RM0453's settings) | Received by the Core Module |
|---|---|
| +14 dBm | -88 dBm |
| +10 dBm | -91 dBm |
| about -3 dBm (the +10 dBm setting asked for 0 dBm) | -103 to -106 dBm; a few frames missed at the edge |

The RF switch for the low-power PA (PC3 and PC4 high, PC5 low) is right: the earlier firmware had
it wrong until 2026-09-26 and lost 26 dB to it. **The FIBER's antenna was disconnected** through
all of these measurements (the maintainer, 2026-11-19), and every level on this page is without
it, except the next paragraph. How far the Core Modules were from the FIBER was not measured.

**With the antenna on** (2026-11-19, the same afternoon): an external antenna on the north bridge's
jack, its tip 15 cm from the Core Module r2 (the maintainer). The spike's settings were checked
against ST's own code for the module first (the RF switch, `HAL_RADIO_SetSwitchConfig`; the
switching regulator, `HAL_RADIO_IsDCDC`), and the board against the module's datasheet
(`docs/firmware/radio.md`, "Good to know"): they match. The levels, each regulator run twice, back
to back:

| | Linear regulator | Switching regulator | Another 868 MHz antenna, switching | Antenna off (above; distance not measured) |
|---|---|---|---|---|
| North bridge sends +14 dBm | -80 to -83 dBm | -80 to -81 dBm | -71 to -74 dBm | -88 dBm |
| North bridge sends +10 dBm | -83 to -85 dBm | -83 to -84 dBm | -74 to -77 dBm | -91 dBm |
| North bridge sends about -3 dBm | -97 to -99 dBm | -97 dBm | -87 to -90 dBm | -103 to -106 dBm |
| North bridge receives the Core Module (`net_confirmed`) | -71 to -78 dBm | -72 to -76 dBm | -64 to -68 dBm | -75 dBm |

The regulator makes no difference. The second antenna (the maintainer swapped it, the same place)
gives 8 to 9 dB more in both directions: so the antenna is in the radio's path, and the switch and
the path to the jack work. Whether about 80 dB between the two radios at 15 cm is what this pair
should give is not settled: the Core Module's chip antenna, on a board with its USB cables, may
well be the weaker side, and the two antennas' orientation counts. A known reference is still
missing (two Core Modules at the same distance, or the level against distance). Earlier the same
afternoon the levels were 10 dB lower still in both directions (-85 dBm received, -91 to -95 dBm
sent at +10 and +14 dBm); the back-to-back runs rule the regulator out, so something on the bench
moved.

**Against a second Core Module** (2026-11-19, evening): two Core Modules r2 on the maintainer's
computer, 10 cm apart, both about 20 cm from the FIBER's antenna (the second one tried). TOWER
sends at about +12 dBm (its `firmware/src/radio/config.rs`).

| From | To | Received | Lost on the way |
|---|---|---|---|
| Core Module B (its own node application, 12 frames) | Core Module A, 10 cm | -49 dBm | about 61 dB |
| Core Module B, the same 12 frames | the north bridge, 20 cm | -66 to -68 dBm | about 79 dB |
| Core Module A (`net_confirmed`) | the north bridge, 20 cm | -73 dBm (most of 36) | about 85 dB |
| the north bridge, +14 dBm | Core Module A, 20 cm | -81 dBm | 95 dB |
| the north bridge, +10 dBm | Core Module A, 20 cm | -84 dBm | 94 dB |

- **The receiver is fine:** the same frames reach the north bridge 17 to 19 dB below a Core Module
  at half the distance, which is about what twice the distance costs this close to the antennas
  (both within half a wavelength, 17 cm, where the field falls faster than in the open). Its
  readings are trustworthy: the noise floor reads what thermal noise gives (above).
- **The RF switch is right, tried:** frames at +10 dBm through each of the 8 states of PC3, PC4
  and PC5 (the spike's switch sweep, 6 rounds): only PC4 high with PC5 low gets them out, -84
  dBm with PC3 high or low; with PC4 low and PC5 high (ST's state for the high-power PA) one of
  6 was heard at -103 dBm; the other 5 states nothing, or one at -104 dBm. ST's state for the
  low-power PA (PC3 and PC4 high, PC5 low) is the spike's.
- **Sending is 10 dB short of receiving on the same pair** (A both ways: 95 against 85 dB), and
  the two directions should lose the same. The PA follows its settings (+10 to +14 dBm gives 3
  dB, -3 to +10 dBm gives 13 dB), so it is a fixed loss, not a PA that saturates. What is not
  known: how far the SPIRIT1's RSSI is from the truth (TOWER takes the datasheet's `raw/2 - 130`)
  and what a Core Module really sends. Settling it needs a calibrated receiver: a spectrum
  analyser or a power meter on the north bridge's jack through an attenuator. The two Core
  Modules differ too: B reaches the north bridge 6 dB stronger than A.

Not done yet: the error rates at -60, -90, -100 and -105 dBm (they need attenuators), more than
one Core Module, and the frequency offset in Hz.

Before the Core Module (the same day): the radio up, the noise floor, and no TOWER traffic in the
office. A bench firmware (`hil/radio-spike/`, plain registers), written to the north bridge
through its bootloader with `hil/north_flash.py` after a copy of its earlier firmware was read out
(256 KB, sha256 `1690ce2d...`, kept on the FIBER and on the maintainer's computer). On the radio,
from RM0453: the TCXO at 1.7 V and every calibration (the image one for 863 to 870 MHz) without a
device error; the radio sets `XOSC_START_ERR` at its start, before it is told about the TCXO, and
it stays clear once cleared. GFSK as TOWER sends it on 868.1 MHz (19 216.6 bit/s, 19.8 kHz
deviation, BT 1, sync word `0xDB624715`, a preamble detector of 16 bits), received raw (fixed 64
bytes, no whitening, no CRC); the RF switch to receive (PC3 to PC5 high); the radio stays in
receive. The core runs on MSI at 16 MHz, not on the TCXO.

The noise floor, as the radio's own RSSI reads it: 500 samples in each setting, with the FIBER's
antenna disconnected (so this is the receiver's own floor, with little from outside; whether the
cover was on was not looked at). The average in dBm; the lowest reading was -127 dBm everywhere, the highest between -107 and
-80 dBm (short spikes, from something else on the band).

| Receive filter | LDO, normal gain | LDO, boosted | SMPS, normal | SMPS, boosted |
|---|---|---|---|---|
| 117.3 kHz | -116 | -118 | -116 | -117 |
| 156.2 kHz | -115 | -116 | -114 | -116 |
| 187.2 kHz | -114 | -116 | -114 | -116 |

The floor rises with the filter about as thermal noise does (1.25 dB and 0.8 dB from one to the
next), and is about what thermal noise and a receiver's usual noise figure give (-117 dBm at 117
kHz). The switching regulator adds nothing measurable; the boosted gain reads 1 to 2 dB lower. A
second survey, after a reset, agreed within 1 dB. Then the radio listened on TOWER's three channels
in turn (868.1, 868.3 and 868.5 MHz, 156.2 kHz, boosted gain), about 35 s each, three rounds: no
sync word on any. On 868.1 and 868.5 MHz short spikes up to -81 dBm and a few preamble detections
(up to 13 in 35 s); on 868.3 MHz neither, the highest reading -108 dBm. 868.1, 868.3 and 868.5 MHz
are also LoRaWAN's three default channels in EU 868: the spikes are probably LoRa traffic nearby
(not checked). The preamble detector of 16 bits also fires on noise now and then. What this does not answer: the receiver's
sensitivity (the earlier firmware measured a weak one; that takes a known signal), and everything
that needed a Core Module (answered above, but frames sent the other way and the error rates).

**Result (2026-11-20): the FIBER is a TOWER gateway.** The north bridge's firmware
(`firmware/north`, written by the agent) and two Core Modules r2 running TOWER's `radio_button` at
the pinned revision, 10 cm apart and about 20 cm from the FIBER's antenna: both paired over the air
(the code on the FIBER's display and in the node's log), their clicks arrived on MQTT at -65 to
-70 dBm, a `set` reached a node after its next uplink, and the nodes' counters went on across a
power cycle of the FIBER and restarts of the nodes. At the first try: 19 of 20 clicks before a fix
of the north bridge's firmware (a packet still arriving at the end of a 100 ms receive slice was
cut off), then 40 of 40 from one node and 15 of 15 from each of two alternating. Not tried yet: a
day of it (T27), a replay sent on air, other distances.

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
in CI. Runs on a FIBER since 2026-11-19 (T16).

## 6. Is the rest of the board as the map says?

Quick looks while the FIBER is on the bench, each against [`hardware.md`](hardware.md):

| What | How | Result |
|---|---|---|
| The I2C bus | `i2cdetect -y 10` (after `dtparam=i2c_vc=on`) shows 0x18, 0x19 and 0x51. The schematic leaves the bus's pull-up resistors unfitted | **Yes** (2026-11-19): all three answer, at 97.5 kHz. The LIS2DH12 answers `0x33` to `WHO_AM_I`, the DS2482 reads `0x18` after a reset, the PCF85063A holds the right time in UTC with its oscillator flag clear. With the platform's block, 0x18 and 0x51 show `UU` (taken by their drivers) |
| The backlight | `dtoverlay=pwm,pin=13,func=4`, then PWM channel 1 dims it | **Yes** (2026-11-19): `pwmchip0` with two channels; channel 1 on GPIO13 (`PWM0_1`), set to 80 % by the agent, and the backlight lights (the maintainer looked), with `dtparam=audio=on` still in the file; it dims after a minute without a press |
| The buttons | GPIO23, 24 and 25 read low while pressed | **Yes** (2026-11-19): all three read high, not pressed, against the CM4's own pull-downs, so the board's pull-ups are there; pressed, the agent's pages follow them (the maintainer) |
| The buzzer | GPIO17 low sounds it. Listen at power-on too: until `config.txt` sets the line, only the Raspberry Pi's weak pull-down holds it | **Yes** (2026-11-19): it sounds (the maintainer heard the agent's beeps). On a plain Raspberry Pi OS the line reads high at start, against the CM4's pull-down, so the driver's pull-up keeps it quiet before `config.txt` sets it: through a power cycle it stayed silent (the maintainer listened) |

More that the bench showed (2026-11-19), with the platform installed:

- **The clock is set from the PCF85063A at start,** by the kernel itself when the driver registers
  `rtc0` (19 s after the start; before that the time comes from `systemd-timesyncd`'s file). No
  `fake-hwclock` is installed. Seen across restarts, and across a power cycle of about a minute
  with NTP off: set from the RTC 6.7 s into the start, within a second of the right time.
- **The stock `config.txt`** of the image of 2026-10-06 is byte for byte the converge test's
  `provision/tests/fixtures/config.txt.stock`, and the file the `base` role wrote from it is byte
  for byte `config.txt.expected`. A kernel package installed again leaves `fiber-ds2482.dtbo` in
  place (its hook copies with `rsync` and deletes nothing).
- **Eight 1-Wire bus masters** appear with the DS2482 overlay. Master k is port k: a probe on each
  of the eight ports in turn showed on that port alone (2026-11-21; ports 1, 2 and 5 by switching
  a port off, 2026-11-19). A probe moved to another port shows there only after Linux drops it
  from the old one, 100 to 115 s after the pull ([`hardware.md`](hardware.md)).
- **The south bridge's `VREFINT_CAL` is at 0x1FFF 756A,** not 0x1FFF 7568 as the firmware first
  read it (that is `TS_CAL1`, the temperature sensor's): 1662 and 1048 on this chip, read through
  its bootloader. The first firmware's voltages were 1.59 times too low.
- **The south bridge's pins follow `Desired`,** read over SWD with a J-Link while the firmware
  ran (the output registers of GPIOA, GPIOB and GPIOD): the ports, the LEDs set by the agent and
  by a client, and the fallback 5 s after the agent stopped (the LEDs dark but the power LED's
  yellow, briefly once a second; the ports as they were). By eye (2026-11-21, the maintainer): two
  ports' LEDs red while their probes had no readings, then green; the fallback not looked at.
- **After SWD the CM4 cannot reach the south bridge's bootloader:** `fiber firmware chip south`
  got `0x00` where the bootloader's ACK should be, and the firmware started instead. It stays so
  until the next power-on: after a power cycle the bootloader answered again, and the agent wrote
  the south bridge's next image at its first try. Keep a debug probe off the south bridge on a
  FIBER whose firmware the agent should still be able to write.
- **The voltages** with that fixed: input 13.6 V, battery 1.7 V (no cell fitted, by the look of
  it). The input's readings spread over 0.13 V. Not checked with a meter.
