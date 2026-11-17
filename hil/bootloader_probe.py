#!/usr/bin/env python3
"""Bench check: does each STM32's ROM bootloader answer when the CM4 drives BOOT0 and NRST?

Everything about flashing and recovering the two microcontrollers of a FIBER rests on this:
the CM4 raises BOOT0, pulses NRST, and the chip's ROM bootloader answers on the UART
(ST AN3155: 115200 baud, 8 data bits, even parity). The schematic of FIBER-R2 rev 2.1 says the
four lines are there (docs/hardware.md); this script proves it on a real unit.

    north bridge  STM32WL55    /dev/ttyAMA3  BOOT0 = GPIO27  NRST = GPIO22  product id 0x497
    south bridge  STM32C031C6  /dev/ttyAMA4  BOOT0 = GPIO26  NRST = GPIO7   product id 0x453

It only READS: the bootloader's version and command list, the product id, and the option bytes.
Nothing is erased or written. Each chip is reset twice (into the bootloader, then back into its
firmware), so whatever firmware it runs restarts.

Run it on the FIBER itself, as root:

    sudo python3 bootloader_probe.py            # both chips
    sudo python3 bootloader_probe.py north      # one of them

Before that, on a plain Raspberry Pi OS (written by FIBER Setup with the FIBER software off):
add `dtoverlay=uart3` and `dtoverlay=uart4` to /boot/firmware/config.txt and restart. No other
program may hold the two serial ports (stop the legacy FIBER software if it is installed).
`pinctrl` (package raspi-utils) sets the GPIO lines.

    python3 bootloader_probe.py --self-test     # anywhere: the protocol code against a fake ROM

Status: written from the schematic and ST's documents, not yet run on a FIBER (docs/bench-checks.md).
"""

import argparse
import os
import select
import shutil
import subprocess
import sys
import termios
import threading
import time

ACK = 0x79
NACK = 0x1F
OPTION_BYTES = 0x1FFF7800  # FLASH_OPTR as stored in system memory, on both chips

CHIPS = {
    "north": {
        "name": "north bridge (STM32WL55)",
        "port": "/dev/ttyAMA3",
        "boot0": 27,
        "nrst": 22,
        "product_id": 0x497,
        # RM0453: 1 = BOOT0 is taken from the PH3 pin (the factory setting).
        "boot_bit": (26, "nSWBOOT0", "BOOT0 comes from the pin", "BOOT0 comes from the nBOOT0 option bit, the pin is ignored"),
    },
    "south": {
        "name": "south bridge (STM32C031C6)",
        "port": "/dev/ttyAMA4",
        "boot0": 26,
        "nrst": 7,
        "product_id": 0x453,
        # RM0490: 0 = BOOT0 is taken from the PA14 pin; 1 (the factory setting) = from an option bit.
        "boot_bit": (24, "nBOOT_SEL", "BOOT0 comes from an option bit, the pin is ignored once flash is programmed", "BOOT0 comes from the pin"),
    },
}


class ProbeError(Exception):
    pass


class Bootloader:
    """The few read-only AN3155 commands, over an open file descriptor."""

    def __init__(self, fd, timeout=1.0):
        self.fd = fd
        self.timeout = timeout

    def read(self, count, what):
        data = b""
        deadline = time.monotonic() + self.timeout
        while len(data) < count:
            left = deadline - time.monotonic()
            if left <= 0 or not select.select([self.fd], [], [], left)[0]:
                raise ProbeError(f"no answer while waiting for {what} (got {len(data)} of {count} bytes)")
            data += os.read(self.fd, count - len(data))
        return data

    def expect_ack(self, what):
        byte = self.read(1, what)[0]
        if byte == NACK:
            raise ProbeError(f"the bootloader refused {what} (NACK)")
        if byte != ACK:
            raise ProbeError(f"unexpected byte 0x{byte:02X} in place of the ACK for {what}")

    def command(self, code, what):
        os.write(self.fd, bytes([code, code ^ 0xFF]))
        self.expect_ack(what)

    def start(self):
        """The first byte after reset: 0x7F lets the bootloader measure the baud rate."""
        os.write(self.fd, b"\x7f")
        self.expect_ack("the start byte 0x7F")

    def get(self):
        """Bootloader version (one byte, 0x31 = 3.1) and the command codes it supports."""
        self.command(0x00, "Get")
        count = self.read(1, "the length of the Get reply")[0]
        body = self.read(count + 1, "the Get reply")
        self.expect_ack("the end of the Get reply")
        return body[0], list(body[1:])

    def get_id(self):
        self.command(0x02, "Get ID")
        count = self.read(1, "the length of the Get ID reply")[0]
        body = self.read(count + 1, "the product id")
        self.expect_ack("the end of the Get ID reply")
        return int.from_bytes(body, "big")

    def read_memory(self, address, count):
        self.command(0x11, "Read Memory (refused while readout protection is on)")
        addr = address.to_bytes(4, "big")
        os.write(self.fd, addr + bytes([addr[0] ^ addr[1] ^ addr[2] ^ addr[3]]))
        self.expect_ack(f"the address 0x{address:08X}")
        os.write(self.fd, bytes([count - 1, (count - 1) ^ 0xFF]))
        self.expect_ack("the byte count")
        return self.read(count, "the memory contents")


def open_port(path):
    """115200 baud, 8 data bits, EVEN parity, one stop bit, raw."""
    fd = os.open(path, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    attrs = termios.tcgetattr(fd)
    attrs[0] = 0  # iflag: no translation, parity errors not stripped
    attrs[1] = 0  # oflag
    attrs[2] = termios.CS8 | termios.PARENB | termios.CREAD | termios.CLOCAL  # cflag: 8E1
    attrs[3] = 0  # lflag: raw
    attrs[4] = termios.B115200
    attrs[5] = termios.B115200
    attrs[6][termios.VMIN] = 0
    attrs[6][termios.VTIME] = 0
    termios.tcsetattr(fd, termios.TCSANOW, attrs)
    termios.tcflush(fd, termios.TCIOFLUSH)
    return fd


def pinctrl(gpio, *mode):
    subprocess.run(["pinctrl", "set", str(gpio), *mode], check=True)


def reset(chip, into_bootloader):
    """Pulse NRST with BOOT0 high (bootloader) or low (firmware).

    NRST is only ever pulled low or released to its pull-up, never driven high: the chip must
    stay able to pull its own reset line.
    """
    if into_bootloader:
        pinctrl(chip["boot0"], "op", "dh")
    else:
        pinctrl(chip["boot0"], "ip", "pd")
    pinctrl(chip["nrst"], "op", "dl")
    time.sleep(0.05)
    pinctrl(chip["nrst"], "ip", "pu")
    time.sleep(0.2)  # the bootloader needs a moment before it listens


def describe_option_bytes(chip, raw):
    optr = int.from_bytes(raw, "little")
    bit, name, if_set, if_clear = chip["boot_bit"]
    value = (optr >> bit) & 1
    lines = [f"option bytes (OPTR)  0x{optr:08X}"]
    rdp = optr & 0xFF
    lines.append(f"readout protection   0x{rdp:02X} ({'level 0, off' if rdp == 0xAA else 'ON: flashing from the CM4 is not possible'})")
    lines.append(f"{name:<20} {value} ({if_set if value else if_clear})")
    return lines


def probe(key):
    chip = CHIPS[key]
    print(f"\n== {chip['name']}: {chip['port']}, BOOT0 = GPIO{chip['boot0']}, NRST = GPIO{chip['nrst']}")
    if not os.path.exists(chip["port"]):
        print(f"FAILED: {chip['port']} does not exist. Is the UART enabled in /boot/firmware/config.txt?")
        return False
    fd = open_port(chip["port"])
    ok = False
    try:
        reset(chip, into_bootloader=True)
        termios.tcflush(fd, termios.TCIOFLUSH)
        rom = Bootloader(fd)
        rom.start()
        print("bootloader           answered (ACK)")
        version, commands = rom.get()
        print(f"bootloader version   {version >> 4}.{version & 0x0F}")
        print("commands             " + " ".join(f"{c:02X}" for c in commands))
        product_id = rom.get_id()
        expected = chip["product_id"]
        print(f"product id           0x{product_id:03X} ({'as expected' if product_id == expected else f'EXPECTED 0x{expected:03X}'})")
        for line in describe_option_bytes(chip, rom.read_memory(OPTION_BYTES, 4)):
            print(line)
        ok = product_id == expected
    except ProbeError as error:
        print(f"FAILED: {error}")
        print("  Possible reasons: the BOOT0 or NRST line is not fitted on this unit; the chip ignores")
        print("  its BOOT0 pin (option bytes); another program holds the port; the UART is not enabled.")
    finally:
        reset(chip, into_bootloader=False)
        os.close(fd)
    print("RESULT: " + ("the CM4 can reach this chip's bootloader" if ok else "NOT confirmed"))
    return ok


class FakeRom(threading.Thread):
    """A ROM bootloader on the far end of a pseudo-terminal, for the self-test."""

    def __init__(self, fd, product_id, optr):
        super().__init__(daemon=True)
        self.fd = fd
        self.product_id = product_id
        self.optr = optr

    def take(self, count):
        data = b""
        while len(data) < count:
            data += os.read(self.fd, count - len(data))
        return data

    def run(self):
        try:
            assert self.take(1) == b"\x7f"
            os.write(self.fd, bytes([ACK]))
            while True:
                code, complement = self.take(2)
                if code ^ complement != 0xFF:
                    os.write(self.fd, bytes([NACK]))
                elif code == 0x00:
                    os.write(self.fd, bytes([ACK, 3, 0x31, 0x00, 0x02, 0x11, ACK]))
                elif code == 0x02:
                    os.write(self.fd, bytes([ACK, 1]) + self.product_id.to_bytes(2, "big") + bytes([ACK]))
                elif code == 0x11:
                    os.write(self.fd, bytes([ACK]))
                    frame = self.take(5)
                    good = frame[0] ^ frame[1] ^ frame[2] ^ frame[3] == frame[4]
                    good = good and int.from_bytes(frame[:4], "big") == OPTION_BYTES
                    os.write(self.fd, bytes([ACK if good else NACK]))
                    count, complement = self.take(2)
                    os.write(self.fd, bytes([ACK if count ^ complement == 0xFF else NACK]))
                    os.write(self.fd, self.optr.to_bytes(4, "little")[: count + 1])
                else:
                    os.write(self.fd, bytes([NACK]))
        except OSError:
            pass  # the test closed its end


def self_test():
    """Run the protocol code against FakeRom. No GPIO, no serial port."""
    for key, optr in (("north", 0x3FFFF0AA), ("south", 0xDFFFE1AA)):
        chip = CHIPS[key]
        ours, theirs = os.openpty()
        attrs = termios.tcgetattr(theirs)
        attrs[0] = attrs[1] = attrs[3] = 0  # raw: the fake must see every byte as it is
        termios.tcsetattr(theirs, termios.TCSANOW, attrs)
        FakeRom(ours, chip["product_id"], optr).start()
        rom = Bootloader(theirs, timeout=2.0)
        rom.start()
        version, commands = rom.get()
        assert (version, commands) == (0x31, [0x00, 0x02, 0x11]), (version, commands)
        assert rom.get_id() == chip["product_id"]
        raw = rom.read_memory(OPTION_BYTES, 4)
        assert int.from_bytes(raw, "little") == optr
        text = "\n".join(describe_option_bytes(chip, raw))
        assert "level 0" in text and f"{chip['boot_bit'][1]}" in text, text
        try:
            rom.command(0x44, "a command the fake does not know")
        except ProbeError as error:
            assert "NACK" in str(error)
        else:
            raise AssertionError("an unknown command must be refused")
        os.close(theirs)
        os.close(ours)
    # Factory option bytes: the WL takes BOOT0 from its pin, the C0 does not.
    assert "from the pin" in describe_option_bytes(CHIPS["north"], (0x3FFFF0AA).to_bytes(4, "little"))[2]
    assert "ignored" in describe_option_bytes(CHIPS["south"], (0xDFFFE1AA).to_bytes(4, "little"))[2]
    assert "from the pin" in describe_option_bytes(CHIPS["south"], (0xDEFFE1AA).to_bytes(4, "little"))[2]
    print("self-test passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("chips", nargs="*", metavar="north|south", help="which chips to probe (default: both)")
    parser.add_argument("--self-test", action="store_true", help="test the protocol code against a fake ROM, no hardware")
    args = parser.parse_args()
    for name in args.chips:
        if name not in CHIPS:
            parser.error(f"unknown chip '{name}' (north or south)")

    if args.self_test:
        self_test()
        return 0
    if os.geteuid() != 0:
        sys.exit("run it as root (sudo): it sets GPIO lines and opens the serial ports")
    if shutil.which("pinctrl") is None:
        sys.exit("`pinctrl` was not found (Raspberry Pi OS package raspi-utils)")

    results = [probe(key) for key in (args.chips or list(CHIPS))]
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
