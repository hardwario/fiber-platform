# fiber-radio-spike

A bench firmware for the north bridge (STM32WL55, its M4 core), for the radio spike (`TASKS.md`,
T22, T23): it brings the sub-GHz radio up as TOWER's radio sends (`docs/firmware/radio.md`) and,
by `MODE` in `src/main.rs`, receives TOWER packets through the codec (the default; it never
sends), sends `radio_beacon`'s frames, captures raw bytes after any preamble, or surveys the noise
floor. It reports on USART1 (115200 8N1). Plain registers for the board, and the host workspace's
`fiber-subghz` and `fiber-tower-air` for the radio and the packet, as the north bridge's firmware
(T25) will use them; nothing of Embassy.

```sh
cd hil/radio-spike && cargo build --release      # needs flip-link
llvm-objcopy -O binary target/thumbv7em-none-eabi/release/fiber-radio-spike radio-spike.bin
```

On the FIBER, with `north_flash.py` and `bootloader_probe.py` beside it:

```sh
sudo python3 north_flash.py read north-backup.bin     # first: a copy of what the chip has
sudo python3 north_flash.py write radio-spike.bin 130
```

What it found is in [`docs/bench-checks.md`](../../docs/bench-checks.md), check 4.
