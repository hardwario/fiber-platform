# Fixtures of the converge test

- `config.txt.stock`: `config.txt` of the image FIBER Setup pins (Raspberry Pi OS Lite trixie
  arm64, 2026-10-06), taken from `pi-gen` at the tag `2026-10-06-raspios-trixie-arm64`,
  `stage1/00-boot-files/files/config.txt`. Not compared with the image itself yet.
- `config.txt.expected`: the same file after the `base` role, byte for byte: the stock file with
  `provision/boot/config-fiber.txt` between the two marks at its end.
