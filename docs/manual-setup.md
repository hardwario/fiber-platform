# FIBER Dev Platform — Manual Setup Guide

Step-by-step commands to set up the FIBER system on a Raspberry Pi 4 CM after a fresh **Raspberry Pi OS (64-bit, Bookworm)** installation.

> This guide replaces the `install.sh` script. Every command is listed so you understand what each step does.

---

## 1. Update the system

```bash
sudo apt update && sudo apt upgrade -y
```

## 2. Install dependencies

```bash
sudo apt install -y i2c-tools mosquitto mosquitto-clients sqlite3 jq curl git
```

| Package | Why |
|---------|-----|
| `i2c-tools` | Talk to I2C devices (accelerometer, RTC) |
| `mosquitto` | MQTT broker — routes messages between FIBER app and dashboard |
| `mosquitto-clients` | `mosquitto_pub` / `mosquitto_sub` CLI tools for testing |
| `sqlite3` | Inspect the FIBER database from the command line |
| `jq` | Pretty-print JSON (useful for reading MQTT payloads) |
| `curl`, `git` | Download files and clone repositories |

## 3. Enable hardware interfaces

### 3.1 Enable I2C and SPI

```bash
sudo raspi-config nonint do_i2c 0
sudo raspi-config nonint do_spi 0
```

### 3.2 Add device tree overlays

Open the boot config:

```bash
sudo nano /boot/firmware/config.txt
```

Add these lines at the end of the file (inside the `[all]` section, or add `[all]` first):

```
[all]
dtoverlay=w1-gpio
dtoverlay=uart3
dtoverlay=uart4
dtoverlay=pwm,pin=13,func=4
dtparam=spi=on
dtoverlay=spi6-1cs
dtoverlay=i2c-rtc,pcf85063a,i2c_csi_dsi
```

| Overlay | What it enables |
|---------|-----------------|
| `w1-gpio` | 1-Wire bus on GPIO4 — DS18B20 temperature sensors |
| `uart3` | Serial port for STM32 communication |
| `uart4` | Serial port for STM32 communication |
| `pwm,pin=13,func=4` | PWM output for the buzzer |
| `spi6-1cs` | SPI6 bus for the ST7920 LCD display (SPI6 avoids pin conflict with UART4) |
| `i2c-rtc,pcf85063a,i2c_csi_dsi` | Real-time clock on I2C bus (via CSI/DSI I2C) |

Save with `Ctrl+O`, `Enter`, `Ctrl+X`.

## 4. Add user to hardware groups

```bash
sudo usermod -aG dialout $USER
sudo usermod -aG i2c $USER
sudo usermod -aG gpio $USER
sudo usermod -aG spi $USER
```

| Group | Access to |
|-------|-----------|
| `dialout` | Serial ports (`/dev/ttyAMA3`, `/dev/ttyAMA4`) |
| `i2c` | I2C bus (`/dev/i2c-10`) |
| `gpio` | GPIO pins (LEDs, buttons, buzzer) |
| `spi` | SPI bus (display) |

> These take effect after logout/login or reboot.

## 5. Create directory structure

Raspberry Pi OS does not have a `/data` directory. We create it here to match the production layout — this is where the FIBER app stores its database, config, and backups.

```bash
sudo mkdir -p /opt/fiber
sudo mkdir -p /data/fiber/config
sudo mkdir -p /data/fiber/backups
sudo mkdir -p /data/ble
sudo touch /data/fiber/config/DEV_MODE_ENABLED
echo "000000" | sudo tee /data/ble/pin.txt
```

| Path | Purpose |
|------|---------|
| `/opt/fiber/` | FIBER application binary |
| `/data/fiber/config/` | YAML configuration files |
| `/data/fiber/config/DEV_MODE_ENABLED` | Required marker — tells the app this is a dev platform (no crypto verification) |
| `/data/fiber/backups/` | Automatic database backups |
| `/data/ble/pin.txt` | BLE pairing PIN (dummy value for dev platform) |

## 6. Configure Mosquitto MQTT broker

### 6.1 Create the config file

```bash
sudo nano /etc/mosquitto/conf.d/fiber.conf
```

Paste this content:

```
listener 1883
protocol mqtt

allow_anonymous false
password_file /etc/mosquitto/passwd

log_dest syslog
log_type error
log_type warning
log_type notice
log_type information
connection_messages true
```

> Persistence is already configured in the default `/etc/mosquitto/mosquitto.conf` — do not add it again here or Mosquitto will refuse to start.

Save and exit.

### 6.2 Create MQTT user

```bash
sudo touch /etc/mosquitto/passwd
sudo mosquitto_passwd -b /etc/mosquitto/passwd fiber 123456789
sudo chown mosquitto:mosquitto /etc/mosquitto/passwd
sudo chmod 0600 /etc/mosquitto/passwd
```

### 6.3 Restart and enable Mosquitto

```bash
sudo systemctl restart mosquitto
sudo systemctl enable mosquitto
```

### 6.4 Test it

Open two terminals (or two SSH sessions).

**Terminal 1** — subscribe:

```bash
mosquitto_sub -h localhost -u fiber -P 123456789 -t "test/hello" -v
```

**Terminal 2** — publish:

```bash
mosquitto_pub -h localhost -u fiber -P 123456789 -t "test/hello" -m "it works"
```

You should see `test/hello it works` in Terminal 1. Press `Ctrl+C` to stop.

## 7. Clone the dev-plat repository

```bash
cd ~
git clone <repo-url> fiber-dev-plat
```

> Replace `<repo-url>` with the repository URL provided by your instructor.

The repository contains:

| File | Purpose |
|------|---------|
| `fiber_app` | Pre-built FIBER application binary |
| `fiber.config.yaml` | Main application configuration |
| `fiber.sensors.config.yaml` | Sensor alarm thresholds |

## 8. Install binary and config files

```bash
sudo cp ~/fiber-dev-plat/fiber_app /opt/fiber/fiber_app
sudo chmod +x /opt/fiber/fiber_app
sudo cp ~/fiber-dev-plat/config/fiber.config.yaml /data/fiber/config/
sudo cp ~/fiber-dev-plat/config/fiber.sensors.config.yaml /data/fiber/config/
```

Verify:

```bash
ls -la /opt/fiber/fiber_app
ls /data/fiber/config/
```

## 9. Create the systemd service

```bash
sudo nano /etc/systemd/system/fiber.service
```

Paste:

```ini
[Unit]
Description=FIBER Application (Dev Platform)
After=network.target mosquitto.service
Wants=network.target mosquitto.service

[Service]
Type=simple
User=root
ExecStartPre=/bin/mkdir -p /data/fiber/config /data/fiber/backups
ExecStart=/opt/fiber/fiber_app
WorkingDirectory=/opt/fiber
Restart=on-failure
RestartSec=10
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

Save and exit. Then reload systemd:

```bash
sudo systemctl daemon-reload
sudo systemctl enable fiber.service
```

## 10. Reboot

A reboot is required for the device tree overlays and group changes to take effect.

```bash
sudo reboot
```

Reconnect via SSH after ~60 seconds.

## 11. Install 1-Wire sensor services

The FIBER hardware uses a DS2482 I2C-to-1-Wire bridge to communicate with DS18B20 temperature sensors. Two services are needed: one to initialize the bridge at boot, and a timer to periodically scan for new sensors.

Both scripts and service files are in the repo:

```bash
sudo cp ~/fiber-dev-plat/config/ds2482-init.sh /usr/bin/
sudo cp ~/fiber-dev-plat/config/w1-rescan.sh /usr/bin/
sudo chmod +x /usr/bin/ds2482-init.sh /usr/bin/w1-rescan.sh

sudo cp ~/fiber-dev-plat/config/ds2482-init.service /etc/systemd/system/
sudo cp ~/fiber-dev-plat/config/w1-rescan.service /etc/systemd/system/
sudo cp ~/fiber-dev-plat/config/w1-rescan.timer /etc/systemd/system/

sudo systemctl daemon-reload
sudo systemctl enable --now ds2482-init.service
sudo systemctl enable --now w1-rescan.timer
```

Verify sensors are detected:

```bash
ls /sys/bus/w1/devices/
```

You should see directories starting with `28-` (one per DS18B20 sensor).

## 12. Start the FIBER application

```bash
sudo systemctl start fiber.service
```

Check status:

```bash
sudo systemctl status fiber.service
```

You should see `active (running)`. If there are errors:

```bash
journalctl -u fiber.service -f
```

## 13. Verify sensors

Check that 1-Wire sensors are detected:

```bash
ls /sys/bus/w1/devices/
```

You should see directories starting with `28-` (one per DS18B20). Read a sensor directly:

```bash
cat /sys/bus/w1/devices/28-*/w1_slave
```

The last line contains `t=36500` meaning 36.5 C.

## 14. Verify MQTT data flow

Monitor all messages from the FIBER app:

```bash
mosquitto_sub -h localhost -u fiber -P 123456789 -t "fiber/#" -v
```

You should see sensor data, system info, and status messages appearing. Press `Ctrl+C` to stop.

Request system info manually:

```bash
HOSTNAME=$(hostname)
mosquitto_pub -h localhost -u fiber -P 123456789 \
  -t "fiber/$HOSTNAME/commands/system/get_info" \
  -m '{"command":"get_info"}'
```

## 15. Install Node-RED

```bash
bash <(curl -sL https://raw.githubusercontent.com/node-red/linux-installers/master/deb/update-nodejs-and-nodered) \
  --confirm-install --confirm-pi --no-init
```

This installs Node.js LTS and Node-RED with systemd integration. It takes a few minutes.

## 16. Install Dashboard 2.0

```bash
cd ~/.node-red
npm install @flowfuse/node-red-dashboard
```

## 17. Import FIBER dashboard flows

Copy the flows file from the dev-plat repository:

```bash
cp ~/fiber-dev-plat/node-red/flows.json ~/.node-red/flows.json
```

> Or import via the Node-RED editor: Menu -> Import -> select the file.

## 18. Start Node-RED

```bash
sudo systemctl enable nodered.service
sudo systemctl start nodered.service
```

## 19. Open the dashboard

In your browser, go to:

```
http://<pi-ip>:1880/dashboard
```

You should see the FIBER dashboard with sensor gauges, system info, power status, and alarm log.

The Node-RED flow editor is at:

```
http://<pi-ip>:1880
```

---

## Verification checklist

Run these to confirm everything is working:

```bash
# All services running?
sudo systemctl status mosquitto --no-pager
sudo systemctl status fiber.service --no-pager
sudo systemctl status nodered --no-pager

# Sensors detected?
ls /sys/bus/w1/devices/

# I2C devices?
i2cdetect -y 10

# MQTT flowing?
mosquitto_sub -h localhost -u fiber -P 123456789 -t "fiber/#" -v

# Your user groups?
groups
```

---

## Useful commands

```bash
# Service management
sudo systemctl start|stop|restart fiber.service
sudo systemctl start|stop|restart mosquitto
sudo systemctl start|stop|restart nodered

# Live logs
journalctl -u fiber.service -f
journalctl -u mosquitto -f

# MQTT shortcuts
HOSTNAME=$(hostname)
MQTT="-h localhost -u fiber -P 123456789"

# Monitor all
mosquitto_sub $MQTT -t "fiber/#" -v

# Monitor sensors only
mosquitto_sub $MQTT -t "fiber/+/sensors/aggregated" -v

# Monitor alarms only
mosquitto_sub $MQTT -t "fiber/+/alarms/events" -v

# Request system info
mosquitto_pub $MQTT -t "fiber/$HOSTNAME/commands/system/get_info" -m '{"command":"get_info"}'

# Get sensor config
mosquitto_pub $MQTT -t "fiber/$HOSTNAME/commands/sensor/get_config" -m '{"command":"get_sensor_config"}'

# Set threshold on line 0
mosquitto_pub $MQTT -t "fiber/$HOSTNAME/commands/sensor/set_threshold" \
  -m '{"command":"set_threshold","line":0,"thresholds":{"critical_low":18,"warning_low":27,"warning_high":38.5,"critical_high":41}}'

# Restart application
mosquitto_pub $MQTT -t "fiber/$HOSTNAME/commands/system/restart" -m '{"command":"restart"}'
```

---

## Credentials

| Service | User | Password |
|---------|------|----------|
| MQTT | `fiber` | `123456789` |

## Ports

| Service | Port |
|---------|------|
| Mosquitto MQTT | 1883 |
| Node-RED Editor | 1880 |
| Node-RED Dashboard | 1880/dashboard |
