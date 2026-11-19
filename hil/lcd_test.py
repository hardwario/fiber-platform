#!/usr/bin/env python3
"""Bench (TASKS.md, T2): a test pattern on the LCD, as an ST7920 takes it.

    sudo python3 lcd_test.py [MODE] [SPEED]      # SPI mode (default 3) and speed (default 400000)

Four lines of text from the controller's own font, and in its graphics memory a frame with a
checkerboard on the right. The text is not turned: if it reads upside down, the panel is mounted
upside down, as the earlier software assumed. Needs `python3-spidev`, `/dev/spidev6.0`
(`dtoverlay=spi6-1cs`) and the agent stopped (it drives the LCD). Run on a FIBER (2026-11-19)
without an error; what it showed was not seen.
"""
import subprocess
import sys
import time

import spidev

MODE = int(sys.argv[1]) if len(sys.argv) > 1 else 3
SPEED = int(sys.argv[2]) if len(sys.argv) > 2 else 400_000

spi = spidev.SpiDev()
spi.open(6, 0)
spi.mode = MODE
spi.max_speed_hz = SPEED


def send(sync, byte, wait=0.0001):
    spi.writebytes([sync, byte & 0xF0, (byte << 4) & 0xF0])
    time.sleep(wait)


def cmd(byte, wait=0.0002):
    send(0xF8, byte, wait)


def data(byte):
    send(0xFA, byte, 0.00005)


def pinctrl(*args):
    subprocess.run(["pinctrl", "set", *args], check=True)


# Reset on GPIO16: low for 10 ms, then back up.
pinctrl("16", "op", "dl")
time.sleep(0.01)
pinctrl("16", "op", "dh")
time.sleep(0.05)

cmd(0x30)  # basic instruction set
cmd(0x30)
cmd(0x0C)  # display on, no cursor
cmd(0x01, 0.002)  # clear
cmd(0x06)  # entry: increment

lines = ["FIBER bench T2", f"SPI mode {MODE}", f"{SPEED // 1000} kHz", "2026-11-19"]
for address, text in zip((0x80, 0x90, 0x88, 0x98), lines):
    cmd(address)
    for ch in text.ljust(16)[:16]:
        data(ord(ch))

# Graphics: a border, and a checkerboard on the right quarter.
cmd(0x34)  # extended set, graphics off while writing
def row_bytes(y):
    row = bytearray(16)
    if y in (0, 63):
        row[:] = b"\xff" * 16
    else:
        row[0] |= 0x80
        row[15] |= 0x01
        if 8 <= y < 56:
            pattern = 0xCC if (y // 2) % 2 == 0 else 0x33
            for x in range(12, 15):
                row[x] = pattern
    return row

for y in range(32):
    cmd(0x80 | y)
    cmd(0x80)
    for byte in row_bytes(y) + row_bytes(y + 32):
        data(byte)
cmd(0x36)  # extended set, graphics on
print(f"pattern written: mode {MODE}, {SPEED} Hz")
