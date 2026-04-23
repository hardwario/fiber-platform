# FIBER Development Platform for Students

## Overview

Create a student-friendly development platform that replicates the FIBER system on **Raspberry Pi OS** (standard Debian-based), replacing the Yocto-built production image. Students install components manually to learn the full stack, and use **Node-RED** for data visualization instead of the custom Fiber Viewer (FastAPI + Next.js).

---

## Current Production Architecture (Reference)

| Component | Production (Yocto) | Dev Platform (Raspberry Pi OS) |
|-----------|--------------------|---------------------------------|
| **OS** | Custom Yocto image (`fiber-image-minimal`) | Raspberry Pi OS (64-bit, Bookworm) |
| **Rust Application** | Built via `meta-rust-bin` + `cargo_bin` class, deployed as systemd service | Manually compiled from source with `cargo build --release` |
| **ChirpStack** | Pre-built binary in Yocto recipe (`chirpstack.bb`) | Manually installed from ChirpStack Debian packages or binary |
| **ChirpStack Concentratord** | Yocto recipe for RAK5146/SX1302 USB gateway | Manually installed + configured for gateway hardware |
| **ChirpStack MQTT Forwarder** | Yocto recipe | Manually installed |
| **MQTT Broker** | Mosquitto (installed via Yocto) | Mosquitto (installed via `apt`) |
| **Data Visualization** | Fiber Viewer (FastAPI + Next.js on port 8000) | **Node-RED** (flows + dashboard nodes) |
| **LoRaWAN Setup** | Auto-provisioned via `lorawan-setup.bb` Python script | Manual ChirpStack configuration via web UI |
| **BLE Provisioning** | `ble-fiber` Rust service | Optional / manual WiFi config |
| **OTA Updates** | RAUC A/B rootfs bundles | Standard `apt upgrade` |
| **Device Management** | qbee-agent | Not needed (local development) |

---

## What Needs to Be Built

### 1. Installation Guide / Scripts for Raspberry Pi OS

**Goal:** Documented steps (and optional helper scripts) for students to set up each component on a fresh Raspberry Pi OS install.

#### 1.1 Base System Setup
- [ ] Raspberry Pi OS 64-bit (Bookworm) flashing instructions
- [ ] Enable required interfaces: SPI, I2C, UART, 1-Wire
- [ ] Device tree overlay configuration (equivalent to `local.conf.sample` overlays):
  - `dtoverlay=i2c-rtc,pcf85063a` (RTC)
  - `dtoverlay=spi0-1cs` (SPI for display)
  - `dtoverlay=uart3`, `dtoverlay=uart4` (extra serial ports)
  - `dtoverlay=w1-gpio` (1-Wire for DS18B20 sensors)
  - `dtoverlay=dwc2` (USB OTG if needed)
- [ ] Install base dependencies: `build-essential`, `pkg-config`, `libssl-dev`, `libdbus-1-dev`, `libudev-dev`, `i2c-tools`, `python3-pip`
- [ ] Set up `/data` directory structure for persistent config (mirrors production layout)

#### 1.2 Rust Toolchain Installation
- [ ] Install Rust via `rustup` (stable toolchain, aarch64-unknown-linux-gnu)
- [ ] Document cross-compilation option from x86 host (optional, for faster builds)
- [ ] Verify build with a simple GPIO blink example (similar to `hello-rs` in Yocto)

#### 1.3 FIBER Rust Application
**Source:** `/home/frese/fiber/application` (the existing Rust codebase)

- [ ] Clone the application repository
- [ ] Install Rust dependencies (handled by Cargo, but some system libs needed):
  - `libsqlite3-dev` (for `rusqlite` with bundled feature — or system SQLite)
  - `libdbus-1-dev` (for D-Bus / BlueZ integration)
  - Serial port access (`/dev/ttyAMA4`) — add user to `dialout` group
  - GPIO access — add user to `gpio` group or run with appropriate permissions
  - I2C access (`/dev/i2c-10`) — add user to `i2c` group
- [ ] Build: `cargo build --release`
- [ ] Deploy configuration files:
  - `fiber.config.yaml` → `/data/fiber/config/`
  - `fiber.sensors.config.yaml` → `/data/fiber/config/`
  - `fiber.network.config.yaml` → `/data/fiber/config/`
- [ ] Create systemd service file (equivalent to what `fiber.bb` recipe installs)
- [ ] Verify the application starts and reads sensors

**Decisions needed:**
- Should students build on the Pi directly or cross-compile? (Building on Pi is slower but simpler)
- Should the Rust app be simplified/stripped down for educational use? (e.g., remove EU MDR compliance, crypto/authorization, BLE provisioning)
- Should we provide a pre-built binary as an alternative to compiling from source?

#### 1.4 Mosquitto MQTT Broker
- [ ] Install: `sudo apt install mosquitto mosquitto-clients`
- [ ] Configure authentication (match production: user `fiber`, password from config)
- [ ] Configure listener on port 1883
- [ ] Enable and start service
- [ ] Test with `mosquitto_pub` / `mosquitto_sub`

#### 1.5 ChirpStack LoRaWAN Network Server
**Source:** Currently installed as pre-built binary in Yocto (`chirpstack.bb`)

- [ ] Install ChirpStack v4 from official Debian repository or download binary
- [ ] Install Redis (required by ChirpStack): `sudo apt install redis-server`
- [ ] Configure ChirpStack:
  - Adapt `/etc/chirpstack/chirpstack.toml` from Yocto recipe config files
  - Set up SQLite or PostgreSQL backend
  - Configure MQTT integration (point to local Mosquitto)
  - Set up region configuration (EU868 or as needed)
- [ ] Create persistent data directory: `/data/chirpstack/`
- [ ] Enable and start ChirpStack service
- [ ] Access ChirpStack web UI (port 8080) for device management

#### 1.6 ChirpStack Concentratord (LoRa Gateway)
**Source:** Currently in Yocto as `chirpstack-concentratord.bb` for RAK5146/SX1302

- [ ] Install ChirpStack Concentratord binary
- [ ] Configure for specific gateway hardware (RAK5146 USB, SX1302 chipset)
- [ ] Set up concentratord config (frequency plan, SPI/USB interface)
- [ ] Enable and start service
- [ ] Verify gateway is seen in ChirpStack web UI

#### 1.7 ChirpStack MQTT Forwarder
- [ ] Install ChirpStack MQTT Forwarder binary
- [ ] Configure to bridge concentratord to ChirpStack via Mosquitto
- [ ] Enable and start service

#### 1.8 LoRaWAN Device Provisioning
In production, `lorawan-setup.bb` auto-provisions via a Python script. For the dev platform:

- [ ] Document manual provisioning steps via ChirpStack web UI:
  - Create network server profile
  - Create device profile (STICKER codec)
  - Create application
  - Register devices (DevEUI, AppKey)
- [ ] Optionally provide the provisioning Python script for automation

---

### 2. Node-RED Data Visualization (Replaces Fiber Viewer)

**Goal:** Replace the custom FastAPI + Next.js viewer with Node-RED flows that students can understand, modify, and extend.

#### 2.1 Node-RED Installation
- [ ] Install Node-RED: `sudo apt install nodejs npm && sudo npm install -g node-red`
  - Or use the official Pi install script: `bash <(curl -sL https://raw.githubusercontent.com/node-red/linux-installers/master/deb/update-nodejs-and-nodered)`
- [ ] Install required palette nodes:
  - `node-red-dashboard` (or `@flowfuse/node-red-dashboard` for Dashboard 2.0)
  - `node-red-contrib-mqtt-broker` (if not using built-in MQTT nodes)
  - `node-red-node-sqlite` (for historical data)
- [ ] Enable and start Node-RED as a service (port 1880)

#### 2.2 MQTT Topic Subscription Flows
Replicate what the Fiber Viewer backend does (`mqtt/handlers.py`, `mqtt/topics.py`):

- [ ] **Sensor Data Flow:** Subscribe to `fiber/{hostname}/sensors/aggregated`
  - Parse JSON payload (min, max, avg temperatures per sensor line)
  - Route to dashboard gauges/charts
  - Store in SQLite for historical view

- [ ] **System Info Flow:** Subscribe to `fiber/{hostname}/system/info`
  - Parse power status (AC/battery, voltage, percentage)
  - Parse network status (WiFi/Ethernet, IP address)
  - Parse storage status (database size, record count)
  - Display on dashboard info panel

- [ ] **Alarm Events Flow:** Subscribe to `fiber/{hostname}/alarms/events`
  - Parse alarm state transitions (Normal→Warning→Critical→Disconnected)
  - Display alarm notifications on dashboard
  - Log to SQLite for alarm history

- [ ] **Device Status Flow:** Subscribe to `fiber/{hostname}/status`
  - Track device online/offline state (Last Will and Testament)
  - Show connection indicator on dashboard

- [ ] **LoRaWAN Sensor Flow:** Subscribe to `application/+/device/+/event/up`
  - Parse ChirpStack uplink messages
  - Extract STICKER sensor data (temperature, humidity, voltage, illuminance)
  - Display on separate LoRaWAN dashboard tab

#### 2.3 Node-RED Dashboard Pages
Replicate key Fiber Viewer pages:

- [ ] **Overview Dashboard:**
  - Temperature gauges for each sensor line (0-7)
  - Color-coded alarm state (green/yellow/red/gray)
  - Last update timestamp per sensor
  - Device connection status indicator

- [ ] **Temperature Charts:**
  - Real-time line chart (last hour, configurable)
  - Historical chart with time range selector
  - Min/Max/Avg display per sensor

- [ ] **Alarm Log:**
  - Table of alarm events with timestamp, sensor, old state, new state
  - Filter by severity
  - Alarm acknowledgment button

- [ ] **System Status:**
  - Power mode (AC/Battery) with voltage display
  - Battery percentage bar
  - Network connection type and IP
  - Storage usage
  - Device uptime

- [ ] **LoRaWAN Devices (if gateway present):**
  - List of connected STICKER sensors
  - Temperature/humidity readings per device
  - RSSI/SNR signal quality

#### 2.4 Command Publishing (Dashboard → Device)
Replicate Fiber Viewer command functionality (`api/routes.py`):

- [ ] **Set Threshold:** Dashboard form → MQTT publish to `fiber/{hostname}/commands/sensor/set_threshold`
- [ ] **Restart Device:** Button → MQTT publish to `fiber/{hostname}/commands/system/restart_device`
- [ ] **Request Info:** Button → MQTT publish to `fiber/{hostname}/commands/system/get_info`

#### 2.5 Export Node-RED Flows
- [ ] Export all flows as `flows.json` for students to import
- [ ] Document each flow with comments/annotations
- [ ] Provide a "minimal" flow set (just sensor display) and a "full" flow set

---

### 3. Documentation

#### 3.1 Student Setup Guide
- [ ] Step-by-step guide from "flash Raspberry Pi OS" to "see data in Node-RED"
- [ ] Hardware requirements list (Pi 4/5, RAK5146 gateway, DS18B20 sensors, wiring diagram)
- [ ] Troubleshooting section (common issues: permissions, serial port, I2C detection)
- [ ] Estimated time for each step

#### 3.2 Architecture Documentation
- [ ] System architecture diagram showing how components connect:
  ```
  DS18B20 sensors → FIBER Rust App → Mosquitto MQTT → Node-RED Dashboard
                                         ↑
  STICKER LoRa sensors → RAK5146 → Concentratord → ChirpStack → MQTT
  ```
- [ ] MQTT topic reference (reuse/adapt from `docs/MQTT_TOPICS.md` in application repo)
- [ ] Data flow explanation (sensor reading → aggregation → MQTT → Node-RED → display)

#### 3.3 Exercise/Lab Guide (Optional)
- [ ] Lab 1: Install Raspberry Pi OS and enable hardware interfaces
- [ ] Lab 2: Install and test Mosquitto MQTT broker
- [ ] Lab 3: Build and run the FIBER Rust application
- [ ] Lab 4: Create a basic Node-RED flow to display temperature
- [ ] Lab 5: Install and configure ChirpStack for LoRaWAN
- [ ] Lab 6: Build a complete monitoring dashboard in Node-RED
- [ ] Lab 7: Add alarm logic and notifications

---

### 4. Configuration Adaptations

Files from the production system that need adaptation for Raspberry Pi OS:

| Production File | Location in Yocto | What to Adapt |
|-----------------|-------------------|---------------|
| `chirpstack.toml` | `meta-fiber/recipes-connectivity/chirpstack/files/` | Paths, database backend, region config |
| `concentratord.toml` | `meta-fiber/recipes-connectivity/chirpstack-concentratord/files/` | Gateway hardware type, USB/SPI interface |
| `chirpstack-mqtt-forwarder.toml` | `meta-fiber/recipes-connectivity/chirpstack-mqtt-forwarder/files/` | MQTT broker address |
| `mosquitto.conf` | Yocto default + append | Auth, listener, persistence |
| `fiber.config.yaml` | `application/fiber.config.yaml` | Paths (use `/data/fiber/` or `/opt/fiber/`) |
| `fiber.sensors.config.yaml` | `application/fiber.sensors.config.yaml` | Sensor line count, thresholds |
| systemd service files | Generated by Yocto recipes | Recreate for manual install |

---

### 5. Packaging & Distribution

**Decision:** How to distribute the dev platform to students?

| Option | Pros | Cons |
|--------|------|------|
| **A. Full documentation only** | Maximum learning, students do everything | Slow, error-prone, support burden |
| **B. Install script + docs** | Guided automation with learning | Script maintenance, Pi OS version drift |
| **C. Pre-built SD card image** | Instant setup, consistent environment | Defeats "manual install" learning goal |
| **D. Ansible playbook** | Reproducible, self-documenting | Extra tooling for students to learn |

**Recommendation:** Option **B** — Provide a well-documented install script (`install.sh`) that students can read, understand, and run step-by-step. Each section is clearly commented so it doubles as documentation.

---

### 6. Differences from Production to Address

| Concern | Production Solution | Dev Platform Approach |
|---------|--------------------|-----------------------|
| **Persistent data across updates** | Separate `/data` partition (Yocto wks) | Create `/data` directory, document backup |
| **First-boot provisioning** | `fiber-firstboot` systemd service | Part of install script |
| **BLE WiFi provisioning** | `ble-fiber` Rust service + phone app | Skip — students use keyboard/SSH for WiFi |
| **A/B rootfs updates** | RAUC bundles | Not needed — use `apt` |
| **Device management** | qbee-agent | Not needed — local access |
| **Display driver** | ST7920 via SPI/GPIO | Keep if hardware present, skip otherwise |
| **Authorization/crypto** | Ed25519 signed commands | Simplify or disable for dev |
| **LCD + Buttons** | Hardware UI on device | Optional — Node-RED is the primary UI |

---

### 7. Component Dependency Map

```
                    ┌─────────────────┐
                    │  Raspberry Pi OS │
                    │  (64-bit Bookworm)│
                    └────────┬────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
              ▼              ▼              ▼
     ┌────────────┐  ┌────────────┐  ┌──────────┐
     │ Rust       │  │ Node.js    │  │ Python3  │
     │ (rustup)   │  │ (for       │  │ (system) │
     │            │  │  Node-RED) │  │          │
     └─────┬──────┘  └─────┬──────┘  └────┬─────┘
           │                │              │
           ▼                ▼              │
   ┌───────────────┐ ┌──────────┐         │
   │ FIBER App     │ │ Node-RED │         │
   │ (cargo build) │ │ (npm -g) │         │
   └───────┬───────┘ └────┬─────┘         │
           │               │              │
           │    ┌──────────┘              │
           ▼    ▼                         │
      ┌──────────────┐                   │
      │  Mosquitto   │◄──────────────────┘
      │  MQTT Broker │        (optional provisioning
      └──────┬───────┘         scripts)
             │
      ┌──────┴────────────────────┐
      │                           │
      ▼                           ▼
┌───────────────┐        ┌──────────────────┐
│ ChirpStack v4 │        │ Redis            │
│ (network srv) │◄───────│ (required by CS) │
└──────┬────────┘        └──────────────────┘
       │
┌──────┴───────────────────────┐
│                              │
▼                              ▼
┌────────────────────┐  ┌──────────────────────┐
│ Concentratord      │  │ MQTT Forwarder       │
│ (RAK5146 gateway)  │  │ (concentratord→MQTT) │
└────────────────────┘  └──────────────────────┘
```

Install order: OS → base deps → Mosquitto → Rust → FIBER app → Redis → ChirpStack → Concentratord → MQTT Forwarder → Node.js → Node-RED

---

### 8. Estimated Scope

| Work Item | Effort | Notes |
|-----------|--------|-------|
| Base install documentation | Medium | RPi OS setup, interfaces, dependencies |
| FIBER app build instructions | Medium | Cargo build, systemd service, config files |
| ChirpStack install instructions | Medium | Packages, config adaptation, provisioning |
| Mosquitto setup | Small | Standard apt install + config |
| Node-RED install + basic flows | Medium | Install + MQTT subscription flows |
| Node-RED dashboard (full) | Large | Replicate key Fiber Viewer functionality |
| Node-RED command flows | Small | Publish to command topics |
| Export/package flows.json | Small | Export and document |
| Install helper script | Medium | Automated but readable |
| Architecture docs + diagrams | Medium | Adapt from existing docs |
| Lab exercises (optional) | Large | 7 structured labs with validation steps |
| Config file adaptation | Small | Paths, permissions for RPi OS |
| Testing on fresh Pi | Medium | Validate full flow end-to-end |

---

### 9. Open Questions

1. **Target Pi hardware:** Raspberry Pi 4 only, or also Pi 5? (affects device tree overlays, GPIO library compatibility)
2. **Gateway hardware:** RAK5146 USB only, or support other LoRa gateways?
3. **Sensor hardware:** DS18B20 only, or also STICKER LoRaWAN sensors?
4. **FIBER app scope:** Full application or stripped-down educational version?
5. **Node-RED Dashboard version:** Dashboard 1.0 (`node-red-dashboard`) or Dashboard 2.0 (`@flowfuse/node-red-dashboard`)?
6. **Student skill level:** Do they know Linux CLI basics? Rust? MQTT?
7. **Distribution method:** Git repo, downloadable ZIP, or course platform?
8. **Should the existing Fiber Viewer still be available as an optional "advanced" install?**
