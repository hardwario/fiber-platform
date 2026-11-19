#!/usr/bin/env python3
"""Bench: read or write the north bridge's flash through its ROM bootloader, from the CM4.

    sudo python3 north_flash.py read backup.bin           # all 256 KB, as a copy to write back
    sudo python3 north_flash.py write image.bin [S]       # erase what it needs, write, verify, reset
    sudo python3 north_flash.py listen S                  # reset the chip, print what it says

After a write it prints what the program says on the port (115200 8N1) for S seconds. The block
with the vector table is written last, so that a write cut short leaves a chip that waits in its
bootloader. The chip is started with a reset (BOOT0 low, NRST pulsed), not with Go: a program
started by Go finds the clocks and USART1 as the bootloader left them.

For the radio spike (TASKS.md, T22) until the agent flashes the north bridge itself (T26). Uses
the protocol code of bootloader_probe.py, in the same directory; nothing else may hold
/dev/ttyAMA3. Tried on a FIBER (2026-11-19): a backup read in 28 s, the spike written and started.
"""
import os
import sys
import termios

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bootloader_probe as bp  # noqa: E402

FLASH = 0x08000000
SIZE = 256 * 1024
PAGE = 2048
CHIP = bp.CHIPS["north"]


def enter():
    fd = bp.open_port(CHIP["port"])
    bp.reset(CHIP, into_bootloader=True)
    termios.tcflush(fd, termios.TCIOFLUSH)
    rom = bp.Bootloader(fd)
    rom.start()
    if rom.get_id() != CHIP["product_id"]:
        raise SystemExit("not the north bridge")
    return fd, rom


def address(rom, addr, what):
    a = addr.to_bytes(4, "big")
    os.write(rom.fd, a + bytes([a[0] ^ a[1] ^ a[2] ^ a[3]]))
    rom.expect_ack(what)


def read_all(rom):
    data = bytearray()
    for addr in range(FLASH, FLASH + SIZE, 256):
        data += rom.read_memory(addr, 256)
    return bytes(data)


def erase(rom, pages):
    rom.command(0x44, "Extended Erase")
    body = (len(pages) - 1).to_bytes(2, "big") + b"".join(p.to_bytes(2, "big") for p in pages)
    check = 0
    for b in body:
        check ^= b
    os.write(rom.fd, body + bytes([check]))
    saved = rom.timeout
    rom.timeout = 30.0
    try:
        rom.expect_ack("the erase")
    finally:
        rom.timeout = saved


def write(rom, addr, chunk):
    rom.command(0x31, "Write Memory")
    address(rom, addr, f"the address 0x{addr:08X}")
    body = bytes([len(chunk) - 1]) + chunk
    check = 0
    for b in body:
        check ^= b
    os.write(rom.fd, body + bytes([check]))
    rom.expect_ack(f"the write at 0x{addr:08X}")


def go(rom, addr):
    rom.command(0x21, "Go")
    address(rom, addr, "the start address")


def to_8n1(fd):
    attrs = termios.tcgetattr(fd)
    attrs[2] = termios.CS8 | termios.CREAD | termios.CLOCAL
    termios.tcsetattr(fd, termios.TCSANOW, attrs)


def listen(fd, seconds):
    """Everything that comes for `seconds`, at 115200 8N1."""
    import select
    import time
    to_8n1(fd)
    end = time.monotonic() + seconds
    while (left := end - time.monotonic()) > 0:
        if select.select([fd], [], [], left)[0]:
            sys.stdout.write(os.read(fd, 4096).decode(errors="replace"))
            sys.stdout.flush()


def main():
    if len(sys.argv) < 3 or sys.argv[1] not in ("read", "write", "listen"):
        raise SystemExit(__doc__)
    if sys.argv[1] == "listen":
        fd = bp.open_port(CHIP["port"])
        to_8n1(fd)
        bp.reset(CHIP, into_bootloader=False)
        listen(fd, float(sys.argv[2]))
        os.close(fd)
        return
    seconds = float(sys.argv[3]) if len(sys.argv) > 3 else 0
    fd, rom = enter()
    try:
        if sys.argv[1] == "read":
            data = read_all(rom)
            with open(sys.argv[2], "wb") as f:
                f.write(data)
            used = len(data.rstrip(b"\xff"))
            print(f"read {len(data)} bytes; the last byte that is not 0xFF is at {used}")
            return
        image = open(sys.argv[2], "rb").read()
        image += b"\xff" * (-len(image) % 256)
        pages = list(range((len(image) + PAGE - 1) // PAGE))
        erase(rom, pages)
        print(f"erased {len(pages)} pages")
        # The first 256 bytes (the vector table) last, so that a write cut short leaves a chip
        # that stays in its bootloader.
        for offset in list(range(256, len(image), 256)) + [0]:
            write(rom, FLASH + offset, image[offset:offset + 256])
        back = b"".join(rom.read_memory(FLASH + o, 256) for o in range(0, len(image), 256))
        if back != image:
            raise SystemExit("the image read back differs")
        print(f"wrote and verified {len(image)} bytes; starting it with a reset")
    finally:
        # BOOT0 low and NRST pulsed: the chip starts from its flash, as after a power-on. The
        # port is 8N1 first, for what the program says at once.
        if sys.argv[1] == "write" and seconds:
            to_8n1(fd)
        bp.reset(CHIP, into_bootloader=False)
    if sys.argv[1] == "write" and seconds:
        listen(fd, seconds)
    os.close(fd)


if __name__ == "__main__":
    main()
