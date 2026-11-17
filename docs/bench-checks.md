# Bench checks

Six questions that documents cannot answer. Each takes minutes on a real FIBER and each removes a
risk from the plan, so they come before the work that depends on them (`TASKS.md`: T2 for checks
1 to 3 and 6, T22 for check 4, T3 for check 5).

**Status: none has been run yet.** Fill in the result and the date as they are answered.

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

Result: not built.

## 6. Is the rest of the board as the map says?

Quick looks while the FIBER is on the bench, each against [`hardware.md`](hardware.md):

| What | How | Result |
|---|---|---|
| The I2C bus | `i2cdetect -y 10` (after `dtparam=i2c_vc=on`) shows 0x18, 0x19 and 0x51. The schematic leaves the bus's pull-up resistors unfitted | not run |
| The backlight | `dtoverlay=pwm,pin=13,func=4`, then PWM channel 1 dims it | not run |
| The buttons | GPIO23, 24 and 25 read low while pressed | not run |
| The buzzer | GPIO17 low sounds it. Listen at power-on too: until `config.txt` sets the line, only the Raspberry Pi's weak pull-down holds it | not run |
