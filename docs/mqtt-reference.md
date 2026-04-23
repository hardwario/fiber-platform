# FIBER Dev Platform — MQTT Topic Reference

All MQTT communication uses the pattern: `fiber/{hostname}/{category}/{type}`

The `{hostname}` is the device's hostname (e.g., `FIBER-001`). Use `hostname` command on the Pi to check.

---

## Published by Device (Subscribe to these)

### Sensor Data (Aggregated)

**Topic:** `fiber/{hostname}/sensors/aggregated`
**QoS:** 0 | **Interval:** every `report_interval_ms` (default 60s)

```json
{
  "timestamp": "2026-01-13T14:30:00Z",
  "period_start_ts": 1736775000,
  "period_end_ts": 1736775060,
  "duration_sec": 60,
  "sensors": [
    {
      "line": 0,
      "name": "Sensor 1",
      "location": null,
      "sample_count": 60,
      "disconnected_count": 0,
      "temperature": {
        "min_celsius": 36.5,
        "max_celsius": 37.2,
        "avg_celsius": 36.8
      },
      "alarm_counts": {
        "normal": 60,
        "warning": 0,
        "critical": 0,
        "disconnected": 0,
        "reconnecting": 0
      },
      "dominant_alarm_state": "NORMAL",
      "alarm_triggered_at": null
    }
  ]
}
```

### Sensor Summary

**Topic:** `fiber/{hostname}/sensors/summary`
**QoS:** 0

Lightweight summary of all sensor lines (current state, no history).

### Per-Line Alarm Status

**Topic:** `fiber/{hostname}/sensors/line{N}/alarm` (N = 0-7)
**QoS:** 0

Published when alarm state changes on a specific line. Contains the current alarm state and temperature.

### Device Status (Online/Offline)

**Topic:** `fiber/{hostname}/status`
**QoS:** 1 | **Retained:** yes

Online:
```json
{
  "status": "online",
  "timestamp": "2026-01-13T14:30:00Z",
  "dev_mode": false
}
```

Offline (Last Will and Testament — sent automatically by broker):
```json
{
  "status": "offline",
  "reason": "unexpected_disconnect"
}
```

### System Info

**Topic:** `fiber/{hostname}/system/info`
**QoS:** 1 | **Retained:** yes | **Interval:** every 60s

```json
{
  "timestamp": "2026-01-13T14:30:00Z",
  "hostname": "FIBER-001",
  "device_label": "DEV-PLATFORM",
  "firmware_version": "0.1.0",
  "dev_mode": true,
  "uptime_seconds": 86400,
  "uptime_human": "1d 0h 0m 0s",
  "power": {
    "battery": {
      "voltage_mv": 4200,
      "percentage": 85,
      "status": "normal"
    },
    "dc": {
      "voltage_mv": 5000,
      "connected": true
    },
    "last_dc_loss": "2026-01-12T10:15:00Z"
  },
  "network": {
    "wifi": {
      "connected": true,
      "signal_dbm": -45,
      "ip": "192.168.1.100"
    },
    "ethernet": {
      "connected": false,
      "ip": null
    },
    "has_internet": true
  },
  "storage": {
    "data_partition": {
      "total_bytes": 5368709120,
      "available_bytes": 3221225472,
      "used_percent": 40
    }
  },
  "lorawan_gateway_present": false,
  "lorawan_concentratord_running": false,
  "lorawan_chirpstack_running": false,
  "lorawan_sensor_count": 0
}
```

### Power Topics

Individual power metrics published on dedicated topics:

| Topic | QoS | Description |
|-------|-----|-------------|
| `fiber/{hostname}/power/battery/percentage` | 0 | Battery charge percentage (integer) |
| `fiber/{hostname}/power/battery/voltage` | 0 | Battery voltage in millivolts |
| `fiber/{hostname}/power/battery/status` | 0 | Battery status string (`normal`, `low`, `critical`) |
| `fiber/{hostname}/power/ac/connected` | 0 | AC/DC power connected (`true`/`false`) |
| `fiber/{hostname}/power/events/dc_loss` | 1 | Published when DC power is lost |

### Network Topics

| Topic | QoS | Description |
|-------|-----|-------------|
| `fiber/{hostname}/network/status` | 0 | Overall network status summary |
| `fiber/{hostname}/network/wifi/connected` | 0 | WiFi connection state (`true`/`false`) |
| `fiber/{hostname}/network/wifi/signal` | 0 | WiFi signal strength in dBm |
| `fiber/{hostname}/network/ethernet/connected` | 0 | Ethernet connection state (`true`/`false`) |

### Device Info Topics

| Topic | QoS | Description |
|-------|-----|-------------|
| `fiber/{hostname}/info/version` | 0 | Firmware version string |
| `fiber/{hostname}/info/uptime` | 0 | Uptime in seconds |
| `fiber/{hostname}/info/hostname` | 0 | Device hostname |

### Alarm Events

**Topic:** `fiber/{hostname}/alarms/events`
**QoS:** 1

```json
{
  "timestamp": "2026-01-13T14:30:00Z",
  "line": 0,
  "name": "Sensor 1",
  "from_state": "NORMAL",
  "to_state": "WARNING",
  "temperature_celsius": 39.5,
  "event_type": "alarm_transition"
}
```

**Alarm states:** `NEVER_CONNECTED`, `DISCONNECTED`, `RECONNECTING`, `NORMAL`, `WARNING`, `ALARM`, `CRITICAL`

### Error Messages

**Topic:** `fiber/{hostname}/errors`
**QoS:** 1

```json
{
  "timestamp": "2026-01-13T14:30:00Z",
  "command": "set_threshold",
  "error": "validation_failed",
  "message": "critical_low must be less than warning_low"
}
```

### LoRaWAN Sensors

**Topic:** `fiber/{hostname}/lorawan/sensors`
**QoS:** 0

Published when LoRaWAN sensor data is received via the ChirpStack gateway.

### Configuration Management Topics

These topics support the EU MDR-compliant signed configuration protocol (production only — dev platform accepts commands directly):

| Topic | QoS | Description |
|-------|-----|-------------|
| `fiber/{hostname}/config/challenge` | 1 | Device sends challenge for 2-phase config commit |
| `fiber/{hostname}/config/response` | 1 | Device confirms/rejects config change |
| `fiber/{hostname}/config/state` | 1 | Current device configuration state dump |

### Pairing Topics (Production Only)

| Topic | QoS | Description |
|-------|-----|-------------|
| `fiber/{hostname}/pair/request` | 1 | Pairing request (from client) |
| `fiber/{hostname}/pair/response` | 1 | Pairing response (from device) |

### Response Topics

| Topic | QoS | Description |
|-------|-----|-------------|
| `fiber/{hostname}/responses/sensor_config` | 1 | Response to `get_sensor_config` command |
| `fiber/{hostname}/responses/interval_config` | 1 | Response to `get_interval` command |
| `fiber/{hostname}/responses/{command_type}` | 1 | Generic response for other commands |

---

## Commands (Publish to these)

All commands are published to `fiber/{hostname}/commands/{path}`. Since authentication is disabled in the dev binary, no signatures are needed.

### Get System Info

**Topic:** `fiber/{hostname}/commands/system/get_info`
```json
{ "command": "get_info" }
```
Response arrives on `fiber/{hostname}/system/info`.

### Get Sensor Configuration

**Topic:** `fiber/{hostname}/commands/sensor/get_config`
```json
{ "command": "get_sensor_config" }
```
Response arrives on `fiber/{hostname}/responses/sensor_config`.

### Get Intervals

**Topic:** `fiber/{hostname}/commands/system/get_interval`
```json
{ "command": "get_interval" }
```
Response arrives on `fiber/{hostname}/responses/interval_config`.

### Set Threshold

**Topic:** `fiber/{hostname}/commands/sensor/set_threshold`

The application supports a **6-level threshold system**. The `alarm_low` and `alarm_high` fields are optional (default to 0.0 and 100.0 respectively), so the simpler 4-level format also works.

**6-level format (full):**
```json
{
  "command": "set_threshold",
  "line": 0,
  "thresholds": {
    "critical_low": 18.0,
    "alarm_low": 20.0,
    "warning_low": 27.0,
    "warning_high": 38.5,
    "alarm_high": 40.0,
    "critical_high": 41.0
  }
}
```

**4-level format (simplified — alarm_low/alarm_high use defaults):**
```json
{
  "command": "set_threshold",
  "line": 0,
  "thresholds": {
    "critical_low": 18.0,
    "warning_low": 27.0,
    "warning_high": 38.5,
    "critical_high": 41.0
  }
}
```

### Set Interval

**Topic:** `fiber/{hostname}/commands/system/set_interval`
```json
{
  "command": "set_interval",
  "sample_interval_ms": 2000,
  "aggregation_interval_ms": 15000,
  "report_interval_ms": 60000
}
```
Constraints: `sample_interval_ms <= aggregation_interval_ms <= report_interval_ms`, max 24 hours.

### Flush Storage

**Topic:** `fiber/{hostname}/commands/system/flush_storage`
```json
{ "command": "flush_storage" }
```

### Set Display Screen

**Topic:** `fiber/{hostname}/commands/display/set_screen`
```json
{ "command": "set_screen", "screen": "sensors" }
```
Valid screens: `sensors`, `power`, `network`, `qr_code`

### Silence Buzzer

**Topic:** `fiber/{hostname}/commands/sensor/silence_buzzer`
```json
{ "command": "silence_buzzer" }
```

### Restart Application

**Topic:** `fiber/{hostname}/commands/system/restart`
```json
{ "command": "restart" }
```

### Configuration Request (Production — signed commands)

**Topic:** `fiber/{hostname}/commands/config/request`

In production, configuration changes use a 2-phase commit with signed commands. The dev platform binary accepts commands directly without signatures.

```json
{ "command": "config_request", "payload": { ... }, "signature": "..." }
```

### Configuration Confirm (Production — signed commands)

**Topic:** `fiber/{hostname}/commands/config/confirm`
```json
{ "command": "config_confirm", "challenge_id": "...", "signature": "..." }
```

---

## Quick Test Commands

```bash
HOSTNAME=$(hostname)
MQTT_OPTS="-h localhost -u fiber -P 123456789"

# Monitor all messages
mosquitto_sub $MQTT_OPTS -t "fiber/#" -v

# Monitor only sensor data
mosquitto_sub $MQTT_OPTS -t "fiber/+/sensors/aggregated" -v

# Monitor only alarms
mosquitto_sub $MQTT_OPTS -t "fiber/+/alarms/events" -v

# Monitor power topics
mosquitto_sub $MQTT_OPTS -t "fiber/+/power/#" -v

# Monitor network topics
mosquitto_sub $MQTT_OPTS -t "fiber/+/network/#" -v

# Request system info
mosquitto_pub $MQTT_OPTS -t "fiber/$HOSTNAME/commands/system/get_info" \
  -m '{"command":"get_info"}'

# Set threshold on sensor line 0 (4-level)
mosquitto_pub $MQTT_OPTS -t "fiber/$HOSTNAME/commands/sensor/set_threshold" \
  -m '{"command":"set_threshold","line":0,"thresholds":{"critical_low":18,"warning_low":27,"warning_high":38.5,"critical_high":41}}'

# Set threshold on sensor line 0 (6-level)
mosquitto_pub $MQTT_OPTS -t "fiber/$HOSTNAME/commands/sensor/set_threshold" \
  -m '{"command":"set_threshold","line":0,"thresholds":{"critical_low":18,"alarm_low":20,"warning_low":27,"warning_high":38.5,"alarm_high":40,"critical_high":41}}'

# Get current sensor config
mosquitto_pub $MQTT_OPTS -t "fiber/$HOSTNAME/commands/sensor/get_config" \
  -m '{"command":"get_sensor_config"}'

# Get current intervals
mosquitto_pub $MQTT_OPTS -t "fiber/$HOSTNAME/commands/system/get_interval" \
  -m '{"command":"get_interval"}'

# Set intervals
mosquitto_pub $MQTT_OPTS -t "fiber/$HOSTNAME/commands/system/set_interval" \
  -m '{"command":"set_interval","sample_interval_ms":2000,"aggregation_interval_ms":15000,"report_interval_ms":60000}'

# Silence buzzer
mosquitto_pub $MQTT_OPTS -t "fiber/$HOSTNAME/commands/sensor/silence_buzzer" \
  -m '{"command":"silence_buzzer"}'

# Restart application
mosquitto_pub $MQTT_OPTS -t "fiber/$HOSTNAME/commands/system/restart" \
  -m '{"command":"restart"}'
```

---

## Topic Summary Table

### Published by Device

| Topic | QoS | Retained | Interval | Description |
|-------|-----|----------|----------|-------------|
| `status` | 1 | yes | on connect | Online/offline (LWT) |
| `sensors/aggregated` | 0 | no | 60s | Aggregated sensor readings |
| `sensors/summary` | 0 | no | periodic | Lightweight sensor summary |
| `sensors/line{N}/alarm` | 0 | no | on change | Per-line alarm state |
| `system/info` | 1 | yes | 60s | Full system status |
| `info/version` | 0 | no | periodic | Firmware version |
| `info/uptime` | 0 | no | periodic | Device uptime |
| `info/hostname` | 0 | no | periodic | Device hostname |
| `power/battery/percentage` | 0 | no | periodic | Battery % |
| `power/battery/voltage` | 0 | no | periodic | Battery mV |
| `power/battery/status` | 0 | no | periodic | Battery status |
| `power/ac/connected` | 0 | no | periodic | AC power state |
| `power/events/dc_loss` | 1 | no | on event | DC power lost |
| `network/status` | 0 | no | periodic | Network summary |
| `network/wifi/connected` | 0 | no | periodic | WiFi state |
| `network/wifi/signal` | 0 | no | periodic | WiFi signal dBm |
| `network/ethernet/connected` | 0 | no | periodic | Ethernet state |
| `alarms/events` | 1 | no | on event | Alarm transitions |
| `errors` | 1 | no | on event | Command errors |
| `lorawan/sensors` | 0 | no | on event | LoRaWAN sensor data |
| `config/challenge` | 1 | no | on request | Config 2-phase challenge |
| `config/response` | 1 | no | on request | Config change result |
| `config/state` | 1 | no | on request | Config state dump |
| `pair/response` | 1 | no | on request | Pairing response |
| `responses/{type}` | 1 | no | on request | Command responses |

### Commands (Subscribed by Device)

| Topic | Command | Description |
|-------|---------|-------------|
| `commands/system/get_info` | `get_info` | Request system info |
| `commands/sensor/get_config` | `get_sensor_config` | Request sensor config |
| `commands/system/get_interval` | `get_interval` | Request interval config |
| `commands/sensor/set_threshold` | `set_threshold` | Set alarm thresholds (4 or 6 level) |
| `commands/system/set_interval` | `set_interval` | Set sample/aggregation/report intervals |
| `commands/system/flush_storage` | `flush_storage` | Flush pending DB writes |
| `commands/display/set_screen` | `set_screen` | Switch LCD display screen |
| `commands/sensor/silence_buzzer` | `silence_buzzer` | Silence active buzzer |
| `commands/system/restart` | `restart` | Restart the application |
| `commands/config/request` | `config_request` | Signed config change (production) |
| `commands/config/confirm` | `config_confirm` | Confirm config change (production) |
