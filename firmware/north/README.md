# fiber-north

The north bridge's firmware: the FIBER's TOWER gateway on the STM32WL55's radio, on the agent's
word. What it does: [`../../docs/firmware/north.md`](../../docs/firmware/north.md); its link:
[`../../docs/firmware/north-link.md`](../../docs/firmware/north-link.md).

```sh
cargo build --release --locked   # inside this directory: its own target and linker
```

Runs on a FIBER since 2026-11-20, written there by the agent (T25, T26).
