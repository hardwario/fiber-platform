# FIBER Dev Platform

Student development platform for the **FIBER IoT temperature monitoring system**.

Runs on **Raspberry Pi 4 Compute Module** with **Raspberry Pi OS (Bookworm 64-bit)** — a simplified version of the production Yocto-based system designed for learning.

## System Overview

```
  DS18B20 Sensors (1-Wire)
         |
         v
  +--------------+       +------------+       +-------------------+
  |  FIBER App   | MQTT  | Mosquitto  | MQTT  | Node-RED          |
  |  (Rust)      |------>| Broker     |------>| Dashboard 2.0     |
  |              |<------| :1883      |<------| :1880             |
  +--------------+       +------------+       +-------------------+
         |                                           |
    reads sensors                               +---------+
    monitors alarms                             | Browser |
    controls LEDs/buzzer                        +---------+
```

## Repository Contents

```
config/
  fiber.config.yaml          # FIBER app configuration
  fiber.sensors.config.yaml  # Sensor alarm thresholds (6-level)
  fiber.service              # FIBER systemd service
  mosquitto.conf             # MQTT broker config
  ds2482-init.sh             # 1-Wire bridge init script
  ds2482-init.service        # 1-Wire bridge systemd service
  w1-rescan.sh               # 1-Wire bus rescan script
  w1-rescan.service          # 1-Wire rescan systemd service
  w1-rescan.timer            # 1-Wire rescan timer (1s interval)
docs/
  manual-setup.md            # Step-by-step setup (recommended)
  architecture.md            # System architecture and data flow
  mqtt-reference.md          # MQTT topics and payload formats
  setup-guide.md             # Setup guide (uses install.sh)
node-red/
  flows.json                 # Dashboard 2.0 flows
install.sh                   # Automated installer (alternative to manual setup)
```

## Getting Started

Follow the **[Manual Setup Guide](docs/manual-setup.md)** for step-by-step commands after a fresh Raspberry Pi OS install.

### Prerequisites

- Raspberry Pi 4 Compute Module + IO Board
- Raspberry Pi OS 64-bit (Bookworm)
- DS18B20 temperature sensors with DS2482 I2C-to-1-Wire bridge
- SSH access to the Pi

### Quick Overview

1. Install system dependencies (`i2c-tools`, `mosquitto`, `sqlite3`, ...)
2. Enable hardware interfaces (1-Wire, UART, SPI6, I2C, PWM)
3. Configure Mosquitto MQTT broker
4. Clone this repo and copy config files + binary
5. Set up 1-Wire bridge services
6. Start the FIBER application
7. Install Node-RED + Dashboard 2.0
8. Open the dashboard at `http://<pi-ip>:1880/dashboard`

## Services and Ports

| Service | Port | Access |
|---------|------|--------|
| Mosquitto MQTT | 1883 | `mosquitto_sub -h localhost -u fiber -P 123456789 -t 'fiber/#' -v` |
| Node-RED Editor | 1880 | `http://<pi-ip>:1880` |
| Node-RED Dashboard | 1880 | `http://<pi-ip>:1880/dashboard` |

## MQTT Credentials

| User | Password |
|------|----------|
| `fiber` | `123456789` |

## Documentation

| Document | Description |
|----------|-------------|
| [Manual Setup](docs/manual-setup.md) | Step-by-step commands after fresh RPi OS install |
| [Architecture](docs/architecture.md) | System diagram, data flow, component details |
| [MQTT Reference](docs/mqtt-reference.md) | All topics, payload formats, and test commands |
