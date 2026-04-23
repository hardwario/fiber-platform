# FIBER Dev Platform — Architecture

## System Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                     Raspberry Pi 4 CM                           │
│                                                                 │
│  ┌──────────────┐    MQTT     ┌──────────────┐    HTTP/WS      │
│  │  FIBER App   │───────────►│  Mosquitto   │◄──────────────┐  │
│  │  (Rust bin)  │◄───────────│  MQTT Broker │───────────┐   │  │
│  └──────┬───────┘            │  :1883       │           │   │  │
│         │                    └──────────────┘           │   │  │
│         │                                               │   │  │
│    ┌────┴────┐                                    ┌─────┴───┴─┐│
│    │ Sensors │                                    │  Node-RED ││
│    │ DS18B20 │                                    │  :1880    ││
│    │ (1-Wire)│                                    │           ││
│    └─────────┘                                    │ Dashboard ││
│                                                   │   2.0     ││
│                                                   └───────────┘│
└─────────────────────────────────────────────────────────────────┘
                                                         │
                                                    ┌────┴────┐
                                                    │ Browser │
                                                    │ :1880/  │
                                                    │dashboard│
                                                    └─────────┘
```

## Data Flow

```
DS18B20 Sensors (1-Wire bus)
       │
       ▼
FIBER Rust Application
  ├── Reads sensors every 2s (sample_interval_ms)
  ├── Aggregates data every 15s (aggregation_interval_ms)
  ├── Reports via MQTT every 60s (report_interval_ms)
  ├── Monitors 6-level alarm thresholds
  ├── Publishes sensor, power, network, and system data
  └── Stores data in SQLite (EU MDR compliant)
       │
       ▼
Mosquitto MQTT Broker (localhost:1883)
  ├── fiber/{hostname}/sensors/aggregated   → aggregated sensor readings
  ├── fiber/{hostname}/sensors/summary      → lightweight sensor summary
  ├── fiber/{hostname}/sensors/line{N}/alarm → per-line alarm state
  ├── fiber/{hostname}/system/info          → device status (power, network, storage)
  ├── fiber/{hostname}/power/...            → battery %, voltage, AC state
  ├── fiber/{hostname}/network/...          → WiFi, Ethernet status
  ├── fiber/{hostname}/alarms/events        → alarm transitions
  ├── fiber/{hostname}/status               → online/offline (LWT)
  ├── fiber/{hostname}/lorawan/sensors      → LoRaWAN sensor data
  └── fiber/{hostname}/commands/...         ← commands from dashboard
       │
       ▼
Node-RED (localhost:1880)
  ├── Subscribes to MQTT topics
  ├── Parses JSON payloads
  ├── Routes data to Dashboard 2.0 widgets
  └── Publishes commands back via MQTT
       │
       ▼
Browser Dashboard (http://<ip>:1880/dashboard)
  ├── Overview: temperature gauges + connection status
  ├── Charts: real-time temperature line charts
  ├── Alarms: event log + notifications
  ├── System: power, network, storage, uptime
  └── Commands: set thresholds, request info, restart
```

## Component Details

### FIBER Application (Rust Binary)

The core IoT application that reads DS18B20 temperature sensors and publishes data via MQTT.

| Aspect | Detail |
|--------|--------|
| Binary | `/opt/fiber/fiber_app` |
| Config | `/data/fiber/config/fiber.config.yaml` |
| Sensors Config | `/data/fiber/config/fiber.sensors.config.yaml` |
| Database | `/data/fiber/fiber_medical.db` (SQLite) |
| Service | `fiber.service` (systemd) |
| Serial | `/dev/ttyAMA4` (115200 baud) |
| I2C | `/dev/i2c-10` (accelerometer) |

### Mosquitto MQTT Broker

Standard MQTT message broker. All communication between the FIBER app and Node-RED goes through Mosquitto.

| Aspect | Detail |
|--------|--------|
| Port | 1883 |
| User | `fiber` |
| Password | `123456789` |
| Config | `/etc/mosquitto/conf.d/fiber.conf` |
| Persistence | `/var/lib/mosquitto/` |

### Node-RED + Dashboard 2.0

Visual programming tool for data flow processing and dashboard creation.

| Aspect | Detail |
|--------|--------|
| Editor | `http://<ip>:1880` |
| Dashboard | `http://<ip>:1880/dashboard` |
| Flows | `~/.node-red/flows.json` |
| Dashboard pkg | `@flowfuse/node-red-dashboard` |
| Service | `nodered.service` (systemd) |

## Directory Layout

```
/opt/fiber/
  └── fiber_app              # Application binary

/data/fiber/
  ├── config/
  │   ├── fiber.config.yaml           # Main config
  │   └── fiber.sensors.config.yaml   # Sensor thresholds
  ├── fiber_medical.db        # SQLite database
  ├── sensor_log.csv          # CSV log file
  └── backups/                # Database backups

/etc/mosquitto/
  ├── conf.d/
  │   └── fiber.conf          # FIBER MQTT config
  └── passwd                  # MQTT password file

~/.node-red/
  ├── flows.json              # Node-RED flows (FIBER dashboard)
  ├── node_modules/           # Installed nodes
  └── settings.js             # Node-RED settings
```

## Sensor Hardware

### DS18B20 (1-Wire Temperature Sensors)

- **Bus:** 1-Wire (GPIO4 default)
- **Lines:** Up to 8 sensor lines
- **Range:** -55 to +125 C
- **Precision:** +/-0.5 C
- **Pull-up:** 4.7k ohm resistor required between data and VCC

### Alarm Thresholds (6-Level System)

The FIBER application uses a 6-level alarm threshold hierarchy. The `alarm_low` and `alarm_high` levels are optional (used in production for finer-grained alerting).

```
Critical Low  <  Alarm Low  <  Warning Low  <  [NORMAL]  <  Warning High  <  Alarm High  <  Critical High
   18.0           (opt)          27.0                          38.5           (opt)           41.0
```

| State | Condition | Action |
|-------|-----------|--------|
| Normal | Between warning_low and warning_high | Green LED |
| Warning | Below warning_low or above warning_high | Yellow LED, buzzer |
| Alarm | Below alarm_low or above alarm_high (optional) | Red LED (steady) |
| Critical | Below critical_low or above critical_high | Red LED (fast blink), buzzer |
| Disconnected | Sensor not responding | Red LED (slow blink), buzzer |

**Default thresholds:**

| Level | Low (C) | High (C) |
|-------|---------|-----------|
| Critical | 18.0 | 41.0 |
| Alarm | — (optional) | — (optional) |
| Warning | 27.0 | 38.5 |
