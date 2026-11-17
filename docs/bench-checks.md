# Bench checks

Six questions that documents cannot answer. Each takes minutes on a real FIBER and each removes a
risk from the plan, so they come before the work that depends on them (`TASKS.md`: T2 for checks
1 to 3 and 6, T22 for check 4, T3 for check 5).

**Status: check 5 is answered (2026-11-17); the others have not been run yet.** Fill in the
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
| North bridge | ACK, product id `0x497`, `nSWBOOT0` = 1 | not run |
| South bridge | ACK, product id `0x453`, and the option bytes of a production unit (`nBOOT_SEL`) | not run |

If the south bridge does not answer on a unit whose firmware runs, its `nBOOT_SEL` bit is
probably still 1 (the BOOT0 pin is ignored): that unit can only be reached over SWD, and the new
firmware's one-time option-byte fix matters for every unit like it.

## 2. Does the LCD work on `/dev/spidev6.0`?

Add `dtoverlay=spi6-1cs` and check that `/dev/spidev6.0` appears and that the display accepts data
with chip select on GPIO18 (through the inverter). The earlier software drove GPIO12 by hand; on
rev 2.1 that link is not fitted. Also confirm that nothing objects to the overlay claiming GPIO19.

Result: not run.

## 3. Which level does the RAK5146 USB card need on mini-PCIe pin 22?

GPIO11 drives that pin. On the LTE modem it is PERST# (low = reset). RAK documents it as an
active-high reset on the SPI variants of the card; the USB variant is not documented. Until this is
known, nothing drives GPIO11 at boot.

Result: not run.

## 4. Radio: can the STM32WL55 exchange frames with a TOWER Core Module?

The spike that opens the north bridge work (T22). With a Core Module running TOWER's
`radio_beacon` example at the pinned revision:

1. Receive in raw mode (sync word `0xDB624715`, fixed length, no whitening, no CRC) and capture the
   bytes after the sync word. Settle the whitening sequence and the CRC start value; both are
   inferred from documents so far. Commit the captures as test vectors.
2. Transmit frames built in software back to the Core Module and read its frequency-offset report.
3. Measure the packet error rate at 117, 156 and 187 kHz receive bandwidth, at several signal
   levels.

Result: not run.

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

## 6. Is the rest of the board as the map says?

Quick looks while the FIBER is on the bench, each against [`hardware.md`](hardware.md):

| What | How | Result |
|---|---|---|
| The I2C bus | `i2cdetect -y 10` (after `dtparam=i2c_vc=on`) shows 0x18, 0x19 and 0x51. The schematic leaves the bus's pull-up resistors unfitted | not run |
| The backlight | `dtoverlay=pwm,pin=13,func=4`, then PWM channel 1 dims it | not run |
| The buttons | GPIO23, 24 and 25 read low while pressed | not run |
| The buzzer | GPIO17 low sounds it. Listen at power-on too: until `config.txt` sets the line, only the Raspberry Pi's weak pull-down holds it | not run |
