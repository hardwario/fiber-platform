# fiber-radio-spike

A bench firmware for the north bridge (STM32WL55, its M4 core), for the radio spike (`TASKS.md`,
T22): it brings the sub-GHz radio up as TOWER's radio sends (GFSK, 19 216.6 bit/s, 19.8 kHz
deviation, sync word `0xDB624715`, EU 868 channel 0), listens raw, and reports the noise floor
and every frame it hears on USART1 (115200 8N1). Plain registers, nothing of Embassy. Not the north
bridge's firmware, which is T25.

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
