# Documentation

All documentation of the FIBER platform lives here, by subject. A component's own `README.md`
says only what the directory is and how to build it.

| Page | What it holds |
|---|---|
| [`hardware.md`](hardware.md) | The board as the software sees it: chips, pins, buses, how the microcontrollers are flashed |
| [`architecture.md`](architecture.md) | The pieces, the repository layout, the agent, both firmwares, the two radio stacks |
| [`provisioning.md`](provisioning.md) | Components, the state file, the release archive, the `config.txt` block, package sources |
| [`cli.md`](cli.md) | The `fiber` command line: its commands and their exit statuses |
| [`release.md`](release.md) | How a release is made, and the weekly canary |
| [`mqtt-api.md`](mqtt-api.md) | The agent's MQTT API: its topics, its requests (a draft, T13) |
| [`front-panel.md`](front-panel.md) | The LCD's pages, the buttons, the menu (a draft, T20, T21) |
| [`examples.md`](examples.md) | Clients of the MQTT API to copy: the command line, Python, Rust, Node-RED flows (T31) |
| [`ble.md`](ble.md) | The Bluetooth profile for HARDWARIO Manager (a draft, T28) |
| [`firmware/south-link.md`](firmware/south-link.md) | The serial link between the agent and the south bridge (a draft, T13) |
| [`firmware/north-link.md`](firmware/north-link.md) | The serial link between the agent and the north bridge, the radio (a draft, T25) |
| [`firmware/south.md`](firmware/south.md) | The south bridge's firmware: what it does, its pins, its faults, the option bytes |
| [`firmware/north.md`](firmware/north.md) | The north bridge's firmware: the gateway on the radio, its clock, its loop, the watermark (T25) |
| [`firmware/radio.md`](firmware/radio.md) | The radio: what a TOWER node sends on air, and how the north bridge's radio is set to speak it (T22) |
| [`bench-checks.md`](bench-checks.md) | What must be confirmed on a real FIBER first, how, and the results |

To come, each written before the code it describes: the north link in `firmware/`. `cli.md` and `mqtt-api.md` grow with each task.

The work itself, its order and the decisions taken are in [`../TASKS.md`](../TASKS.md).
