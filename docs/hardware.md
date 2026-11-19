# FIBER hardware map

What the software needs to know about the board: which chip is where, which pin does what, and
which of it has been confirmed on a real unit.

**Source.** The EAGLE schematic of **FIBER-R2 revision 2.1** (`hio-fiber-r2`), the display board
(FIBER DISPLAY R1.1) and the cover board (FIBER-COVER 1.2), read net by net. The south bridge's pin
table also agrees with the pin table of the earlier Zephyr firmware. Nothing in this file has been
measured on a board yet: [`bench-checks.md`](bench-checks.md) lists what must be confirmed first.

**Scope.** Revision 2.1 only. Revision 1.3 (`hio-fiber`) is a different design (see the end).

The one place in the code that may contain a pin number is the agent's board module; this file is
its reference.

## Blocks

| Block | Part | Notes |
|---|---|---|
| Compute module | Raspberry Pi CM4 (the schematic fits CM4101008: 1 GB RAM, 8 GB eMMC, Wi-Fi) | Runs Raspberry Pi OS |
| South bridge | **STM32C031C6T6** (Cortex-M0+, 32 KB flash, 12 KB RAM, no EEPROM), LQFP-48 | Status LEDs, 1-Wire port power, two ADC inputs. Powered from the CM4's 3.3 V, so it is off whenever the CM4 is off. 32.768 kHz crystal, no high-speed crystal |
| North bridge | **STM32WL5MOCH6** module (STM32WL55: Cortex-M4 + M0+, sub-GHz radio, TCXO and RF switch inside) | Radio gateway. Own 3.3 V regulator from +5 V: it keeps running while the CM4 restarts |
| Radio output | Low-power PA only (RFO_LP, +15 dBm at most); one micro-coax antenna jack | The high-power PA is not wired |
| 1-Wire master | DS2482S-800, I2C address 0x18 | 8 channels, one per RJ12 port |
| Clock | PCF85063A, I2C address 0x51, CR2032 backup | Interrupt and clock outputs go to test points only |
| Accelerometer | LIS2DH12, I2C address 0x19 | Interrupt outputs go to test points only |
| LCD | Winstar WG12864J3, 128 x 64, white backlight, on the display board | Serial interface with CS, SID, SCLK and RST: ST7920 style (the controller is not named in the schematic) |
| Backlight | TPS92612 linear LED driver, PWM input | |
| Buzzer | CUI CMI-1295IC-0385T, self-driven (about 2.4 kHz), on the display board | A steady level is enough: no tone has to be generated |
| Buttons | UP, ENTER, DOWN on a keypad behind the display board | 10 k pull-ups to 3.3 V, pressed = low |
| mini-PCIe slot | USB only. Fitted in the schematic: Quectel EG25-G LTE modem. The slot can instead hold a RAK5146 USB LoRaWAN concentrator | There is no LoRaWAN radio soldered on the board |
| USB | One USB 2.0 port of the CM4 behind a switch: the USB-B socket or the mini-PCIe card | A computer on USB-B takes the bus away from the card |
| Ethernet and PoE | Gigabit Ethernet, PoE 802.3af through a Silvertel Ag9712S module (12 V) | |
| Battery | One Li-Ion cell (NCR18650GA) on the cover board, BQ25303J charger, protection circuit | No fuel gauge: battery and input voltage are read by the south bridge's ADC |

Not on the board: SWD from the CM4 to either microcontroller, any link between the two
microcontrollers, a secure element, an ID EEPROM, a USB hub, RS-485, relays or digital inputs.

## CM4 GPIO map

| BCM GPIO | Net | Goes to | Used as |
|---|---|---|---|
| 4 | `WL_RX_RPI_TX` | North bridge PA10 (USART1 RX), 1 k | `uart3` TXD, `/dev/ttyAMA3` |
| 5 | `WL_TX_RPI_RX` | North bridge PA9 (USART1 TX), 1 k | `uart3` RXD |
| 6 | `RPI_DEBUG_GPIO` | Test point | |
| 7 | `MCU_RESET_EXT` | South bridge NRST, 1 k (R69), 100 nF on the pin | Reset of the south bridge |
| 8 | `MCU_RX_RPI_TX` | South bridge PA10 (USART1 RX), 1 k | `uart4` TXD, `/dev/ttyAMA4` |
| 9 | `MCU_TX_RPI_RX` | South bridge PA9 (USART1 TX), 1 k | `uart4` RXD |
| 10 | `LTE_DTR` | mini-PCIe pin 31, 1 k | Modem DTR |
| 11 | `LTE_RESET` | mini-PCIe pin 22, 1 k | PERST# of the card |
| 12 | `DISP_CS_2` | 10 k to ground; the link to the LCD (R86) is not fitted | Unused as built |
| 13 | `DISP_PWM` | Backlight driver PWM input | PWM channel 1 |
| 14, 15 | `RPI_DEBUG_TX`, `RPI_DEBUG_RX` | Test points | Linux console, if enabled |
| 16 | `DISP_RST` | LCD reset | |
| 17 | `BUZZER` | PNP driver of the buzzer | **Low = sound** |
| 18 | `DISP_CS_1` | LCD chip select through an inverter, 10 k pull-up | SPI chip select; Linux drives it active-low, the LCD sees active-high |
| 19 | `DISP_MISO` | 10 k to ground, not routed to the display | Nothing (the SPI overlay claims it) |
| 20 | `DISP_MOSI` | LCD SID | SPI MOSI |
| 21 | `DISP_SCLK` | LCD SCLK | SPI clock |
| 22 | `WL_RESET_EXT` | North bridge NRST, 1 k (R58), 100 nF on the pin | Reset of the north bridge |
| 23, 24, 25 | `BTN_UP`, `BTN_ENTER`, `BTN_DOWN` | Buttons | Pressed = low |
| 26 | `MCU_BOOT_EXT` | South bridge PA14-BOOT0, 1 k (R82), 10 k pull-down | BOOT0 of the south bridge (the pin is also SWCLK) |
| 27 | `WL_BOOT_EXT` | North bridge PH3, 1 k (R62), 10 k pull-down | BOOT0 of the north bridge |
| CM4 pins 80, 82 | `SCL`, `SDA` | DS2482-800, PCF85063A, LIS2DH12 | The CM4's `SCL0`/`SDA0` pair: GPIO44/45, `i2c_csi_dsi`, `/dev/i2c-10` |

Neither UART has RTS or CTS. GPIO2 and 3 are not connected.

Jumper header JP1: `nRPIBOOT` (USB boot, what FIBER Setup asks for), EEPROM write protect, Wi-Fi
disable, Bluetooth disable.

### What this means for `config.txt`

- North bridge on `uart3`, south bridge on `uart4`. Never with `ctsrts` (it would take GPIO6/7 and
  GPIO10/11).
- **SPI0 must stay off** (`dtparam=spi=on` is wrong on this board): it would claim GPIO7 to 11,
  which are the south bridge's reset, its UART and the card's lines.
- The LCD is on GPIO18 to 21: `spi6-1cs` gives `/dev/spidev6.0` with chip select on GPIO18.
- `dtoverlay=w1-gpio` is wrong on this board (its default pin, GPIO4, is the north bridge's UART);
  1-Wire goes through the DS2482 on I2C.
- At power-on the Raspberry Pi pulls GPIO17 and GPIO22 low until something configures them:
  GPIO17 is the buzzer line, and GPIO22 fights the pull-up on the north bridge's reset. Both get a
  defined level from `config.txt` (`gpio=` lines).

The managed block itself is in [`provisioning.md`](provisioning.md).

## South bridge pins (STM32C031C6)

| Pin | Net | Function |
|---|---|---|
| PA0, PA1 | `PWR_LED_G`, `PWR_LED_Y` | Power LED, green and yellow |
| PA2, PA3 | `LED_0_G`, `LED_0_R` | Port 1 LED, green and red |
| PA4, PA5 | `LED_1_G`, `LED_1_R` | Port 2 |
| PA6, PA7 | `LED_2_G`, `LED_2_R` | Port 3 |
| PB0, PB1 | `LED_3_G`, `LED_3_R` | Port 4 |
| PB2, PB10 | `LED_4_G`, `LED_4_R` | Port 5 |
| PB11, PB12 | `LED_5_G`, `LED_5_R` | Port 6 |
| PB13, PB14 | `LED_6_G`, `LED_6_R` | Port 7 |
| PB15, PA8 | `LED_7_G`, `LED_7_R` | Port 8 |
| PB9, PB8, PB7, PB6, PB5, PB4, PB3, PD3 | `P0` to `P7` | Power switch of ports 1 to 8 (high = on; 100 k pull-downs, so off until driven) |
| PA9, PA10 | `MCU_TX`, `MCU_RX` | USART1 to the CM4 (also the ROM bootloader's UART) |
| PA11 | `ADC_VBAT` | Battery voltage through 68 k / 68 k: pin = VBAT / 2 |
| PA12 | `ADC_VIN` | PoE 12 V rail through 330 k / 68 k: pin = VIN x 68 / 398 |
| PA13, PA14 | SWDIO, SWCLK / BOOT0 | Debug header JP19; PA14 is also BOOT0 from the CM4 |
| PF2 | NRST | From the CM4 (GPIO7) and the debug header |
| PC14, PC15 | 32.768 kHz crystal | |

All LEDs share a 3.3 V anode: **low = lit**. Every port's RJ12 socket carries data on pin 3,
ground on pin 4 and switched 3.3 V (about 0.1 A limit) on pin 5.

**Port k is DS2482 channel k - 1:** the DS2482's `IO0` to `IO7` (nets `DQ0` to `DQ7`, each
through a V2F105C150Y1F protection part) reach pin 3 of `JP7` to `JP14`, the sockets that the
switches `P0` to `P7` power. The switches (TPS22950, current limit set by `ILIM`) have no fault
output: a short on a port's 3.3 V is limited, and nothing reports it.

The schematic's `NB` assembly variant leaves the two ADC dividers unfitted.

## North bridge pins (STM32WL5MOC module)

| Pin | Function |
|---|---|
| PA9, PA10 | USART1 to the CM4 (also the ROM bootloader's UART) |
| PH3 | BOOT0, from the CM4 (GPIO27) |
| NRST | From the CM4 (GPIO22) and the debug header JP15 |
| PA13, PA14 | SWD, debug header only |
| Inside the module | RF switch on PC3, PC4, PC5; TCXO supply |

No other pin of the module is connected: no LEDs, nothing to the south bridge.

RF switch (from ST's driver for this module, matching the earlier firmware): receive = PC3, PC4,
PC5 all high; transmit on the low-power PA = PC3 and PC4 high, PC5 low; off = all low.

## The 1-Wire ports in Linux

What the kernel's drivers (`ds2482`, `wire`, `w1_therm`) give, read in their source at
`raspberrypi/linux` `rpi-6.12.y` and mainline v6.12 (2026-11-18). Not seen on a FIBER yet.

- **Eight bus masters,** `/sys/devices/w1_bus_master1` and up (linked from
  `/sys/bus/w1/devices/`). The `ds2482` driver registers one for each channel, channel 0 first,
  each taking the lowest free number. Only when the chip answers a channel select to channel 7
  does it register eight; otherwise one.
- **Nothing names a master's chip or channel:** a master has no parent device, no link to the
  I2C device `10-0018`, and no attribute that says where it comes from (`w1_master_pointer` is a
  hashed kernel address). The agent therefore takes exactly eight masters, sorted by their
  number, as ports 1 to 8, and reads nothing when there are more or fewer: any other 1-Wire
  master (a `w1-gpio` overlay, a DS2490 on USB) would shift them.
- **A probe** is `/sys/devices/w1_bus_masterN/28-xxxxxxxxxxxx` (family `28`, the DS18B20, then
  its serial number). The kernel searches each bus every 10 s (`wire.timeout`), and drops a
  probe after 10 searches in a row without it (`wire.slave_ttl`): about 100 s. Writing to
  `w1_master_search` starts a search at once (root only).
- **Reading `temperature`** starts a conversion and blocks until it is done: 750 ms at 12 bits.
  It reads millidegrees as text (`21437`). Every error, a probe gone, a bad CRC after 5 tries,
  reads as an empty file, not as an error code. Each master has its own lock, so the eight ports
  can be read side by side; the probes of one port are read one after another.
- **85 °C** is what a DS18B20 holds after power-on, before its first conversion. The driver
  passes it on as a reading unless bit 1 of the probe's `features` is set.
- **`therm_bulk_read`** (all probes of a bus converted at once) appears on one master only: the
  driver keeps one counter for all buses and puts the file on the master of the first DS18B20
  it saw. It is of no use with eight ports.
- **The probes want the port's 3.3 V** (pin 5), not parasite power: the driver arms the
  DS2482's strong pull-up without the chip's lock and without selecting a channel, so with
  several busy ports it may not hold through a conversion (read in the source, not tried).

## Flashing and recovery

Both chips are programmed through their **ROM UART bootloader** (ST AN3155, 115200 baud, 8 data
bits, even parity) on the same UART the firmware uses: the CM4 raises BOOT0, pulses NRST, and the
bootloader answers. There is no other path from the CM4, so this is also the only way to recover a
chip whose firmware does not start.

| | South bridge | North bridge |
|---|---|---|
| Bootloader product id | `0x453` | `0x497` |
| Flash | 16 pages of 2 KB | 128 pages of 2 KB |
| BOOT0 pin honoured when | option bit `nBOOT_SEL` (bit 24) is 0 | option bit `nSWBOOT0` (bit 26) is 1 |
| Factory option bytes | `0xDFFFE1AA`: `nBOOT_SEL` = 1, **the pin is ignored** once flash holds a program | `0x3FFFF0AA`: the pin works |

A factory-fresh STM32C031 enters its bootloader only because its flash is empty. The south
bridge's firmware therefore has to clear `nBOOT_SEL` once (target `0xDEFFE1AA`). The value the
earlier production script wrote, `0xEEFFFEAA`, also clears it but switches the brown-out reset
off; do not copy it.

## Power

PoE (12 V) or the Li-Ion cell feed a buck-boost converter (LT3120) that makes 5 V for the CM4. The
board's 3.3 V rail is the CM4's own output. The north bridge has its own regulator from 5 V, the
mini-PCIe card its own from the battery rail: `U27` (TPS62933, a buck converter that needs at least
3.8 V) takes `BAT+`, the charger's battery node, and starts with the 5 V converter (`POWER_EN`).
**Without a cell the card has no power** (seen on the bench, 2026-11-19). The power button cuts the 5 V converter while it is
held: it is a hardware switch that no GPIO reads.

## Not determined from the schematic

- The LCD's controller chip (the interface says ST7920 style). Driven as an ST7920 (SPI mode 3 at
  400 kHz, `0xF8` before a command and `0xFA` before data), it shows the agent's pages
  (2026-11-19). **The panel is mounted upside down:** sent as drawn, the picture is upside down;
  the agent turns it by 180 degrees, as the earlier software did, and then it is upright
  (`bench-checks.md`, check 2).
- Whether the RAK5146 USB card needs mini-PCIe pin 22 high or low (it is PERST#, active-low, on the
  modem; RAK's SPI variants use it as an active-high reset).
- The radio's frequency band is not marked: it is set by software.
- TCXO stability of the module.

## Revision 1.3

Revision 1.3 has no microcontrollers, LCD, buttons, buzzer or modem. Its LEDs and port power are
driven over I2C (two LP5009, a TCA9534), battery voltage is read by a TLA2021, and it has a USB hub
and an ID EEPROM. The DS2482-800, the PCF85063A, the LIS2DH12 and the power stage are the same.
This repository does not support it.
