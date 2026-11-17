# fiber-south

The firmware of the FIBER's south bridge (STM32C031C6): the status lights, the power of the
1-Wire ports and two voltages. It is described in
[`docs/firmware/south.md`](../../docs/firmware/south.md), its link in
[`docs/firmware/south-link.md`](../../docs/firmware/south-link.md).

```sh
just south          # build it, release profile (from the repository's root)
just south-size     # and its size against the budget of 24 KB
```

It needs `flip-link` (`cargo install flip-link`).
